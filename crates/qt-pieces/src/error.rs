use thiserror::Error;

#[derive(Debug, Error)]
pub enum PiecesError {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    #[error("piece {0} hash mismatch")]
    HashMismatch(u32),

    #[error("piece {index} out of range (total: {total})")]
    OutOfRange { index: u32, total: u32 },

    #[error("block out of bounds: piece {piece} offset {begin}+{length} > {piece_len}")]
    BlockOutOfBounds { piece: u32, begin: u32, length: u32, piece_len: u64 },
}

pub type Result<T> = std::result::Result<T, PiecesError>;
