use thiserror::Error;

#[derive(Debug, Error)]
pub enum PeerError {
    #[error("transport error: {0}")]
    Transport(#[from] qt_transport::TransportError),

    #[error("protocol error: {0}")]
    Protocol(#[from] qt_protocol::ProtocolError),

    #[error("hello rejected: {0}")]
    HelloRejected(String),

    #[error("info hash mismatch")]
    InfoHashMismatch,

    #[error("unexpected message")]
    UnexpectedMessage,

    #[error("peer disconnected")]
    Disconnected,

    #[error("session timeout")]
    Timeout,

    #[error("quinn write error: {0}")]
    Write(#[from] quinn::WriteError),

    #[error("quinn read error: {0}")]
    Read(#[from] quinn::ReadExactError),

    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
}

pub type Result<T> = std::result::Result<T, PeerError>;
