//! Daemon de descarga con multiplexación QUIC a nivel de bloque.
//!
//! ## Streams QUIC por conexión
//!
//! | Stream | Dirección | Contenido                                     |
//! |--------|-----------|-----------------------------------------------|
//! | 1      | ambos     | Hello / HelloAck (efímero)                    |
//! | 2      | control   | Have*, KeepAlive, Bye, HavePiece              |
//! | 3…N    | datos     | Request (1 bloque) / Piece / Reject por loop  |
//!
//! ## Multiplexación de bloques
//!
//! Cada stream de datos descarga UN bloque (16 KiB) a la vez.
//! El coordinador asigna bloques usando [`BlockScheduler`] con esta prioridad:
//!
//! 1. Bloque nuevo (`Pending`) — preferencia absoluta (más bloques = más streams).
//! 2. Bloque con `streams < TYPICAL` — añadir redundancia hasta el objetivo (2).
//! 3. Bloque con `streams < MAX`  — endgame (hasta 4 streams por bloque).
//!
//! Nuevos streams se abren solo cuando hay bloques `Pending` sin cubrir, nunca
//! solo para añadir redundancia.

use std::{
    collections::{HashMap, HashSet},
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicU64, AtomicUsize, Ordering},
    },
    time::Duration,
};

use anyhow::{anyhow, Context, Result};
use bytes::Bytes;
use futures_util::{SinkExt, StreamExt};
use quinn::{RecvStream, SendStream};
use rand::RngCore;
use tokio::sync::{Mutex, mpsc};
use tokio::time::{interval, timeout};
use tokio_util::codec::{FramedRead, FramedWrite};
use tracing::{debug, info, warn};

use qt_pieces::{BlockScheduler, BlockTask, TorrentStore, verify_piece};
use qt_protocol::{AuthPayload, Message, MessageCodec, Metainfo, PROTOCOL_VERSION};
use qt_tracker::{AnnounceEvent, AnnounceRequest, TrackerClient};
use qt_transport::{PeerConnection, QuicEndpoint};

use crate::config::Config;
use crate::state::{ClientState, DownloadState};

// ── Tipo aliases ──────────────────────────────────────────────────────────────

type W = FramedWrite<SendStream, MessageCodec>;
type R = FramedRead<RecvStream, MessageCodec>;

// ── Constantes ────────────────────────────────────────────────────────────────

const HELLO_TIMEOUT:       Duration = Duration::from_secs(10);
const KEEPALIVE_INTERVAL:  Duration = Duration::from_secs(60);
const ANNOUNCE_INTERVAL:   Duration = Duration::from_secs(300);
const STATE_SAVE_INTERVAL: Duration = Duration::from_secs(30);

/// Streams de datos abiertos al conectar a un peer.
const DEFAULT_STREAMS: usize = 2;

/// Máximo de streams de datos simultáneos por peer.
/// Más streams solo se abren cuando hay bloques Pending sin cubrir.
const MAX_STREAMS: usize = 4;

// ── Estado compartido de un torrent ───────────────────────────────────────────

pub struct TorrentCtx {
    pub meta:       Metainfo,
    pub store:      TorrentStore,
    /// Un BlockScheduler por archivo.
    schedulers:     Vec<Mutex<BlockScheduler>>,
    /// have[fi][pi] = pieza verificada y completa (para anunciar a peers).
    have:           Mutex<Vec<Vec<bool>>>,
    /// Bytes descargados y verificados.
    pub downloaded: AtomicU64,
    /// Peers conectados actualmente.
    pub peer_count: AtomicUsize,
}

impl TorrentCtx {
    pub async fn new(meta: Metainfo, save_path: &Path, seeding: bool) -> Result<Arc<Self>> {
        let store = TorrentStore::open(save_path, &meta).await
            .context("opening torrent store")?;

        let mut schedulers = Vec::with_capacity(meta.files.len());
        let mut have_init  = Vec::with_capacity(meta.files.len());

        for fi in 0..meta.files.len() {
            let file      = store.file(fi);
            let num       = file.num_pieces() as usize;
            let pl        = meta.files[fi].piece_length();
            let last_pl   = meta.files[fi].last_piece_length();

            let mut sched = BlockScheduler::new(fi, num, pl, last_pl);

            let file_have = if seeding {
                for pi in 0..num { sched.mark_piece_verified(pi as u32); }
                vec![true; num]
            } else {
                vec![false; num]
            };

            schedulers.push(Mutex::new(sched));
            have_init.push(file_have);
        }

        Ok(Arc::new(Self {
            meta,
            store,
            schedulers,
            have: Mutex::new(have_init),
            downloaded: AtomicU64::new(0),
            peer_count:  AtomicUsize::new(0),
        }))
    }

    /// Bitfield de piezas que tenemos completas para el archivo `fi`.
    pub async fn our_bitfield(&self, fi: usize) -> Vec<bool> {
        self.have.lock().await[fi].clone()
    }

    /// Lee la pieza completa del disco, verifica SHA-256 y la marca completa.
    ///
    /// Devuelve `true` si la verificación fue correcta.
    /// En caso de fallo resetea todos los bloques de la pieza a `Pending`.
    pub async fn verify_and_complete(&self, fi: usize, pi: u32) -> bool {
        let data = match self.store.read_piece(fi, pi).await {
            Ok(d)  => d,
            Err(e) => {
                warn!(fi, pi, "read_piece: {e}");
                self.schedulers[fi].lock().await.mark_piece_hash_failed(pi);
                return false;
            }
        };
        let expected = &self.meta.files[fi].piece_hashes[pi as usize];
        if verify_piece(&data, expected) {
            self.have.lock().await[fi][pi as usize] = true;
            self.schedulers[fi].lock().await.mark_piece_verified(pi);
            self.downloaded.fetch_add(data.len() as u64, Ordering::Relaxed);
            info!(fi, pi, bytes = data.len(), "piece verified ok");
            true
        } else {
            warn!(fi, pi, "hash mismatch — resetting piece to Pending");
            self.schedulers[fi].lock().await.mark_piece_hash_failed(pi);
            false
        }
    }

    pub async fn is_complete(&self) -> bool {
        for sched in &self.schedulers {
            if !sched.lock().await.is_complete() { return false; }
        }
        true
    }
}

// ── Disponibilidad del peer ───────────────────────────────────────────────────

#[derive(Clone)]
enum PeerAvail {
    Unknown,
    HaveAll,
    HaveNone,
    Bitmap(Vec<bool>),
}

impl PeerAvail {
    fn as_bitfield(&self, num: usize) -> Vec<bool> {
        match self {
            Self::Unknown | Self::HaveNone => vec![false; num],
            Self::HaveAll                  => vec![true;  num],
            Self::Bitmap(v) => {
                let mut out = v.clone();
                out.resize(num, false);
                out
            }
        }
    }
}

// ── Helpers de bitmap ─────────────────────────────────────────────────────────

fn bitfield_to_bytes(bits: &[bool]) -> Vec<u8> {
    let mut bytes = vec![0u8; bits.len().div_ceil(8)];
    for (i, &has) in bits.iter().enumerate() {
        if has { bytes[i / 8] |= 0x80 >> (i % 8); }
    }
    bytes
}

fn bytes_to_bitfield(bytes: &[u8], num: usize) -> Vec<bool> {
    (0..num)
        .map(|i| bytes.get(i / 8).is_some_and(|b| (b >> (7 - (i % 8))) & 1 == 1))
        .collect()
}

// ── Tipos del sistema multi-stream por bloque ─────────────────────────────────

/// Resultado reportado por un stream worker al coordinador.
enum StreamResult {
    /// Bloque descargado y escrito en disco.
    BlockOk   { stream_id: usize, fi: usize, pi: u32, bi: u32 },
    /// Bloque fallido (Reject del seeder o error de red).
    BlockFail { stream_id: usize, fi: usize, pi: u32, bi: u32 },
    /// Stream muerto por error QUIC irrecuperable.
    StreamDead { stream_id: usize },
}

/// Slot que representa un stream de datos activo en el coordinador.
struct StreamSlot {
    task_tx:      mpsc::Sender<BlockTask>,
    /// Bloque actualmente asignado (fi, pi, bi), o None si idle.
    active_block: Option<(usize, u32, u32)>,
}

// ── Stream worker: descarga un bloque a la vez ────────────────────────────────

/// Descarga repetidamente bloques asignados por el coordinador sobre un
/// stream QUIC dedicado.
///
/// Por cada [`BlockTask`]:
/// 1. Envía un `Request` para ese único bloque.
/// 2. Espera la respuesta `Piece` (o `Reject`).
/// 3. Escribe los datos en disco.
/// 4. Notifica `BlockOk` / `BlockFail` al coordinador.
async fn download_stream_worker(
    stream_id: usize,
    mut writer: W,
    mut reader: R,
    mut task_rx: mpsc::Receiver<BlockTask>,
    result_tx:  mpsc::Sender<StreamResult>,
    ctx:        Arc<TorrentCtx>,
) {
    while let Some(BlockTask { fi, pi, bi, begin, length }) = task_rx.recv().await {
        // ── Enviar Request para el bloque ────────────────────────────────
        if writer.send(Message::Request {
            file_index: fi as u16, piece_index: pi, begin, length,
        }).await.is_err() {
            let _ = result_tx.send(StreamResult::StreamDead { stream_id }).await;
            return;
        }

        // ── Esperar respuesta ────────────────────────────────────────────
        let result = loop {
            match reader.next().await {
                // Piece para este bloque exacto
                Some(Ok(Message::Piece { file_index, piece_index, begin: b, data }))
                    if file_index as usize == fi && piece_index == pi && b == begin =>
                {
                    match ctx.store.write_block(fi, pi, begin, &data).await {
                        Ok(()) => break StreamResult::BlockOk { stream_id, fi, pi, bi },
                        Err(e) => {
                            warn!(fi, pi, bi, "write_block: {e}");
                            break StreamResult::BlockFail { stream_id, fi, pi, bi };
                        }
                    }
                }
                // Reject para este bloque
                Some(Ok(Message::Reject { file_index, piece_index, begin: b, .. }))
                    if file_index as usize == fi && piece_index == pi && b == begin =>
                {
                    debug!(fi, pi, bi, "block rejected by seeder");
                    break StreamResult::BlockFail { stream_id, fi, pi, bi };
                }
                // Stream cerrado o error
                None | Some(Err(_)) => {
                    let _ = result_tx.send(StreamResult::StreamDead { stream_id }).await;
                    return;
                }
                // Mensaje de otro bloque o tipo inesperado: ignorar y seguir
                Some(Ok(_)) => {}
            }
        };
        let _ = result_tx.send(result).await;
    }
}

// ── Seeder: sirve blocks sobre un data stream ─────────────────────────────────

/// Procesa `Request`s del downloader sobre un stream de datos dedicado
/// y responde con `Piece` o `Reject` bloque a bloque.
async fn serve_data_stream(
    mut writer: W,
    mut reader: R,
    ctx: Arc<TorrentCtx>,
) {
    while let Some(msg) = reader.next().await {
        match msg {
            Ok(Message::Request { file_index, piece_index, begin, length }) => {
                let fi   = file_index as usize;
                let have = ctx.our_bitfield(fi).await;
                if have.get(piece_index as usize).copied().unwrap_or(false) {
                    match ctx.store.read_block(fi, piece_index, begin, length).await {
                        Ok(data) => {
                            let _ = writer.send(Message::Piece {
                                file_index, piece_index, begin,
                                data: Bytes::from(data),
                            }).await;
                        }
                        Err(e) => {
                            warn!(fi, piece_index, begin, "read_block: {e}");
                            let _ = writer.send(Message::Reject {
                                file_index, piece_index, begin, length,
                            }).await;
                        }
                    }
                } else {
                    let _ = writer.send(Message::Reject {
                        file_index, piece_index, begin, length,
                    }).await;
                }
            }
            Ok(_)  => {} // solo se esperan Request en data streams
            Err(e) => { debug!("data stream error: {e}"); break; }
        }
    }
}

// ── Helpers del coordinador ───────────────────────────────────────────────────

/// Envía nuestra disponibilidad de todos los archivos al peer.
async fn send_our_bitfields(ctx: &TorrentCtx, ctrl_w: &mut W) -> Result<()> {
    for fi in 0..ctx.meta.files.len() {
        let bits = ctx.our_bitfield(fi).await;
        let msg = if bits.iter().all(|&h| h) {
            Message::HaveAll  { file_index: fi as u16 }
        } else if bits.iter().all(|&h| !h) {
            Message::HaveNone { file_index: fi as u16 }
        } else {
            Message::HaveBitmap {
                file_index: fi as u16,
                bitmap:     Bytes::from(bitfield_to_bytes(&bits)),
            }
        };
        ctrl_w.send(msg).await?;
    }
    Ok(())
}

/// Selecciona el siguiente bloque a descargar para un stream existente.
///
/// Incluye bloques Pending, InFlight<TYPICAL e InFlight<MAX (ver [`BlockScheduler::schedule`]).
async fn pick_block(ctx: &TorrentCtx, peer_avail: &[PeerAvail]) -> Option<BlockTask> {
    for (fi, avail) in peer_avail.iter().enumerate() {
        let num  = ctx.schedulers[fi].lock().await.num_pieces();
        let bits = avail.as_bitfield(num);
        if bits.iter().all(|&h| !h) { continue; }
        if let Some(task) = ctx.schedulers[fi].lock().await.schedule(&bits) {
            return Some(task);
        }
    }
    None
}

/// Selecciona un bloque `Pending` (solo) para decidir si abrir un nuevo stream.
///
/// Solo devuelve algo si hay trabajo genuinamente nuevo; no abre streams solo
/// para añadir redundancia.
async fn pick_pending_block(ctx: &TorrentCtx, peer_avail: &[PeerAvail]) -> Option<BlockTask> {
    for (fi, avail) in peer_avail.iter().enumerate() {
        let num  = ctx.schedulers[fi].lock().await.num_pieces();
        let bits = avail.as_bitfield(num);
        if bits.iter().all(|&h| !h) { continue; }
        if let Some(task) = ctx.schedulers[fi].lock().await.schedule_pending(&bits) {
            return Some(task);
        }
    }
    None
}

// ── Bucle del downloader (conexión saliente) ──────────────────────────────────

async fn run_peer_downloader(
    conn:    &PeerConnection,
    ctx:     &Arc<TorrentCtx>,
    peer_id: &[u8; 32],
) -> Result<()> {
    // ── Stream 1: Hello / HelloAck ──────────────────────────────────────
    let (mut hello_w, mut hello_r) = conn.open_bidi_stream().await?;

    hello_w.send(Message::Hello {
        version:   PROTOCOL_VERSION,
        peer_id:   *peer_id,
        info_hash: ctx.meta.info_hash,
        auth:      AuthPayload::None,
    }).await.context("sending hello")?;

    let ack = timeout(HELLO_TIMEOUT, hello_r.next()).await
        .map_err(|_| anyhow!("hello ack timeout"))?
        .ok_or_else(|| anyhow!("disconnected during hello"))??;

    match ack {
        Message::HelloAck { accepted: true, .. } => {}
        Message::HelloAck { accepted: false, reason, .. } =>
            return Err(anyhow!("hello rejected: {}", reason.unwrap_or_default())),
        _ => return Err(anyhow!("expected HelloAck")),
    }
    drop(hello_w);
    drop(hello_r);

    // ── Stream 2: control (Have*, KeepAlive, Bye) ───────────────────────
    let (mut ctrl_w, mut ctrl_r) = conn.open_bidi_stream().await?;
    send_our_bitfields(ctx, &mut ctrl_w).await?;

    // ── Streams 3…N: datos por bloque ───────────────────────────────────
    let num_files = ctx.meta.files.len();
    let (result_tx, mut result_rx) = mpsc::channel::<StreamResult>(MAX_STREAMS * 8);
    let mut slots: HashMap<usize, StreamSlot> = HashMap::new();
    let mut next_id: usize = 0;
    let mut peer_avail: Vec<PeerAvail> = vec![PeerAvail::Unknown; num_files];

    // Abrir DEFAULT_STREAMS streams de datos desde el inicio.
    for _ in 0..DEFAULT_STREAMS {
        let (w, r) = conn.open_bidi_stream().await?;
        let (task_tx, task_rx) = mpsc::channel::<BlockTask>(1);
        tokio::spawn(download_stream_worker(
            next_id, w, r, task_rx, result_tx.clone(), ctx.clone(),
        ));
        slots.insert(next_id, StreamSlot { task_tx, active_block: None });
        next_id += 1;
    }

    let mut ka_timer = interval(KEEPALIVE_INTERVAL);
    ka_timer.tick().await;

    // ── Bucle coordinador ──────────────────────────────────────────────
    loop {
        // ─ Asignar bloques a slots idle ────────────────────────────────
        for slot in slots.values_mut() {
            if slot.active_block.is_none() {
                if let Some(task) = pick_block(ctx, &peer_avail).await {
                    let key = (task.fi, task.pi, task.bi);
                    let _ = slot.task_tx.send(task).await;
                    slot.active_block = Some(key);
                }
            }
        }

        // ─ Scale-up: nuevo stream solo para bloques Pending ────────────
        // No se abren streams solo para añadir redundancia.
        if slots.len() < MAX_STREAMS {
            if let Some(task) = pick_pending_block(ctx, &peer_avail).await {
                match conn.open_bidi_stream().await {
                    Ok((w, r)) => {
                        let id = next_id;
                        next_id += 1;
                        let (task_tx, task_rx) = mpsc::channel::<BlockTask>(1);
                        tokio::spawn(download_stream_worker(
                            id, w, r, task_rx, result_tx.clone(), ctx.clone(),
                        ));
                        let key = (task.fi, task.pi, task.bi);
                        let mut slot = StreamSlot { task_tx, active_block: None };
                        let _ = slot.task_tx.send(task).await;
                        slot.active_block = Some(key);
                        slots.insert(id, slot);
                        debug!(streams = slots.len(), "scaled up data streams");
                    }
                    Err(e) => {
                        // Bloque ya fue marcado InFlight por pick_pending_block;
                        // lo devolvemos a Pending si falla la apertura del stream.
                        warn!("open data stream: {e}");
                        // El scheduler lo verá como InFlight(1) sin consumidor;
                        // el timeout del stream worker lo marcará fallido eventualmente.
                        // Para apertura fallida lo reseteamos directamente:
                        if let Some(task_recov) = pick_pending_block(ctx, &peer_avail).await {
                            // Esto no es ideal pero es seguro: la pieza se reintentará.
                            ctx.schedulers[task_recov.fi].lock().await
                                .mark_block_failed(task_recov.pi, task_recov.bi);
                        }
                    }
                }
            }
        }

        // ─ Esperar eventos ─────────────────────────────────────────────
        tokio::select! {
            // Mensajes de control del peer
            msg_opt = ctrl_r.next() => {
                let msg = match msg_opt {
                    Some(Ok(m))  => m,
                    Some(Err(e)) => return Err(e.into()),
                    None         => return Ok(()),
                };

                match msg {
                    Message::KeepAlive => {}

                    Message::HaveAll { file_index: fi } => {
                        let fi = fi as usize;
                        if fi < num_files {
                            let num = ctx.schedulers[fi].lock().await.num_pieces();
                            let old = peer_avail[fi].as_bitfield(num);
                            ctx.schedulers[fi].lock().await.remove_peer_bitfield(&old);
                            ctx.schedulers[fi].lock().await.add_peer_bitfield(&vec![true; num]);
                            peer_avail[fi] = PeerAvail::HaveAll;
                        }
                    }
                    Message::HaveNone { file_index: fi } => {
                        let fi = fi as usize;
                        if fi < num_files {
                            let num = ctx.schedulers[fi].lock().await.num_pieces();
                            let old = peer_avail[fi].as_bitfield(num);
                            ctx.schedulers[fi].lock().await.remove_peer_bitfield(&old);
                            peer_avail[fi] = PeerAvail::HaveNone;
                        }
                    }
                    Message::HaveBitmap { file_index: fi, bitmap } => {
                        let fi = fi as usize;
                        if fi < num_files {
                            let num      = ctx.schedulers[fi].lock().await.num_pieces();
                            let old      = peer_avail[fi].as_bitfield(num);
                            let new_bits = bytes_to_bitfield(&bitmap, num);
                            ctx.schedulers[fi].lock().await.remove_peer_bitfield(&old);
                            ctx.schedulers[fi].lock().await.add_peer_bitfield(&new_bits);
                            peer_avail[fi] = PeerAvail::Bitmap(new_bits);
                        }
                    }
                    Message::HavePiece { file_index: fi, piece_index: pi } => {
                        let fi = fi as usize;
                        if fi < num_files {
                            ctx.schedulers[fi].lock().await.add_peer_have(pi as usize);
                            match &mut peer_avail[fi] {
                                PeerAvail::Bitmap(v) => {
                                    if (pi as usize) < v.len() { v[pi as usize] = true; }
                                }
                                other => {
                                    let num = ctx.schedulers[fi].lock().await.num_pieces();
                                    let mut b = other.as_bitfield(num);
                                    if (pi as usize) < num { b[pi as usize] = true; }
                                    *other = PeerAvail::Bitmap(b);
                                }
                            }
                        }
                    }
                    Message::Bye { reason } => {
                        info!("peer bye: {reason}");
                        return Ok(());
                    }
                    _ => {}
                }
            }

            // Resultados de los stream workers
            event = result_rx.recv() => {
                match event {
                    Some(StreamResult::BlockOk { stream_id, fi, pi, bi }) => {
                        if let Some(slot) = slots.get_mut(&stream_id) {
                            slot.active_block = None;
                        }

                        let piece_ready = ctx.schedulers[fi].lock().await
                            .mark_block_done(pi, bi);

                        if piece_ready {
                            // Verificar SHA-256 y anunciar si ok.
                            if ctx.verify_and_complete(fi, pi).await {
                                let _ = ctrl_w.send(Message::HavePiece {
                                    file_index: fi as u16, piece_index: pi,
                                }).await;
                            }
                        }
                    }

                    Some(StreamResult::BlockFail { stream_id, fi, pi, bi }) => {
                        if let Some(slot) = slots.get_mut(&stream_id) {
                            slot.active_block = None;
                        }
                        ctx.schedulers[fi].lock().await.mark_block_failed(pi, bi);
                    }

                    Some(StreamResult::StreamDead { stream_id }) => {
                        if let Some(slot) = slots.remove(&stream_id) {
                            if let Some((fi, pi, bi)) = slot.active_block {
                                ctx.schedulers[fi].lock().await.mark_block_failed(pi, bi as u32);
                            }
                        }
                        if slots.is_empty() {
                            return Err(anyhow!("all data streams dead"));
                        }
                    }

                    None => return Ok(()),
                }
            }

            _ = ka_timer.tick() => {
                ctrl_w.send(Message::KeepAlive).await?;
            }
        }
    }
}

// ── Bucle del seeder (conexión entrante) ──────────────────────────────────────

async fn run_peer_seeder(
    conn:      &PeerConnection,
    ctx:       &Arc<TorrentCtx>,
    _peer_id:  &[u8; 32],
) -> Result<()> {
    // Stream 1 ya fue gestionado por handle_inbound (Hello / HelloAck).
    // Aceptamos el stream 2 como canal de control.
    let (mut ctrl_w, mut ctrl_r) = conn.accept_bidi_stream().await?;

    // Anunciar nuestra disponibilidad al downloader.
    send_our_bitfields(ctx, &mut ctrl_w).await?;

    let mut ka_timer = interval(KEEPALIVE_INTERVAL);
    ka_timer.tick().await;

    loop {
        tokio::select! {
            msg_opt = ctrl_r.next() => {
                match msg_opt {
                    Some(Ok(Message::KeepAlive))          => {}
                    Some(Ok(Message::HavePiece { .. }))   => {}
                    Some(Ok(Message::HaveNone  { .. }))   => {}
                    Some(Ok(Message::HaveAll   { .. }))   => {}
                    Some(Ok(Message::HaveBitmap { .. }))  => {}
                    Some(Ok(Message::Bye { reason }))     => {
                        info!("peer bye: {reason}");
                        return Ok(());
                    }
                    Some(Ok(_))  => {}
                    Some(Err(e)) => return Err(e.into()),
                    None         => return Ok(()),
                }
            }

            // Aceptar nuevos data streams del downloader.
            stream = conn.accept_bidi_stream() => {
                match stream {
                    Ok((w, r)) => {
                        tokio::spawn(serve_data_stream(w, r, ctx.clone()));
                    }
                    Err(e) => {
                        debug!("accept data stream: {e}");
                        break;
                    }
                }
            }

            _ = ka_timer.tick() => {
                ctrl_w.send(Message::KeepAlive).await?;
            }
        }
    }

    Ok(())
}

// ── Punto de entrada por peer ─────────────────────────────────────────────────

pub async fn run_peer(
    conn:     PeerConnection,
    ctx:      Arc<TorrentCtx>,
    peer_id:  [u8; 32],
    outbound: bool,
) {
    let addr = conn.remote_addr();
    ctx.peer_count.fetch_add(1, Ordering::Relaxed);

    let result = if outbound {
        run_peer_downloader(&conn, &ctx, &peer_id).await
    } else {
        run_peer_seeder(&conn, &ctx, &peer_id).await
    };

    if let Err(e) = result {
        debug!(%addr, "peer: {e}");
    }

    ctx.peer_count.fetch_sub(1, Ordering::Relaxed);
}

// ── Accept loop (QUIC inbound) ────────────────────────────────────────────────

async fn handle_inbound(
    conn:     PeerConnection,
    torrents: Arc<HashMap<[u8; 32], Arc<TorrentCtx>>>,
    peer_id:  [u8; 32],
) -> Result<()> {
    // Stream 1: Hello / HelloAck
    let (mut hello_w, mut hello_r) = conn.accept_bidi_stream().await?;

    let msg = timeout(HELLO_TIMEOUT, hello_r.next()).await
        .map_err(|_| anyhow!("hello timeout"))?
        .ok_or_else(|| anyhow!("disconnected during hello"))??;

    let info_hash = match msg {
        Message::Hello { info_hash, .. } => info_hash,
        _ => return Err(anyhow!("expected Hello")),
    };

    match torrents.get(&info_hash) {
        Some(ctx) => {
            let ack = Message::HelloAck { peer_id, accepted: true, reason: None };
            timeout(HELLO_TIMEOUT, hello_w.send(ack)).await
                .map_err(|_| anyhow!("ack timeout"))??;
            drop(hello_w);
            drop(hello_r);
            run_peer(conn, ctx.clone(), peer_id, false).await;
            Ok(())
        }
        None => {
            let ack = Message::HelloAck {
                peer_id,
                accepted: false,
                reason:   Some("torrent not found".into()),
            };
            let _ = timeout(HELLO_TIMEOUT, hello_w.send(ack)).await;
            Ok(())
        }
    }
}

async fn accept_loop(
    endpoint: Arc<QuicEndpoint>,
    torrents: Arc<HashMap<[u8; 32], Arc<TorrentCtx>>>,
    peer_id:  [u8; 32],
) {
    loop {
        match endpoint.accept().await {
            None => { info!("QUIC endpoint closed"); break; }
            Some(Err(e)) => { warn!("accept error: {e}"); }
            Some(Ok(conn)) => {
                let torrents = torrents.clone();
                tokio::spawn(async move {
                    if let Err(e) = handle_inbound(conn, torrents, peer_id).await {
                        debug!("inbound handshake: {e}");
                    }
                });
            }
        }
    }
}

// ── Tracker announce loop ─────────────────────────────────────────────────────

async fn tracker_loop(
    tracker_url:  String,
    ctx:          Arc<TorrentCtx>,
    endpoint:     Arc<QuicEndpoint>,
    peer_id:      [u8; 32],
    listen_port:  u16,
    known_peers:  Arc<Mutex<HashSet<String>>>,
) {
    let client = match TrackerClient::new(&tracker_url, None) {
        Ok(c)  => c,
        Err(e) => { warn!("tracker client: {e}"); return; }
    };

    let listen_addr = format!("127.0.0.1:{listen_port}");
    let mut timer   = interval(ANNOUNCE_INTERVAL);
    let mut event   = AnnounceEvent::Started;

    loop {
        timer.tick().await;

        let downloaded = ctx.downloaded.load(Ordering::Relaxed);
        let left       = ctx.meta.total_size().saturating_sub(downloaded);

        let req = AnnounceRequest::new(
            &ctx.meta.info_hash, &peer_id, &listen_addr,
            event, 0, downloaded, left,
        );

        match client.announce(req).await {
            Ok(resp) => {
                info!(tracker = %tracker_url, peers = resp.peers.len(), "announce ok");

                for peer_info in resp.peers {
                    let addr_str = peer_info.addr.clone();
                    let is_new   = known_peers.lock().await.insert(addr_str.clone());
                    if !is_new { continue; }

                    let ep   = endpoint.clone();
                    let ctx2 = ctx.clone();
                    tokio::spawn(async move {
                        match addr_str.parse::<std::net::SocketAddr>() {
                            Ok(addr) => match ep.connect(addr).await {
                                Ok(conn) => run_peer(conn, ctx2, peer_id, true).await,
                                Err(e)   => warn!("connect {addr}: {e}"),
                            },
                            Err(_) => warn!("invalid peer addr: {addr_str}"),
                        }
                    });
                }

                event = if ctx.is_complete().await {
                    AnnounceEvent::Completed
                } else {
                    AnnounceEvent::Update
                };
            }
            Err(e) => warn!("announce to {tracker_url}: {e}"),
        }
    }
}

// ── State persistence loop ────────────────────────────────────────────────────

async fn state_save_loop(
    state_path:  std::path::PathBuf,
    torrent_ids: Vec<(String, Arc<TorrentCtx>)>,
) {
    let mut timer = interval(STATE_SAVE_INTERVAL);
    timer.tick().await;

    loop {
        timer.tick().await;

        let mut state = match ClientState::load(&state_path) {
            Ok(s)  => s,
            Err(e) => { warn!("load state: {e}"); continue; }
        };

        for (id, ctx) in &torrent_ids {
            if let Some(entry) = state.get_mut(id) {
                entry.downloaded = ctx.downloaded.load(Ordering::Relaxed);
                entry.peers      = ctx.peer_count.load(Ordering::Relaxed);
                if ctx.is_complete().await && entry.state == DownloadState::Downloading {
                    entry.state = DownloadState::Seeding;
                    info!("[{id}] {} — completed!", entry.name);
                }
            }
        }

        if let Err(e) = state.save(&state_path) {
            warn!("save state: {e}");
        }
    }
}

// ── Punto de entrada del daemon ───────────────────────────────────────────────

pub async fn run_daemon(config: &Config, state_path: &Path) -> Result<()> {
    let mut our_peer_id = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut our_peer_id);

    let bind_addr: std::net::SocketAddr = format!("0.0.0.0:{}", config.listen_port)
        .parse()
        .context("parsing listen addr")?;
    let endpoint = Arc::new(QuicEndpoint::bind(bind_addr)?);

    let state = ClientState::load(state_path)?;
    let mut torrents: HashMap<[u8; 32], Arc<TorrentCtx>> = HashMap::new();
    let mut torrent_ids: Vec<(String, Arc<TorrentCtx>)>  = Vec::new();

    for entry in state.torrents.values() {
        let active = matches!(entry.state, DownloadState::Downloading | DownloadState::Seeding);
        if !active { continue; }

        if entry.metainfo_path.as_os_str().is_empty() {
            warn!("[{}] no metainfo_path — re-add the torrent", entry.id);
            continue;
        }

        let meta_bytes = match std::fs::read(&entry.metainfo_path) {
            Ok(b)  => b,
            Err(e) => { warn!("[{}] read metainfo: {e}", entry.id); continue; }
        };
        let meta: Metainfo = match serde_json::from_slice(&meta_bytes) {
            Ok(m)  => m,
            Err(e) => { warn!("[{}] parse metainfo: {e}", entry.id); continue; }
        };

        let seeding = entry.state == DownloadState::Seeding;
        let ctx = match TorrentCtx::new(meta, &entry.save_path, seeding).await {
            Ok(c)  => c,
            Err(e) => { warn!("[{}] init ctx: {e}", entry.id); continue; }
        };

        info!("[{}] {} — {}", entry.id, entry.name,
            if seeding { "seeding" } else { "downloading" });

        torrents.insert(ctx.meta.info_hash, ctx.clone());
        torrent_ids.push((entry.id.clone(), ctx));
    }

    let torrents = Arc::new(torrents);

    for (_, ctx) in &torrent_ids {
        let known_peers = Arc::new(Mutex::new(HashSet::<String>::new()));
        for tracker_url in &ctx.meta.trackers {
            tokio::spawn(tracker_loop(
                tracker_url.clone(),
                ctx.clone(),
                endpoint.clone(),
                our_peer_id,
                config.listen_port,
                known_peers.clone(),
            ));
        }
    }

    if !torrent_ids.is_empty() {
        tokio::spawn(state_save_loop(state_path.to_path_buf(), torrent_ids));
    }

    accept_loop(endpoint, torrents, our_peer_id).await;

    Ok(())
}
