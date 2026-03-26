use std::net::SocketAddr;

use tracing::debug;

use bitturbulence_transport::QuicEndpoint;

use crate::{
    error::Result,
    types::{AnnounceRequest, AnnounceResponse, ScrapeResponse, TrackerMessage, TrackerResponse},
    TrackerError,
};

/// Cliente QUIC para un tracker BitTurbulence.
pub struct TrackerClient {
    addr:       SocketAddr,
    auth_token: Option<String>,
}

impl TrackerClient {
    /// Crea un cliente apuntando a la dirección QUIC del tracker (ej. "127.0.0.1:6969").
    pub fn new(addr: impl AsRef<str>, auth_token: Option<String>) -> Result<Self> {
        let addr = addr.as_ref().parse::<SocketAddr>()?;
        Ok(Self { addr, auth_token })
    }

    /// Envía un announce al tracker y devuelve la lista de peers.
    pub async fn announce(&self, req: AnnounceRequest) -> Result<AnnounceResponse> {
        let msg = TrackerMessage::Announce {
            auth_token: self.auth_token.clone(),
            req,
        };
        debug!("announce a {}", self.addr);
        match self.send(msg).await? {
            TrackerResponse::Announce(r) => Ok(r),
            TrackerResponse::Error { message } => Err(TrackerError::Rejected(message)),
            _ => Err(TrackerError::Rejected("respuesta inesperada".into())),
        }
    }

    /// Obtiene estadísticas de un torrent desde el tracker.
    pub async fn scrape(&self, info_hash: &[u8; 32]) -> Result<ScrapeResponse> {
        let msg = TrackerMessage::Scrape {
            auth_token: self.auth_token.clone(),
            info_hash:  hex::encode(info_hash),
        };
        debug!("scrape a {}", self.addr);
        match self.send(msg).await? {
            TrackerResponse::Scrape(r) => Ok(r),
            TrackerResponse::Error { message } => Err(TrackerError::Rejected(message)),
            _ => Err(TrackerError::Rejected("respuesta inesperada".into())),
        }
    }

    async fn send(&self, msg: TrackerMessage) -> Result<TrackerResponse> {
        let endpoint = QuicEndpoint::client_only()?;
        let conn = endpoint.connect(self.addr).await?;
        let (mut send, mut recv) = conn.inner_conn().open_bi().await?;

        // Enviar petición
        let json = serde_json::to_vec(&msg)?;
        send.write_all(&(json.len() as u32).to_be_bytes()).await?;
        send.write_all(&json).await?;
        send.finish()?;

        // Recibir respuesta
        let mut len_buf = [0u8; 4];
        recv.read_exact(&mut len_buf).await?;
        let len = u32::from_be_bytes(len_buf) as usize;
        let mut buf = vec![0u8; len];
        recv.read_exact(&mut buf).await?;

        Ok(serde_json::from_slice(&buf)?)
    }
}
