use bitturbulence_protocol::BLOCK_SIZE;

/// Streams típicos por bloque (objetivo bajo carga normal).
pub const TYPICAL_STREAMS_PER_BLOCK: u8 = 2;

/// Máximo de streams simultáneos por bloque.
pub const MAX_STREAMS_PER_BLOCK: u8 = 4;

// ── Estado de un bloque ───────────────────────────────────────────────────────

/// Estado de un bloque individual (16 KiB).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum BlockState {
    /// No solicitado todavía.
    Pending,
    /// Siendo descargado por `n` streams simultáneos.
    InFlight(u8),
    /// Datos recibidos y escritos en disco.
    Done,
}

// ── Tarea de bloque ───────────────────────────────────────────────────────────

/// Tarea de descarga de un bloque individual.
#[derive(Debug, Clone)]
pub struct BlockTask {
    /// Índice del archivo en el BitFlow.
    pub fi: usize,
    /// Índice de la pieza.
    pub pi: u32,
    /// Índice del bloque dentro de la pieza (bi * BLOCK_SIZE = begin).
    pub bi: u32,
    /// Offset en bytes dentro de la pieza.
    pub begin: u32,
    /// Longitud del bloque (normalmente BLOCK_SIZE, el último puede ser menor).
    pub length: u32,
}

// ── Helpers de longitud ───────────────────────────────────────────────────────

/// Longitud real de una pieza dado su índice.
pub fn piece_len(pi: usize, num_pieces: usize, piece_length: u32, last_piece_len: u32) -> u32 {
    if pi + 1 == num_pieces { last_piece_len } else { piece_length }
}

/// Longitud real de un bloque dado su índice dentro de la pieza.
pub fn block_length(pi: usize, bi: usize, num_pieces: usize, piece_length: u32, last_piece_len: u32) -> u32 {
    let pl    = piece_len(pi, num_pieces, piece_length, last_piece_len);
    let begin = bi as u32 * BLOCK_SIZE;
    (pl - begin).min(BLOCK_SIZE)
}
