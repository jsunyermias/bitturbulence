use std::collections::HashMap;
use std::sync::Arc;

use anyhow::{anyhow, Context, Result};
use futures_util::{SinkExt, StreamExt};
use tokio::sync::mpsc;
use tokio::time::interval;
use tracing::{debug, info};

use bitturbulence_pieces::BlockTask;
use bitturbulence_protocol::{Message, Priority};
use bitturbulence_transport::PeerConnection;

use super::context::FlowCtx;
use super::stream::{
    PeerAvail, StreamResult, StreamSlot, W,
    bitfield_to_bytes, bytes_to_bitfield, download_stream_worker,
};
use super::{DEFAULT_STREAMS, MAX_STREAMS, KEEPALIVE_INTERVAL, HELLO_TIMEOUT, PROTOCOL_VERSION};

// ── Helpers del coordinador ───────────────────────────────────────────────────

/// Envía nuestra disponibilidad de todos los archivos al peer.
pub async fn send_our_bitfields(ctx: &FlowCtx, ctrl_w: &mut W) -> Result<()> {
    for fi in 0..ctx.meta.files.len() {
        let bits = ctx.our_bitfield(fi).await;
        let msg = if bits.iter().all(|&h| h) {
            Message::HaveAll  { file_index: fi as u16 }
        } else if bits.iter().all(|&h| !h) {
            Message::HaveNone { file_index: fi as u16 }
        } else {
            Message::HaveBitmap {
                file_index: fi as u16,
                bitmap:     bytes::Bytes::from(bitfield_to_bytes(&bits)),
            }
        };
        ctrl_w.send(msg).await?;
    }
    Ok(())
}

/// Umbral de "baja disponibilidad": el archivo lo tiene como máximo este número de peers.
const LOW_AVAIL: u32 = 1;

/// Bloques pendientes totales por debajo de los cuales se activa el modo endgame.
/// En endgame se escala a MAX_STREAMS y se permite redundancia (InFlight) en el scale-up.
const ENDGAME_THRESHOLD: u32 = 16;

/// Número de fallos de bloque consecutivos que clasifican la conexión como inestable.
/// En modo inestable se comporta igual que endgame (más streams, más redundancia).
const INSTABILITY_FAIL_THRESHOLD: u32 = 3;

/// Bloques completados con éxito tras los que se resetea el contador de fallos.
const INSTABILITY_RESET_WINDOW: u32 = 32;

/// Cuenta los bloques `Pending` en todos los archivos del flow.
async fn total_pending_blocks(ctx: &FlowCtx) -> u32 {
    let mut n = 0u32;
    for sched in &ctx.schedulers {
        n += sched.lock().await.pending_blocks();
    }
    n
}

/// Passes de prioridad inter-archivo: (prioridad requerida, máx. disponibilidad del archivo).
/// `None` en el segundo campo significa sin filtro de rareza.
const PRIORITY_PASSES: &[(Priority, Option<u32>)] = &[
    (Priority::Maximum, Some(LOW_AVAIL)),   // 1. Máxima + rareza
    (Priority::VeryHigh, Some(LOW_AVAIL)),  // 2. Muy alta + rareza
    (Priority::Maximum, None),              // 3. Máxima
    (Priority::Higher, Some(LOW_AVAIL)),    // 4. Más alta + rareza
    (Priority::VeryHigh, None),             // 5. Muy alta
    (Priority::High, Some(LOW_AVAIL)),      // 6. Alta + rareza
    (Priority::Higher, None),               // 7. Más alta
    (Priority::High, None),                 // 8. Alta
];

/// Selecciona el siguiente bloque a descargar para un stream existente.
///
/// Orden de selección entre archivos:
/// 1–8. Bloques `Pending` de archivos con prioridad de usuario alta × rareza.
///  9.  Bloques `Pending` de archivos con prioridad Normal o inferior.
/// 10–11. Bloques `InFlight` (redundancia/endgame) de cualquier archivo.
async fn pick_block(ctx: &FlowCtx, peer_avail: &[PeerAvail]) -> Option<BlockTask> {
    // Passes 1–8: Pending, priority × rareza del archivo.
    for &(prio, max_avail) in PRIORITY_PASSES {
        for (fi, avail) in peer_avail.iter().enumerate() {
            let mut sched = ctx.schedulers[fi].lock().await;
            if sched.priority() != prio { continue; }
            if max_avail.is_some_and(|m| sched.min_availability() > m) { continue; }
            let bits = avail.as_bitfield(sched.num_pieces());
            if bits.iter().all(|&h| !h) { continue; }
            if let Some(task) = sched.schedule_pending(&bits) {
                return Some(task);
            }
        }
    }

    // Pass 9: Pending de archivos con prioridad Normal o inferior.
    for (fi, avail) in peer_avail.iter().enumerate() {
        let mut sched = ctx.schedulers[fi].lock().await;
        if sched.priority() >= Priority::High { continue; }
        let bits = avail.as_bitfield(sched.num_pieces());
        if bits.iter().all(|&h| !h) { continue; }
        if let Some(task) = sched.schedule_pending(&bits) {
            return Some(task);
        }
    }

    // Passes 10–11: InFlight (redundancia/endgame) de cualquier archivo.
    // schedule() salta los Pending (ya cubiertos arriba) y devuelve InFlight.
    for (fi, avail) in peer_avail.iter().enumerate() {
        let mut sched = ctx.schedulers[fi].lock().await;
        let bits = avail.as_bitfield(sched.num_pieces());
        if bits.iter().all(|&h| !h) { continue; }
        if let Some(task) = sched.schedule(&bits) {
            return Some(task);
        }
    }

    None
}

/// Selecciona un bloque `Pending` para decidir si abrir un nuevo stream.
///
/// Aplica el mismo orden de prioridad inter-archivo que [`pick_block`],
/// pero nunca devuelve bloques `InFlight`.
async fn pick_pending_block(ctx: &FlowCtx, peer_avail: &[PeerAvail]) -> Option<BlockTask> {
    for &(prio, max_avail) in PRIORITY_PASSES {
        for (fi, avail) in peer_avail.iter().enumerate() {
            let mut sched = ctx.schedulers[fi].lock().await;
            if sched.priority() != prio { continue; }
            if max_avail.is_some_and(|m| sched.min_availability() > m) { continue; }
            let bits = avail.as_bitfield(sched.num_pieces());
            if bits.iter().all(|&h| !h) { continue; }
            if let Some(task) = sched.schedule_pending(&bits) {
                return Some(task);
            }
        }
    }

    for (fi, avail) in peer_avail.iter().enumerate() {
        let mut sched = ctx.schedulers[fi].lock().await;
        if sched.priority() >= Priority::High { continue; }
        let bits = avail.as_bitfield(sched.num_pieces());
        if bits.iter().all(|&h| !h) { continue; }
        if let Some(task) = sched.schedule_pending(&bits) {
            return Some(task);
        }
    }

    None
}

// ── Bucle del drainer (conexión saliente) ─────────────────────────────────────

pub async fn run_peer_downloader(
    conn:    &PeerConnection,
    ctx:     &Arc<FlowCtx>,
    peer_id: &[u8; 32],
) -> Result<()> {
    use bitturbulence_protocol::AuthPayload;

    // ── Stream 1: Hello / HelloAck ──────────────────────────────────────
    let (mut hello_w, mut hello_r) = conn.open_bidi_stream().await?;

    hello_w.send(Message::Hello {
        version:   PROTOCOL_VERSION,
        peer_id:   *peer_id,
        info_hash: ctx.meta.info_hash,
        auth:      AuthPayload::None,
    }).await.context("sending hello")?;

    let ack = tokio::time::timeout(HELLO_TIMEOUT, hello_r.next()).await
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

    // Contadores para detección de inestabilidad.
    let mut recent_fails: u32 = 0;
    let mut blocks_ok:    u32 = 0;

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

        // ─ Scale-up adaptativo ─────────────────────────────────────────
        //
        // Modos de operación (por orden de prioridad):
        //   Normal   : DEFAULT_STREAMS, solo bloques Pending.
        //   Endgame  : MAX_STREAMS, también bloques InFlight (redundancia
        //              hasta TYPICAL_STREAMS_PER_BLOCK / MAX_STREAMS_PER_BLOCK).
        //   Inestable: igual que endgame (más redundancia para compensar fallos).
        let unstable   = recent_fails >= INSTABILITY_FAIL_THRESHOLD;
        let endgame    = total_pending_blocks(ctx).await <= ENDGAME_THRESHOLD;
        let max_active = if unstable || endgame { MAX_STREAMS } else { DEFAULT_STREAMS };

        if slots.len() < max_active {
            let task_opt = if unstable || endgame {
                // Incluye bloques InFlight(< TYPICAL) y InFlight(< MAX) en la selección.
                pick_block(ctx, &peer_avail).await
            } else {
                pick_pending_block(ctx, &peer_avail).await
            };
            if let Some(task) = task_opt {
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
                        debug!(
                            streams = slots.len(),
                            endgame, unstable,
                            "scaled up data streams"
                        );
                    }
                    Err(e) => {
                        tracing::warn!("open data stream: {e}");
                        ctx.schedulers[task.fi].lock().await
                            .mark_block_failed(task.pi, task.bi);
                    }
                }
            }
        }

        // ─ Esperar eventos ─────────────────────────────────────────────
        tokio::select! {
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

            event = result_rx.recv() => {
                match event {
                    Some(StreamResult::BlockOk { stream_id, fi, pi, bi, hash }) => {
                        if let Some(slot) = slots.get_mut(&stream_id) {
                            slot.active_block = None;
                        }
                        let piece_ready = ctx.schedulers[fi].lock().await
                            .mark_block_done(pi, bi, hash);
                        if piece_ready {
                            if ctx.verify_and_complete(fi, pi).await {
                                let _ = ctrl_w.send(Message::HavePiece {
                                    file_index: fi as u16, piece_index: pi,
                                }).await;
                            }
                        }
                        // Ventana deslizante: resetear fallos cada INSTABILITY_RESET_WINDOW bloques.
                        blocks_ok += 1;
                        if blocks_ok >= INSTABILITY_RESET_WINDOW {
                            recent_fails = 0;
                            blocks_ok    = 0;
                        }
                    }
                    Some(StreamResult::BlockFail { stream_id, fi, pi, bi }) => {
                        if let Some(slot) = slots.get_mut(&stream_id) {
                            slot.active_block = None;
                        }
                        ctx.schedulers[fi].lock().await.mark_block_failed(pi, bi);
                        recent_fails += 1;
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
