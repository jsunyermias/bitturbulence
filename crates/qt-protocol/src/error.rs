use thiserror::Error;

#[derive(Debug, Error)]
pub enum ProtocolError {
    #[error("unknown message id: {0}")]
    UnknownMessageId(u8),

    #[error("invalid message length: expected at least {expected}, got {got}")]
    InvalidMessageLength { expected: usize, got: usize },

    #[error("frame too large: {0} bytes")]
    FrameTooLarge(usize),

    #[error("invalid info hash length: expected 32, got {0}")]
    InvalidInfoHashLength(usize),

    #[error("invalid priority value: {0}")]
    InvalidPriority(u8),

    #[error("invalid auth payload tag: {0}")]
    InvalidAuthTag(u8),

    #[error("utf8 error: {0}")]
    Utf8(#[from] std::string::FromUtf8Error),

    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
}

pub type Result<T> = std::result::Result<T, ProtocolError>;
