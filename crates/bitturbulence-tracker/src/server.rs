use std::net::SocketAddr;
use std::sync::Arc;

use quinn::{RecvStream, SendStream};
use tracing::{info, warn};

use bitturbulence_transport::QuicEndpoint;

use crate::{
    error::Result,
    store::PeerStore,
    types::{AnnounceResponse, PeerInfo, TrackerMessage, TrackerResponse},
};

/// Configuración del servidor tracker.
#[derive(Debug, Clone)]
pub struct ServerConfig {
    /// Dirección de escucha QUIC.
    pub bind_addr: SocketAddr,
    /// Si true, los clientes deben incluir el token en la petición.
    pub require_auth: bool,
    /// Token válido (si require_auth = true).
    pub auth_token: Option<String>,
    /// Intervalo sugerido a los clientes (segundos).
    pub announce_interval: u32,
    /// Intervalo mínimo (segundos).
    pub min_interval: u32,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            bind_addr:         "0.0.0.0:6969".parse().expect("valid default bind addr"),
            require_auth:      false,
            auth_token:        None,
            announce_interval: 1800,
            min_interval:      60,
        }
    }
}

/// Servidor tracker BitTurbulence sobre QUIC.
pub struct TrackerServer {
    config: ServerConfig,
    store:  PeerStore,
}

impl TrackerServer {
    pub fn new(config: ServerConfig, store: PeerStore) -> Self {
        Self { config, store }
    }

    pub async fn run(self) -> Result<()> {
        let endpoint = QuicEndpoint::bind(self.config.bind_addr)?;
        self.store.start_flush_task();
        info!("tracker QUIC escuchando en {}", self.config.bind_addr);

        let config = Arc::new(self.config);
        loop {
            if let Some(conn_result) = endpoint.accept().await {
                match conn_result {
                    Ok(conn) => {
                        let store  = self.store.clone();
                        let config = config.clone();
                        tokio::spawn(async move {
                            let qconn = conn.inner_conn().clone();
                            loop {
                                match qconn.accept_bi().await {
                                    Ok((send, recv)) => {
                                        let store  = store.clone();
                                        let config = config.clone();
                                        tokio::spawn(handle_request(send, recv, store, config));
                                    }
                                    Err(_) => break,
                                }
                            }
                        });
                    }
                    Err(e) => warn!("tracker: conexión entrante fallida: {e}"),
                }
            }
        }
    }
}

async fn handle_request(
    mut send:  SendStream,
    mut recv:  RecvStream,
    store:     PeerStore,
    config:    Arc<ServerConfig>,
) {
    if let Err(e) = process_request(&mut send, &mut recv, &store, &config).await {
        warn!("tracker: error procesando petición: {e}");
    }
}

async fn process_request(
    send:   &mut SendStream,
    recv:   &mut RecvStream,
    store:  &PeerStore,
    config: &ServerConfig,
) -> Result<()> {
    let msg: TrackerMessage = read_frame(recv).await?;

    let response = match msg {
        TrackerMessage::Announce { auth_token, req } => {
            if !check_auth(auth_token.as_deref(), config) {
                TrackerResponse::Error { message: "autenticación requerida".into() }
            } else {
                match store.announce(&req) {
                    Err(e) => TrackerResponse::Error { message: e.to_string() },
                    Ok(peers) => {
                        let ih = hex::decode(&req.info_hash)
                            .ok()
                            .and_then(|b| b.try_into().ok())
                            .unwrap_or([0u8; 32]);
                        let (complete, incomplete, _) = store.scrape(&ih);
                        let peer_list = peers.into_iter().map(|r| PeerInfo {
                            peer_id: hex::encode(r.peer_id),
                            addr:    r.addr,
                        }).collect();
                        TrackerResponse::Announce(AnnounceResponse {
                            interval:     config.announce_interval,
                            min_interval: Some(config.min_interval),
                            peers:        peer_list,
                            complete,
                            incomplete,
                        })
                    }
                }
            }
        }

        TrackerMessage::Scrape { auth_token, info_hash } => {
            if !check_auth(auth_token.as_deref(), config) {
                TrackerResponse::Error { message: "autenticación requerida".into() }
            } else {
                match hex::decode(&info_hash).ok().and_then(|b| b.try_into().ok()) {
                    None => TrackerResponse::Error { message: "info_hash inválido".into() },
                    Some(ih) => {
                        let (complete, incomplete, downloaded) = store.scrape(&ih);
                        TrackerResponse::Scrape(crate::types::ScrapeResponse {
                            complete, incomplete, downloaded,
                        })
                    }
                }
            }
        }
    };

    write_frame(send, &response).await
}

fn check_auth(token: Option<&str>, config: &ServerConfig) -> bool {
    if !config.require_auth { return true; }
    match (&config.auth_token, token) {
        (Some(expected), Some(provided)) => expected == provided,
        _ => false,
    }
}

// ── Framing: 4 bytes BE longitud + payload JSON ───────────────────────────────

async fn read_frame(recv: &mut RecvStream) -> Result<TrackerMessage> {
    let mut len_buf = [0u8; 4];
    recv.read_exact(&mut len_buf).await?;
    let len = u32::from_be_bytes(len_buf) as usize;
    let mut buf = vec![0u8; len];
    recv.read_exact(&mut buf).await?;
    Ok(serde_json::from_slice(&buf)?)
}

async fn write_frame(send: &mut SendStream, resp: &TrackerResponse) -> Result<()> {
    let json = serde_json::to_vec(resp)?;
    let len  = (json.len() as u32).to_be_bytes();
    send.write_all(&len).await?;
    send.write_all(&json).await?;
    Ok(())
}
