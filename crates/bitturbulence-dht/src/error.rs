use thiserror::Error;

#[derive(Debug, Error)]
pub enum DhtError {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("transport error: {0}")]
    Transport(#[from] bitturbulence_transport::TransportError),

    #[error("invalid node id length: expected 32, got {0}")]
    InvalidNodeId(usize),

    #[error("invalid token")]
    InvalidToken,

    #[error("timeout")]
    Timeout,

    #[error("no nodes available")]
    NoNodes,

    #[error("mutex poisoned (panic en sección crítica)")]
    LockPoisoned,
}

pub type Result<T> = std::result::Result<T, DhtError>;
