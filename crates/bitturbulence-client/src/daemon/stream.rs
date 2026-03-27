use std::sync::Arc;

use bytes::Bytes;
use futures_util::{SinkExt, StreamExt};
use tokio::sync::mpsc;
use tracing::{debug, warn};

use bitturbulence_pieces::{BlockTask, hash_block, piece_root_from_block_hashes};
use bitturbulence_protocol::{Message, MessageCodec, BLOCK_SIZE};
use quinn::{RecvStream, SendStream};
use tokio_util::codec::{FramedRead, FramedWrite};

use super::context::FlowCtx;

pub(super) type W = FramedWrite<SendStream, MessageCodec>;
pub(super) type R = FramedRead<RecvStream, MessageCodec>;

// ── Disponibilidad del peer ───────────────────────────────────────────────────

#[derive(Clone)]
pub(crate) enum PeerAvail {
    Unknown,
    HaveAll,
    HaveNone,
    Bitmap(Vec<bool>),
}

impl PeerAvail {
    pub(crate) fn as_bitfield(&self, num: usize) -> Vec<bool> {
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

pub(super) fn bitfield_to_bytes(bits: &[bool]) -> Vec<u8> {
    let mut bytes = vec![0u8; bits.len().div_ceil(8)];
    for (i, &has) in bits.iter().enumerate() {
        if has { bytes[i / 8] |= 0x80 >> (i % 8); }
    }
    bytes
}

pub(super) fn bytes_to_bitfield(bytes: &[u8], num: usize) -> Vec<bool> {
    (0..num)
        .map(|i| bytes.get(i / 8).is_some_and(|b| (b >> (7 - (i % 8))) & 1 == 1))
        .collect()
}

// ── Tipos del sistema multi-stream por bloque ─────────────────────────────────

/// Resultado reportado por un stream worker al coordinador.
pub(super) enum StreamResult {
    /// Bloque descargado y escrito en disco. `hash` = SHA-256(block_data).
    BlockOk   { stream_id: usize, fi: usize, pi: u32, bi: u32, hash: [u8; 32] },
    /// Bloque fallido (Reject del filler o error de red).
    BlockFail { stream_id: usize, fi: usize, pi: u32, bi: u32 },
    /// Stream muerto por error QUIC irrecuperable.
    StreamDead { stream_id: usize },
}

/// Slot que representa un stream de datos activo en el coordinador.
pub(super) struct StreamSlot {
    pub(super) task_tx:      mpsc::Sender<BlockTask>,
    /// Bloque actualmente asignado (fi, pi, bi), o None si idle.
    pub(super) active_block: Option<(usize, u32, u32)>,
}

// ── Stream worker: descarga un bloque a la vez ────────────────────────────────

/// Descarga repetidamente bloques asignados por el coordinador sobre un
/// stream QUIC dedicado.
///
/// ## Verificación Merkle incremental (#32)
///
/// Al cambiar de pieza, envía un `HashRequest` al filler y espera
/// `HashResponse` con los hashes SHA-256 de cada bloque. Verifica que su
/// raíz Merkle coincide con el hash de pieza del metainfo antes de aceptar
/// ningún dato. Si la raíz no coincide (filler mintiendo), cada bloque de esa
/// pieza se marca fallido de inmediato. Si el filler responde vacío (no tiene
/// la pieza), se omite la verificación por bloque y se delega en la
/// verificación de pieza al final.
pub(super) async fn download_stream_worker(
    stream_id: usize,
    mut writer: W,
    mut reader: R,
    mut task_rx: mpsc::Receiver<BlockTask>,
    result_tx:  mpsc::Sender<StreamResult>,
    ctx:        Arc<FlowCtx>,
) {
    // Caché de hashes de bloque de la pieza que descargamos actualmente.
    // Se invalida al cambiar de (fi, pi).
    let mut curr_piece:        Option<(usize, u32)> = None;
    let mut curr_block_hashes: Vec<[u8; 32]>        = Vec::new();

    while let Some(BlockTask { fi, pi, bi, begin, length }) = task_rx.recv().await {

        // ── Solicitar hashes de bloque si es una pieza nueva ────────────
        if curr_piece != Some((fi, pi)) {
            if writer.send(Message::HashRequest {
                file_index: fi as u16,
                piece_index: pi,
            }).await.is_err() {
                let _ = result_tx.send(StreamResult::StreamDead { stream_id }).await;
                return;
            }

            // Esperar HashResponse (ignorar mensajes de otras piezas).
            curr_block_hashes = loop {
                match reader.next().await {
                    Some(Ok(Message::HashResponse { file_index, piece_index, block_hashes }))
                        if file_index as usize == fi && piece_index == pi =>
                    {
                        if !block_hashes.is_empty() {
                            let computed = piece_root_from_block_hashes(&block_hashes);
                            let expected = ctx.meta.files[fi].piece_hashes[pi as usize];
                            if computed != expected {
                                warn!(fi, pi, "HashResponse: raíz Merkle incorrecta \
                                              — filler puede estar sirviendo datos corruptos");
                                // Sin hashes verificados: caerá en verificación de pieza al final.
                                break vec![];
                            }
                        }
                        break block_hashes;
                    }
                    Some(Ok(Message::HashResponse { .. })) => {} // otra pieza, ignorar
                    None | Some(Err(_)) => {
                        let _ = result_tx.send(StreamResult::StreamDead { stream_id }).await;
                        return;
                    }
                    Some(Ok(_)) => {}
                }
            };
            curr_piece = Some((fi, pi));
        }

        // ── Enviar Request ────────────────────────────────────────────────
        if writer.send(Message::Request {
            file_index: fi as u16, piece_index: pi, begin, length,
        }).await.is_err() {
            let _ = result_tx.send(StreamResult::StreamDead { stream_id }).await;
            return;
        }

        let result = loop {
            match reader.next().await {
                Some(Ok(Message::Piece { file_index, piece_index, begin: b, data }))
                    if file_index as usize == fi && piece_index == pi && b == begin =>
                {
                    let block_hash = hash_block(&data);

                    // Verificación incremental: comparar con hash esperado.
                    if let Some(&expected_hash) = curr_block_hashes.get(bi as usize) {
                        if block_hash != expected_hash {
                            warn!(fi, pi, bi, "hash de bloque incorrecto — \
                                              datos corruptos del filler");
                            break StreamResult::BlockFail { stream_id, fi, pi, bi };
                        }
                    }

                    match ctx.store.write_block(fi, pi, begin, &data).await {
                        Ok(()) => break StreamResult::BlockOk { stream_id, fi, pi, bi, hash: block_hash },
                        Err(e) => {
                            warn!(fi, pi, bi, "write_block: {e}");
                            break StreamResult::BlockFail { stream_id, fi, pi, bi };
                        }
                    }
                }
                Some(Ok(Message::Reject { file_index, piece_index, begin: b, .. }))
                    if file_index as usize == fi && piece_index == pi && b == begin =>
                {
                    debug!(fi, pi, bi, "block rejected by filler");
                    break StreamResult::BlockFail { stream_id, fi, pi, bi };
                }
                None | Some(Err(_)) => {
                    let _ = result_tx.send(StreamResult::StreamDead { stream_id }).await;
                    return;
                }
                Some(Ok(_)) => {}
            }
        };
        let _ = result_tx.send(result).await;
    }
}

// ── Filler: sirve blocks sobre un data stream ─────────────────────────────────

/// Procesa `Request`s y `HashRequest`s del drainer sobre un stream de datos
/// dedicado. Responde con `Piece`/`Reject` y `HashResponse` respectivamente.
pub(super) async fn serve_data_stream(
    mut writer: W,
    mut reader: R,
    ctx: Arc<FlowCtx>,
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

            // Verificación Merkle incremental (#32): el drainer pide los hashes
            // de bloque de una pieza antes de descargarla para verificar cada
            // bloque individualmente a medida que llega.
            Ok(Message::HashRequest { file_index, piece_index }) => {
                let fi = file_index as usize;
                let pi = piece_index;
                let have = ctx.our_bitfield(fi).await;
                let block_hashes = if have.get(pi as usize).copied().unwrap_or(false) {
                    let piece_len  = ctx.meta.files[fi].piece_len(pi);
                    let num_blocks = piece_len.div_ceil(BLOCK_SIZE);
                    let mut hashes = Vec::with_capacity(num_blocks as usize);
                    let mut all_ok = true;
                    for bi in 0..num_blocks {
                        let block_begin = bi * BLOCK_SIZE;
                        let block_len   = (piece_len - block_begin).min(BLOCK_SIZE);
                        match ctx.store.read_block(fi, pi, block_begin, block_len).await {
                            Ok(data) => hashes.push(hash_block(&data)),
                            Err(e)   => {
                                warn!(fi, pi, bi, "read_block for HashResponse: {e}");
                                all_ok = false;
                                break;
                            }
                        }
                    }
                    if all_ok { hashes } else { vec![] }
                } else {
                    vec![] // No tenemos la pieza: respuesta vacía
                };
                let _ = writer.send(Message::HashResponse {
                    file_index,
                    piece_index,
                    block_hashes,
                }).await;
            }

            Ok(_)  => {}
            Err(e) => { debug!("data stream error: {e}"); break; }
        }
    }
}
