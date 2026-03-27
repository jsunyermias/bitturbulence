use std::collections::HashMap;
use std::sync::Arc;

use anyhow::{anyhow, Context, Result};
use futures_util::{SinkExt, StreamExt};
use tokio::sync::mpsc;
use tokio::time::interval;
use tracing::{debug, info, Instrument, Span};

use bitturbulence_pieces::BlockTask;
use bitturbulence_protocol::Message;
use bitturbulence_transport::PeerConnection;

use super::context::FlowCtx;
use super::stream::{
    bitfield_to_bytes, bytes_to_bitfield, download_stream_worker, PeerAvail, StreamResult,
    StreamSlot, W,
};
use super::{DEFAULT_STREAMS, HELLO_TIMEOUT, KEEPALIVE_INTERVAL, MAX_STREAMS, PROTOCOL_VERSION};

// ── Helpers del coordinador ───────────────────────────────────────────────────

/// Envía nuestra disponibilidad de todos los archivos al peer.
pub async fn send_our_bitfields(ctx: &FlowCtx, ctrl_w: &mut W) -> Result<()> {
    for fi in 0..ctx.meta.files.len() {
        let bits = ctx.our_bitfield(fi).await;
        let msg = if bits.iter().all(|&h| h) {
            Message::HaveAll {
                file_index: fi as u16,
            }
        } else if bits.iter().all(|&h| !h) {
            Message::HaveNone {
                file_index: fi as u16,
            }
        } else {
            Message::HaveBitmap {
                file_index: fi as u16,
                bitmap: bytes::Bytes::from(bitfield_to_bytes(&bits)),
            }
        };
        ctrl_w.send(msg).await?;
    }
    Ok(())
}

// ── Constantes de detección de modo ──────────────────────────────────────────

/// Bloques pendientes totales por debajo de los cuales se activa el modo endgame.
/// En endgame se escala a MAX_STREAMS y se permite redundancia (InFlight) en el scale-up.
const ENDGAME_THRESHOLD: u32 = 16;

/// Número de fallos de bloque consecutivos que clasifican la conexión como inestable.
/// En modo inestable se comporta igual que endgame (más streams, más redundancia).
const INSTABILITY_FAIL_THRESHOLD: u32 = 3;

/// Bloques completados con éxito tras los que se resetea el contador de fallos.
const INSTABILITY_RESET_WINDOW: u32 = 32;

/// Timeouts de bloque consecutivos sin ningún `BlockOk` de por medio tras los
/// cuales se desconecta el peer definitivamente.
/// Con SNUB_TIMEOUT=60s esto equivale a ~5 minutos de inactividad total.
pub(crate) const SNUB_DISCONNECT_THRESHOLD: u32 = 5;

// ── Handshake saliente ────────────────────────────────────────────────────────

async fn outbound_handshake(
    conn: &PeerConnection,
    ctx: &FlowCtx,
    peer_id: &[u8; 32],
) -> Result<()> {
    use bitturbulence_protocol::AuthPayload;

    let (mut hello_w, mut hello_r) = conn.open_bidi_stream().await?;

    hello_w
        .send(Message::Hello {
            version: PROTOCOL_VERSION,
            peer_id: *peer_id,
            info_hash: ctx.meta.info_hash,
            auth: AuthPayload::None,
        })
        .await
        .context("sending hello")?;

    let ack = tokio::time::timeout(HELLO_TIMEOUT, hello_r.next())
        .await
        .map_err(|_| anyhow!("hello ack timeout"))?
        .ok_or_else(|| anyhow!("disconnected during hello"))??;

    match ack {
        Message::HelloAck { accepted: true, .. } => Ok(()),
        Message::HelloAck {
            accepted: false,
            reason,
            ..
        } => Err(anyhow!("hello rejected: {}", reason.unwrap_or_default())),
        _ => Err(anyhow!("expected HelloAck")),
    }
}

// ── Bucle del drainer (conexión saliente) ─────────────────────────────────────

pub async fn run_peer_downloader(
    conn: &PeerConnection,
    ctx: &Arc<FlowCtx>,
    peer_id: &[u8; 32],
) -> Result<()> {
    outbound_handshake(conn, ctx, peer_id).await?;

    // ── Stream 2: control (Have*, KeepAlive, Bye) ───────────────────────
    let (mut ctrl_w, mut ctrl_r) = conn.open_bidi_stream().await?;
    send_our_bitfields(ctx, &mut ctrl_w).await?;

    // ── Comprobar si el flow ya está completo antes de abrir streams ────
    let mut complete_rx = ctx.complete_tx.subscribe();
    let mut have_piece_rx = ctx.have_piece_tx.subscribe();
    if *complete_rx.borrow() {
        return Ok(());
    }

    // ── Streams 3…N: datos por bloque ───────────────────────────────────
    let num_files = ctx.meta.files.len();
    let (result_tx, mut result_rx) = mpsc::channel::<StreamResult>(MAX_STREAMS * 8);
    let mut slots: HashMap<usize, StreamSlot> = HashMap::new();
    let mut next_id: usize = 0;
    let mut peer_avail: Vec<PeerAvail> = vec![PeerAvail::Unknown; num_files];

    for _ in 0..DEFAULT_STREAMS {
        let (w, r) = conn.open_bidi_stream().await?;
        let (task_tx, task_rx) = mpsc::channel::<BlockTask>(1);
        tokio::spawn(
            download_stream_worker(next_id, w, r, task_rx, result_tx.clone(), ctx.clone())
                .instrument(Span::current()),
        );
        slots.insert(
            next_id,
            StreamSlot {
                task_tx,
                active_block: None,
            },
        );
        next_id += 1;
    }

    let mut ka_timer = interval(KEEPALIVE_INTERVAL);
    ka_timer.tick().await;

    // Contadores para detección de inestabilidad.
    let mut recent_fails: u32 = 0;
    let mut blocks_ok: u32 = 0;
    // Timeouts de bloque consecutivos sin BlockOk de por medio.
    //   0                          → normal
    //   1 ..< SNUB_DISCONNECT_THRESHOLD → snubbed (1 stream activo)
    //   >= SNUB_DISCONNECT_THRESHOLD    → desconectar
    let mut snub_streak: u32 = 0;

    // ── Bucle coordinador ──────────────────────────────────────────────
    loop {
        // ─ Calcular modo de operación ──────────────────────────────────
        //
        // Modos (por orden de prioridad):
        //   Normal   : DEFAULT_STREAMS, solo bloques Pending.
        //   Endgame  : MAX_STREAMS, también bloques InFlight (redundancia).
        //   Inestable: igual que endgame (compensa fallos de red).
        let unstable = recent_fails >= INSTABILITY_FAIL_THRESHOLD;
        let endgame = ctx.sched.total_pending().await <= ENDGAME_THRESHOLD;
        let endgame_mode = unstable || endgame;
        let max_active = if snub_streak > 0 {
            1
        } else if endgame_mode {
            MAX_STREAMS
        } else {
            DEFAULT_STREAMS
        };

        // ─ Scale-down: cerrar slots idle si superamos max_active ──────
        //
        // Ocurre cuando salimos del modo inestable (recent_fails reset tras
        // INSTABILITY_RESET_WINDOW bloques ok) sin estar en endgame.
        // Cerramos solo slots idle: soltar task_tx → el worker QUIC sale
        // limpiamente al recibir None en su task_rx.
        if slots.len() > max_active {
            let excess = slots.len() - max_active;
            let to_close: Vec<usize> = slots
                .iter()
                .filter(|(_, s)| s.active_block.is_none())
                .map(|(&id, _)| id)
                .take(excess)
                .collect();
            for id in to_close {
                slots.remove(&id);
                debug!(streams = slots.len(), max_active, "scaled down data stream");
            }
        }

        // ─ Asignar bloques a slots idle ────────────────────────────────
        for slot in slots.values_mut() {
            if slot.active_block.is_none() {
                if let Some(task) = ctx.sched.pick_block(&peer_avail, endgame_mode).await {
                    slot.active_block = Some(task.clone());
                    let _ = slot.task_tx.send(task).await;
                }
            }
        }

        // ─ Scale-up adaptativo ─────────────────────────────────────────
        if slots.len() < max_active {
            if let Some(task) = ctx.sched.pick_block(&peer_avail, endgame_mode).await {
                match conn.open_bidi_stream().await {
                    Ok((w, r)) => {
                        let id = next_id;
                        next_id += 1;
                        let (task_tx, task_rx) = mpsc::channel::<BlockTask>(1);
                        tokio::spawn(
                            download_stream_worker(
                                id,
                                w,
                                r,
                                task_rx,
                                result_tx.clone(),
                                ctx.clone(),
                            )
                            .instrument(Span::current()),
                        );
                        let mut slot = StreamSlot {
                            task_tx,
                            active_block: None,
                        };
                        slot.active_block = Some(task.clone());
                        let _ = slot.task_tx.send(task).await;
                        slots.insert(id, slot);
                        debug!(
                            streams = slots.len(),
                            endgame, unstable, "scaled up data streams"
                        );
                    }
                    Err(e) => {
                        tracing::warn!("open data stream: {e}");
                        ctx.sched.block_failed(task.fi, task.pi, task.bi).await;
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
                            let num = ctx.meta.files[fi].piece_hashes.len();
                            let old = peer_avail[fi].as_bitfield(num);
                            ctx.sched.remove_peer_bitfield(fi, old).await;
                            ctx.sched.add_peer_bitfield(fi, vec![true; num]).await;
                            peer_avail[fi] = PeerAvail::HaveAll;
                        }
                    }
                    Message::HaveNone { file_index: fi } => {
                        let fi = fi as usize;
                        if fi < num_files {
                            let num = ctx.meta.files[fi].piece_hashes.len();
                            let old = peer_avail[fi].as_bitfield(num);
                            ctx.sched.remove_peer_bitfield(fi, old).await;
                            peer_avail[fi] = PeerAvail::HaveNone;
                        }
                    }
                    Message::HaveBitmap { file_index: fi, bitmap } => {
                        let fi = fi as usize;
                        if fi < num_files {
                            let num      = ctx.meta.files[fi].piece_hashes.len();
                            let old      = peer_avail[fi].as_bitfield(num);
                            let new_bits = bytes_to_bitfield(&bitmap, num);
                            ctx.sched.remove_peer_bitfield(fi, old).await;
                            ctx.sched.add_peer_bitfield(fi, new_bits.clone()).await;
                            peer_avail[fi] = PeerAvail::Bitmap(new_bits);
                        }
                    }
                    Message::HavePiece { file_index: fi, piece_index: pi } => {
                        let fi = fi as usize;
                        if fi < num_files {
                            ctx.sched.add_peer_have(fi, pi as usize).await;
                            match &mut peer_avail[fi] {
                                PeerAvail::Bitmap(v) => {
                                    if (pi as usize) < v.len() { v[pi as usize] = true; }
                                }
                                other => {
                                    let num = ctx.meta.files[fi].piece_hashes.len();
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
                        // En modo endgame puede haber otros streams descargando
                        // el mismo bloque. Cancelarlos para ahorrar ancho de banda.
                        for slot in slots.values() {
                            if let Some(ref t) = slot.active_block {
                                if t.fi == fi && t.pi == pi && t.bi == bi {
                                    let _ = ctrl_w
                                        .send(Message::Cancel {
                                            file_index: fi as u16,
                                            piece_index: pi,
                                            begin: t.begin,
                                            length: t.length,
                                        })
                                        .await;
                                }
                            }
                        }
                        let piece_ready = ctx.sched.block_done(fi, pi, bi, hash).await;
                        if piece_ready {
                            ctx.verify_and_complete(fi, pi).await;
                        }
                        // Ventana deslizante: resetear fallos cada INSTABILITY_RESET_WINDOW bloques.
                        blocks_ok += 1;
                        if blocks_ok >= INSTABILITY_RESET_WINDOW {
                            recent_fails = 0;
                            blocks_ok    = 0;
                        }
                        // El peer respondió: resetear streak de snubbing.
                        snub_streak = 0;
                    }
                    Some(StreamResult::BlockFail { stream_id, fi, pi, bi }) => {
                        if let Some(slot) = slots.get_mut(&stream_id) {
                            slot.active_block = None;
                        }
                        ctx.sched.block_failed(fi, pi, bi).await;
                        recent_fails += 1;
                    }
                    Some(StreamResult::BlockTimeout { stream_id, fi, pi, bi }) => {
                        if let Some(slot) = slots.get_mut(&stream_id) {
                            slot.active_block = None;
                        }
                        ctx.sched.block_failed(fi, pi, bi).await;
                        snub_streak += 1;
                        tracing::warn!(
                            fi, pi, bi,
                            snub_streak,
                            threshold = SNUB_DISCONNECT_THRESHOLD,
                            "peer snubbed: no respondió en SNUB_TIMEOUT"
                        );
                        if snub_streak >= SNUB_DISCONNECT_THRESHOLD {
                            return Err(anyhow!(
                                "peer persistently snubbed ({snub_streak} timeouts consecutivos)"
                            ));
                        }
                    }
                    Some(StreamResult::StreamDead { stream_id }) => {
                        if let Some(slot) = slots.remove(&stream_id) {
                            if let Some(ref t) = slot.active_block {
                                ctx.sched.block_failed(t.fi, t.pi, t.bi).await;
                                // Un stream muerto es inestabilidad, igual que un BlockFail.
                                recent_fails += 1;
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

            piece = have_piece_rx.recv() => {
                match piece {
                    Ok((fi, pi)) => {
                        let _ = ctrl_w.send(Message::HavePiece {
                            file_index: fi as u16,
                            piece_index: pi,
                        }).await;
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                        debug!("have_piece_rx lagged by {n}");
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => return Ok(()),
                }
            }

            _ = complete_rx.changed() => {
                // El flow se completó (esta u otra conexión verificó la última pieza).
                return Ok(());
            }
        }
    }
}
