use thiserror::Error;

#[derive(Debug, Error)]
pub enum TrackerError {
    #[error("transport error: {0}")]
    Transport(#[from] bitturbulence_transport::TransportError),

    #[error("connection error: {0}")]
    Connection(#[from] quinn::ConnectionError),

    #[error("write error: {0}")]
    Write(#[from] quinn::WriteError),

    #[error("stream closed: {0}")]
    StreamClosed(#[from] quinn::ClosedStream),

    #[error("read error: {0}")]
    Read(#[from] quinn::ReadExactError),

    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("database error: {0}")]
    Db(#[from] rusqlite::Error),

    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    #[error("invalid address: {0}")]
    InvalidAddr(#[from] std::net::AddrParseError),

    #[error("invalid info hash: {0}")]
    InvalidInfoHash(String),

    #[error("authentication required")]
    AuthRequired,

    #[error("authentication failed")]
    AuthFailed,

    #[error("announce rejected: {0}")]
    Rejected(String),

    #[error("mutex poisoned (panic en sección crítica)")]
    LockPoisoned,
}

pub type Result<T> = std::result::Result<T, TrackerError>;
