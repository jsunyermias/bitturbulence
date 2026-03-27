use std::path::Path;
use std::sync::{Arc, atomic::{AtomicU64, AtomicUsize, Ordering}};

use anyhow::{Context, Result};
use tokio::sync::{Mutex, watch};
use tracing::{info, warn};

use bitturbulence_pieces::{BlockScheduler, TorrentStore, piece_root_from_block_hashes};
use bitturbulence_protocol::Metainfo;

use super::scheduler_actor::SchedulerHandle;

/// Estado compartido de un BitFlow activo.
pub struct FlowCtx {
    pub meta:  Metainfo,
    pub store: TorrentStore,
    /// Actor que gestiona los BlockSchedulers de todos los archivos sin locks.
    pub sched: SchedulerHandle,
    /// have[fi][pi] = pieza verificada y completa (para anunciar a peers).
    pub have:  Mutex<Vec<Vec<bool>>>,
    /// Bytes descargados y verificados.
    pub downloaded: AtomicU64,
    /// Peers conectados actualmente.
    pub peer_count: AtomicUsize,
    /// Señal de descarga completa. Se emite `true` una sola vez cuando todos
    /// los archivos están verificados. Drainers y state_loop la suscriben.
    pub complete_tx: watch::Sender<bool>,
}

impl FlowCtx {
    pub async fn new(meta: Metainfo, save_path: &Path, seeding: bool) -> Result<Arc<Self>> {
        let store = TorrentStore::open(save_path, &meta).await
            .context("opening torrent store")?;

        let mut raw_schedulers = Vec::with_capacity(meta.files.len());
        let mut have_init      = Vec::with_capacity(meta.files.len());

        for fi in 0..meta.files.len() {
            let file    = store.file(fi);
            let num     = file.num_pieces() as usize;
            let pl      = meta.files[fi].piece_length();
            let last_pl = meta.files[fi].last_piece_length();

            let mut sched = BlockScheduler::new(fi, num, pl, last_pl);

            let file_have = if seeding {
                for pi in 0..num { sched.mark_piece_verified(pi as u32); }
                vec![true; num]
            } else {
                vec![false; num]
            };

            raw_schedulers.push(sched);
            have_init.push(file_have);
        }

        let (complete_tx, _) = watch::channel(false);

        Ok(Arc::new(Self {
            sched: SchedulerHandle::spawn(raw_schedulers),
            meta,
            store,
            have: Mutex::new(have_init),
            downloaded: AtomicU64::new(0),
            peer_count: AtomicUsize::new(0),
            complete_tx,
        }))
    }

    /// Bitfield de piezas que tenemos completas para el archivo `fi`.
    pub async fn our_bitfield(&self, fi: usize) -> Vec<bool> {
        self.have.lock().await[fi].clone()
    }

    /// Verifica la raíz Merkle de la pieza usando los hashes de bloque
    /// almacenados en el actor (sin releer el disco) y la marca completa.
    ///
    /// Devuelve `true` si la verificación fue correcta.
    /// En caso de fallo resetea todos los bloques de la pieza a `Pending`.
    pub async fn verify_and_complete(&self, fi: usize, pi: u32) -> bool {
        let block_hashes = match self.sched.piece_block_hashes(fi, pi).await {
            Some(h) => h,
            None => {
                warn!(fi, pi, "piece_block_hashes unavailable");
                self.sched.mark_piece_hash_failed(fi, pi).await;
                return false;
            }
        };
        let expected    = &self.meta.files[fi].piece_hashes[pi as usize];
        let computed    = piece_root_from_block_hashes(&block_hashes);
        let piece_bytes = self.meta.files[fi].piece_len(pi) as u64;
        if &computed == expected {
            self.have.lock().await[fi][pi as usize] = true;
            self.sched.mark_piece_verified(fi, pi).await;
            self.downloaded.fetch_add(piece_bytes, Ordering::Relaxed);
            info!(fi, pi, bytes = piece_bytes, "piece merkle root ok");
            self.signal_if_complete().await;
            true
        } else {
            warn!(fi, pi, "merkle root mismatch — resetting piece to Pending");
            self.sched.mark_piece_hash_failed(fi, pi).await;
            false
        }
    }

    pub async fn is_complete(&self) -> bool {
        self.sched.is_complete().await
    }

    /// Comprueba si la descarga está completa y, si es así, emite la señal
    /// `complete_tx` una sola vez y registra un log de compleción.
    ///
    /// Es idempotente: si ya se emitió la señal no hace nada.
    pub async fn signal_if_complete(&self) {
        if *self.complete_tx.borrow() {
            return; // ya señalizado
        }
        if self.sched.is_complete().await {
            let id: String = self.meta.info_hash[..4]
                .iter().map(|b| format!("{b:02x}")).collect();
            info!("[{id}] {} — descarga completa, pasando a seeder", self.meta.name);
            // send_replace actualiza el valor y notifica a receivers activos
            // aunque no haya ninguno suscrito todavía (a diferencia de send()).
            self.complete_tx.send_replace(true);
        }
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use bitturbulence_protocol::{FileEntry, Priority};

    /// Metainfo mínimo: 1 archivo de 1 pieza para tests.
    fn minimal_meta() -> Metainfo {
        Metainfo {
            name: "test.bin".into(),
            info_hash: [0u8; 32],
            files: vec![FileEntry {
                path:         vec!["test.bin".into()],
                size:         bitturbulence_protocol::BLOCK_SIZE as u64,
                piece_hashes: vec![[0u8; 32]],
                priority:     Priority::Normal,
            }],
            trackers: vec![],
            comment:  None,
        }
    }

    #[tokio::test]
    async fn signal_if_complete_fires_when_seeding() {
        let dir = tempfile::tempdir().unwrap();
        // seeding=true: todas las piezas pre-verificadas en el constructor.
        let ctx = FlowCtx::new(minimal_meta(), dir.path(), true).await.unwrap();

        assert!(!*ctx.complete_tx.borrow(), "no debe emitir antes de llamar");
        ctx.signal_if_complete().await;
        assert!(*ctx.complete_tx.borrow(), "debe emitir porque el flow está completo");
    }

    #[tokio::test]
    async fn signal_if_complete_no_fire_when_incomplete() {
        let dir = tempfile::tempdir().unwrap();
        let ctx = FlowCtx::new(minimal_meta(), dir.path(), false).await.unwrap();

        ctx.signal_if_complete().await;
        assert!(!*ctx.complete_tx.borrow(), "no debe emitir si quedan piezas pendientes");
    }

    #[tokio::test]
    async fn signal_if_complete_is_idempotent() {
        let dir = tempfile::tempdir().unwrap();
        let ctx = FlowCtx::new(minimal_meta(), dir.path(), true).await.unwrap();

        ctx.signal_if_complete().await;
        ctx.signal_if_complete().await; // segunda llamada: no-op
        assert!(*ctx.complete_tx.borrow());
    }

    #[tokio::test]
    async fn complete_rx_fires_after_signal() {
        let dir = tempfile::tempdir().unwrap();
        let ctx = FlowCtx::new(minimal_meta(), dir.path(), true).await.unwrap();

        // Subscribir un receiver ANTES de llamar a signal_if_complete.
        let mut rx = ctx.complete_tx.subscribe();
        assert!(!*rx.borrow());

        ctx.signal_if_complete().await;

        // El receiver debe ver el cambio.
        assert!(*rx.borrow_and_update());
    }
}
