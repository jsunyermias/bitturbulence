use std::sync::Arc;

use bytes::Bytes;
use futures_util::{SinkExt, StreamExt};
use tokio::sync::mpsc;
use tracing::{debug, warn};

use bitturbulence_pieces::{BlockScheduler, BlockTask, hash_block};
use bitturbulence_protocol::{Message, MessageCodec};
use bitturbulence_transport::PeerConnection;
use quinn::{RecvStream, SendStream};
use tokio_util::codec::{FramedRead, FramedWrite};

use super::context::TorrentCtx;

pub type W = FramedWrite<SendStream, MessageCodec>;
pub type R = FramedRead<RecvStream, MessageCodec>;

// ── Disponibilidad del peer ───────────────────────────────────────────────────

#[derive(Clone)]
pub enum PeerAvail {
    Unknown,
    HaveAll,
    HaveNone,
    Bitmap(Vec<bool>),
}

impl PeerAvail {
    pub fn as_bitfield(&self, num: usize) -> Vec<bool> {
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

pub fn bitfield_to_bytes(bits: &[bool]) -> Vec<u8> {
    let mut bytes = vec![0u8; bits.len().div_ceil(8)];
    for (i, &has) in bits.iter().enumerate() {
        if has { bytes[i / 8] |= 0x80 >> (i % 8); }
    }
    bytes
}

pub fn bytes_to_bitfield(bytes: &[u8], num: usize) -> Vec<bool> {
    (0..num)
        .map(|i| bytes.get(i / 8).is_some_and(|b| (b >> (7 - (i % 8))) & 1 == 1))
        .collect()
}

// ── Tipos del sistema multi-stream por bloque ─────────────────────────────────

/// Resultado reportado por un stream worker al coordinador.
pub enum StreamResult {
    /// Bloque descargado y escrito en disco. `hash` = SHA-256(block_data).
    BlockOk   { stream_id: usize, fi: usize, pi: u32, bi: u32, hash: [u8; 32] },
    /// Bloque fallido (Reject del filler o error de red).
    BlockFail { stream_id: usize, fi: usize, pi: u32, bi: u32 },
    /// Stream muerto por error QUIC irrecuperable.
    StreamDead { stream_id: usize },
}

/// Slot que representa un stream de datos activo en el coordinador.
pub struct StreamSlot {
    pub task_tx:      mpsc::Sender<BlockTask>,
    /// Bloque actualmente asignado (fi, pi, bi), o None si idle.
    pub active_block: Option<(usize, u32, u32)>,
}

// ── Stream worker: descarga un bloque a la vez ────────────────────────────────

/// Descarga repetidamente bloques asignados por el coordinador sobre un
/// stream QUIC dedicado.
pub async fn download_stream_worker(
    stream_id: usize,
    mut writer: W,
    mut reader: R,
    mut task_rx: mpsc::Receiver<BlockTask>,
    result_tx:  mpsc::Sender<StreamResult>,
    ctx:        Arc<TorrentCtx>,
) {
    while let Some(BlockTask { fi, pi, bi, begin, length }) = task_rx.recv().await {
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

/// Procesa `Request`s del drainer sobre un stream de datos dedicado
/// y responde con `Piece` o `Reject` bloque a bloque.
pub async fn serve_data_stream(
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
            Ok(_)  => {}
            Err(e) => { debug!("data stream error: {e}"); break; }
        }
    }
}
