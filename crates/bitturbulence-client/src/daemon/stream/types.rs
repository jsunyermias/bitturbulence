use std::collections::HashSet;
use std::sync::{Arc, Mutex};

use tokio::sync::mpsc;

use bitturbulence_pieces::BlockTask;
use bitturbulence_protocol::MessageCodec;
use quinn::{RecvStream, SendStream};
use tokio_util::codec::{FramedRead, FramedWrite};

pub(crate) type W = FramedWrite<SendStream, MessageCodec>;
pub(crate) type R = FramedRead<RecvStream, MessageCodec>;

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
            Self::HaveAll => vec![true; num],
            Self::Bitmap(v) => {
                let mut out = v.clone();
                out.resize(num, false);
                out
            }
        }
    }
}

// ── Helpers de bitmap ─────────────────────────────────────────────────────────

pub(crate) fn bitfield_to_bytes(bits: &[bool]) -> Vec<u8> {
    let mut bytes = vec![0u8; bits.len().div_ceil(8)];
    for (i, &has) in bits.iter().enumerate() {
        if has {
            bytes[i / 8] |= 0x80 >> (i % 8);
        }
    }
    bytes
}

pub(crate) fn bytes_to_bitfield(bytes: &[u8], num: usize) -> Vec<bool> {
    (0..num)
        .map(|i| {
            bytes
                .get(i / 8)
                .is_some_and(|b| (b >> (7 - (i % 8))) & 1 == 1)
        })
        .collect()
}

// ── Tipos del sistema multi-stream por bloque ─────────────────────────────────

/// Resultado reportado por un stream worker al coordinador.
pub(crate) enum StreamResult {
    /// Bloque descargado y escrito en disco. `hash` = SHA-256(block_data).
    BlockOk {
        stream_id: usize,
        fi: usize,
        pi: u32,
        bi: u32,
        hash: [u8; 32],
    },
    /// Bloque fallido (Reject del filler o error de red).
    BlockFail {
        stream_id: usize,
        fi: usize,
        pi: u32,
        bi: u32,
    },
    /// El filler no respondió al Request en el tiempo máximo (`SNUB_TIMEOUT`).
    /// El coordinador usa esto para detectar peers snubbed.
    BlockTimeout {
        stream_id: usize,
        fi: usize,
        pi: u32,
        bi: u32,
    },
    /// Stream muerto por error QUIC irrecuperable.
    StreamDead { stream_id: usize },
}

/// Conjunto de bloques cancelados: clave (file_index, piece_index, begin).
///
/// Compartido entre el bucle de control del filler y los data stream workers.
/// El control loop inserta al recibir Cancel; serve_data_stream elimina y
/// envía Reject en lugar de Piece.
pub(crate) type CancelSet = Arc<Mutex<HashSet<(u16, u32, u32)>>>;

/// Slot que representa un stream de datos activo en el coordinador.
pub(crate) struct StreamSlot {
    pub(crate) task_tx: mpsc::Sender<BlockTask>,
    /// Tarea actualmente asignada, o None si idle.
    pub(crate) active_block: Option<BlockTask>,
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stream_result_block_timeout_matches_correctly() {
        let r = StreamResult::BlockTimeout {
            stream_id: 2,
            fi: 0,
            pi: 5,
            bi: 1,
        };
        match r {
            StreamResult::BlockTimeout {
                stream_id,
                fi,
                pi,
                bi,
            } => {
                assert_eq!(stream_id, 2);
                assert_eq!(fi, 0);
                assert_eq!(pi, 5);
                assert_eq!(bi, 1);
            }
            _ => panic!("expected BlockTimeout"),
        }
    }

    #[test]
    fn stream_result_block_timeout_is_not_stream_dead() {
        let timeout = StreamResult::BlockTimeout {
            stream_id: 0,
            fi: 0,
            pi: 0,
            bi: 0,
        };
        assert!(
            !matches!(timeout, StreamResult::StreamDead { .. }),
            "BlockTimeout no debe clasificarse como StreamDead"
        );
    }

    #[test]
    fn snub_timeout_is_at_least_30_seconds() {
        use crate::daemon::SNUB_TIMEOUT;
        assert!(
            SNUB_TIMEOUT.as_secs() >= 30,
            "SNUB_TIMEOUT debe ser al menos 30s para no snubbear peers lentos legítimos"
        );
    }

    #[test]
    fn snub_disconnect_threshold_is_at_least_two() {
        // El threshold debe ser > 1 para que el peer tenga al menos una segunda
        // oportunidad antes de ser desconectado (un único timeout puede ser
        // congestion transitoria).
        use super::super::super::drainer::SNUB_DISCONNECT_THRESHOLD;
        assert!(
            SNUB_DISCONNECT_THRESHOLD >= 2,
            "threshold demasiado agresivo: desconectaría tras un único timeout"
        );
    }
}
