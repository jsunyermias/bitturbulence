//! Actor que centraliza todos los accesos a los [`BlockScheduler`]s.
//!
//! ## Motivación
//!
//! Con el modelo anterior (`Vec<Mutex<BlockScheduler>>`), cada drainer adquiría
//! hasta N×11 locks por iteración de su bucle (N = número de archivos). Con
//! varios peers simultáneos, los schedulers se convertían en el punto de
//! contención central del pipeline de descarga.
//!
//! ## Diseño
//!
//! Un único task de Tokio posee todos los `BlockScheduler`s sin ningún lock.
//! Drainers y contexto se comunican con él mediante [`Msg`] y canales oneshot.
//! Toda la lógica de `pick_block` (11 passes) es síncrona dentro del actor.
//!
//! ## Garantías
//!
//! - Serialización total: no hay carreras de datos entre drainers.
//! - Sin inanición: el canal tiene capacidad 128; el actor procesa cada mensaje
//!   en tiempo O(N) donde N es el número de archivos.

use tokio::sync::{mpsc, oneshot};

use bitturbulence_pieces::{BlockScheduler, BlockTask};
use bitturbulence_protocol::Priority;

use super::stream::PeerAvail;

// ── Constantes de selección inter-archivo ─────────────────────────────────────

/// Umbral de "baja disponibilidad": el archivo lo tiene como máximo este número
/// de peers. Los archivos raros se priorizan sobre los comunes de igual prioridad.
const LOW_AVAIL: u32 = 1;

/// Passes de prioridad alta × rareza aplicados antes de los archivos normales.
///
/// Cada entrada es `(prioridad_mínima_del_archivo, máx_disponibilidad_permitida)`.
/// `None` en el segundo campo deshabilita el filtro de rareza.
pub(super) const PRIORITY_PASSES: &[(Priority, Option<u32>)] = &[
    (Priority::Maximum, Some(LOW_AVAIL)),   // 1. Máxima + rareza
    (Priority::VeryHigh, Some(LOW_AVAIL)),  // 2. Muy alta + rareza
    (Priority::Maximum, None),              // 3. Máxima
    (Priority::Higher, Some(LOW_AVAIL)),    // 4. Más alta + rareza
    (Priority::VeryHigh, None),             // 5. Muy alta
    (Priority::High, Some(LOW_AVAIL)),      // 6. Alta + rareza
    (Priority::Higher, None),               // 7. Más alta
    (Priority::High, None),                 // 8. Alta
];

// ── Mensajes ──────────────────────────────────────────────────────────────────

enum Msg {
    // ── Drainer ──────────────────────────────────────────────────────────────
    /// Selecciona el siguiente bloque a descargar para el peer indicado.
    /// `endgame=true` habilita los passes 10-11 (bloques `InFlight`).
    PickBlock {
        peer_avail: Vec<PeerAvail>,
        endgame:    bool,
        reply:      oneshot::Sender<Option<BlockTask>>,
    },
    /// Número total de bloques `Pending` en todos los archivos.
    TotalPending { reply: oneshot::Sender<u32> },
    /// Marca el bloque como completo y almacena su hash SHA-256.
    /// Devuelve `true` si es el primer stream en completar todos los bloques
    /// de la pieza (señal para iniciar la verificación Merkle).
    BlockDone { fi: usize, pi: u32, bi: u32, hash: [u8; 32], reply: oneshot::Sender<bool> },
    /// Decrementa el contador in-flight de un bloque (fallo o stream muerto).
    BlockFailed { fi: usize, pi: u32, bi: u32 },
    /// Registra el bitfield de disponibilidad de un peer al conectar.
    AddPeerBitfield { fi: usize, bits: Vec<bool> },
    /// Elimina la contribución de un peer que se desconecta.
    RemovePeerBitfield { fi: usize, bits: Vec<bool> },
    /// El peer anuncia que ha descargado la pieza `pi`.
    AddPeerHave { fi: usize, pi: usize },

    // ── Context (verify_and_complete / is_complete) ───────────────────────────
    /// Devuelve los hashes SHA-256 de todos los bloques de la pieza `pi`.
    PieceBlockHashes { fi: usize, pi: u32, reply: oneshot::Sender<Option<Vec<[u8; 32]>>> },
    /// Marca la pieza como verificada y descargada completamente.
    MarkPieceVerified { fi: usize, pi: u32 },
    /// La verificación Merkle ha fallado: resetea todos los bloques a `Pending`.
    MarkPieceHashFailed { fi: usize, pi: u32 },
    /// Devuelve `true` si todos los archivos del BitFlow están completos.
    IsComplete { reply: oneshot::Sender<bool> },
}

// ── Handle ────────────────────────────────────────────────────────────────────

/// Handle clonable para comunicarse con el [`SchedulerActor`].
///
/// Reemplaza a `Vec<Mutex<BlockScheduler>>` en [`FlowCtx`]. Todos los métodos
/// son `async` y devuelven inmediatamente en cuanto el actor los procesa.
#[derive(Clone)]
pub(crate) struct SchedulerHandle {
    tx: mpsc::Sender<Msg>,
}

impl SchedulerHandle {
    /// Lanza el actor en un nuevo task de Tokio y devuelve el handle.
    pub(crate) fn spawn(schedulers: Vec<BlockScheduler>) -> Self {
        let (tx, rx) = mpsc::channel(128);
        tokio::spawn(run_actor(schedulers, rx));
        Self { tx }
    }

    // ── Drainer ──────────────────────────────────────────────────────────────

    /// Selecciona el bloque más prioritario disponible para el peer.
    ///
    /// `endgame`: cuando es `true`, incluye bloques `InFlight` (passes 10-11)
    /// para redundancia en modo endgame o inestabilidad.
    pub(crate) async fn pick_block(&self, peer_avail: &[PeerAvail], endgame: bool) -> Option<BlockTask> {
        let (tx, rx) = oneshot::channel();
        let _ = self.tx.send(Msg::PickBlock {
            peer_avail: peer_avail.to_vec(),
            endgame,
            reply: tx,
        }).await;
        rx.await.unwrap_or(None)
    }

    /// Suma de bloques `Pending` en todos los archivos.
    pub(crate) async fn total_pending(&self) -> u32 {
        let (tx, rx) = oneshot::channel();
        let _ = self.tx.send(Msg::TotalPending { reply: tx }).await;
        rx.await.unwrap_or(0)
    }

    /// Marca el bloque como completado con su hash SHA-256.
    /// Devuelve `true` si todos los bloques de la pieza están `Done`.
    pub(crate) async fn block_done(&self, fi: usize, pi: u32, bi: u32, hash: [u8; 32]) -> bool {
        let (tx, rx) = oneshot::channel();
        let _ = self.tx.send(Msg::BlockDone { fi, pi, bi, hash, reply: tx }).await;
        rx.await.unwrap_or(false)
    }

    /// Decrementa el contador in-flight del bloque (fallo o stream muerto).
    pub(crate) async fn block_failed(&self, fi: usize, pi: u32, bi: u32) {
        let _ = self.tx.send(Msg::BlockFailed { fi, pi, bi }).await;
    }

    /// Registra el bitfield de disponibilidad de un peer al conectar.
    pub(crate) async fn add_peer_bitfield(&self, fi: usize, bits: Vec<bool>) {
        let _ = self.tx.send(Msg::AddPeerBitfield { fi, bits }).await;
    }

    /// Elimina la contribución de un peer que se desconecta.
    pub(crate) async fn remove_peer_bitfield(&self, fi: usize, bits: Vec<bool>) {
        let _ = self.tx.send(Msg::RemovePeerBitfield { fi, bits }).await;
    }

    /// Registra que el peer ha descargado la pieza `pi` del archivo `fi`.
    pub(crate) async fn add_peer_have(&self, fi: usize, pi: usize) {
        let _ = self.tx.send(Msg::AddPeerHave { fi, pi }).await;
    }

    // ── Context ───────────────────────────────────────────────────────────────

    /// Devuelve los hashes SHA-256 de todos los bloques de la pieza `pi`.
    pub(crate) async fn piece_block_hashes(&self, fi: usize, pi: u32) -> Option<Vec<[u8; 32]>> {
        let (tx, rx) = oneshot::channel();
        let _ = self.tx.send(Msg::PieceBlockHashes { fi, pi, reply: tx }).await;
        rx.await.unwrap_or(None)
    }

    /// Marca la pieza como verificada y descargada completamente.
    pub(crate) async fn mark_piece_verified(&self, fi: usize, pi: u32) {
        let _ = self.tx.send(Msg::MarkPieceVerified { fi, pi }).await;
    }

    /// La verificación Merkle ha fallado: resetea todos los bloques a `Pending`.
    pub(crate) async fn mark_piece_hash_failed(&self, fi: usize, pi: u32) {
        let _ = self.tx.send(Msg::MarkPieceHashFailed { fi, pi }).await;
    }

    /// Devuelve `true` si todos los archivos del BitFlow están completos.
    pub(crate) async fn is_complete(&self) -> bool {
        let (tx, rx) = oneshot::channel();
        let _ = self.tx.send(Msg::IsComplete { reply: tx }).await;
        rx.await.unwrap_or(false)
    }
}

// ── Actor ─────────────────────────────────────────────────────────────────────

async fn run_actor(mut schedulers: Vec<BlockScheduler>, mut rx: mpsc::Receiver<Msg>) {
    while let Some(msg) = rx.recv().await {
        match msg {
            Msg::PickBlock { peer_avail, endgame, reply } => {
                let task = pick_block_sync(&mut schedulers, &peer_avail, endgame);
                let _ = reply.send(task);
            }
            Msg::TotalPending { reply } => {
                let n: u32 = schedulers.iter().map(|s| s.pending_blocks()).sum();
                let _ = reply.send(n);
            }
            Msg::BlockDone { fi, pi, bi, hash, reply } => {
                let ready = schedulers.get_mut(fi)
                    .map(|s| s.mark_block_done(pi, bi, hash))
                    .unwrap_or(false);
                let _ = reply.send(ready);
            }
            Msg::BlockFailed { fi, pi, bi } => {
                if let Some(s) = schedulers.get_mut(fi) { s.mark_block_failed(pi, bi); }
            }
            Msg::AddPeerBitfield { fi, bits } => {
                if let Some(s) = schedulers.get_mut(fi) { s.add_peer_bitfield(&bits); }
            }
            Msg::RemovePeerBitfield { fi, bits } => {
                if let Some(s) = schedulers.get_mut(fi) { s.remove_peer_bitfield(&bits); }
            }
            Msg::AddPeerHave { fi, pi } => {
                if let Some(s) = schedulers.get_mut(fi) { s.add_peer_have(pi); }
            }
            Msg::PieceBlockHashes { fi, pi, reply } => {
                let hashes = schedulers.get(fi).and_then(|s| s.piece_block_hashes(pi));
                let _ = reply.send(hashes);
            }
            Msg::MarkPieceVerified { fi, pi } => {
                if let Some(s) = schedulers.get_mut(fi) { s.mark_piece_verified(pi); }
            }
            Msg::MarkPieceHashFailed { fi, pi } => {
                if let Some(s) = schedulers.get_mut(fi) { s.mark_piece_hash_failed(pi); }
            }
            Msg::IsComplete { reply } => {
                let ok = schedulers.iter().all(|s| s.is_complete());
                let _ = reply.send(ok);
            }
        }
    }
}

// ── Lógica de selección de bloque (síncrona dentro del actor) ─────────────────

/// Selecciona el bloque más prioritario disponible para el peer dado.
///
/// Aplica los 11 passes del protocolo de selección inter-archivo de forma
/// completamente síncrona, sin locks, ya que el actor posee los schedulers.
///
/// - Passes 1–8 : bloques `Pending` de archivos con prioridad alta × rareza.
/// - Pass 9     : bloques `Pending` de archivos con prioridad Normal o inferior.
/// - Passes 10–11: bloques `InFlight` (redundancia); solo si `endgame=true`.
fn pick_block_sync(
    schedulers: &mut [BlockScheduler],
    peer_avail: &[PeerAvail],
    endgame: bool,
) -> Option<BlockTask> {
    // Passes 1–8: Pending × prioridad alta × rareza.
    for &(prio, max_avail) in PRIORITY_PASSES {
        for (fi, avail) in peer_avail.iter().enumerate() {
            let Some(sched) = schedulers.get_mut(fi) else { continue };
            if sched.priority() != prio { continue; }
            if max_avail.is_some_and(|m| sched.min_availability() > m) { continue; }
            let bits = avail.as_bitfield(sched.num_pieces());
            if bits.iter().all(|&h| !h) { continue; }
            if let Some(task) = sched.schedule_pending(&bits) { return Some(task); }
        }
    }

    // Pass 9: Pending de archivos con prioridad Normal o inferior.
    for (fi, avail) in peer_avail.iter().enumerate() {
        let Some(sched) = schedulers.get_mut(fi) else { continue };
        if sched.priority() >= Priority::High { continue; }
        let bits = avail.as_bitfield(sched.num_pieces());
        if bits.iter().all(|&h| !h) { continue; }
        if let Some(task) = sched.schedule_pending(&bits) { return Some(task); }
    }

    // Passes 10–11: InFlight (redundancia/endgame).
    if endgame {
        for (fi, avail) in peer_avail.iter().enumerate() {
            let Some(sched) = schedulers.get_mut(fi) else { continue };
            let bits = avail.as_bitfield(sched.num_pieces());
            if bits.iter().all(|&h| !h) { continue; }
            if let Some(task) = sched.schedule(&bits) { return Some(task); }
        }
    }

    None
}
