use std::net::SocketAddr;
use quinn::Endpoint;
use tracing::{info, warn};

use crate::{
    config::{client_config_for_peer, server_config, TofuStore},
    connection::PeerConnection,
    error::{Result, TransportError},
};

pub struct QuicEndpoint {
    endpoint:   Endpoint,
    tofu_store: TofuStore,
}

impl QuicEndpoint {
    /// Crea un endpoint en modo servidor (escucha conexiones entrantes).
    /// También puede realizar conexiones salientes vía `connect()`.
    pub fn bind(bind_addr: SocketAddr) -> Result<Self> {
        let server_cfg = server_config()?;
        let endpoint = Endpoint::server(server_cfg, bind_addr)?;

        info!("QUIC endpoint bound to {}", bind_addr);
        Ok(Self { endpoint, tofu_store: TofuStore::new() })
    }

    /// Crea un endpoint en modo cliente únicamente (sin servidor).
    pub fn client_only() -> Result<Self> {
        let bind_addr = SocketAddr::from(([0, 0, 0, 0], 0));
        let endpoint = Endpoint::client(bind_addr)?;

        Ok(Self { endpoint, tofu_store: TofuStore::new() })
    }

    /// Conecta a un peer. Usa TOFU: la primera conexión almacena el
    /// fingerprint del cert; las siguientes verifican que coincide.
    pub async fn connect(&self, addr: SocketAddr) -> Result<PeerConnection> {
        let client_cfg = client_config_for_peer(addr, self.tofu_store.clone())?;
        let conn = self.endpoint
            .connect_with(client_cfg, addr, "bitturbulence")?
            .await?;

        info!("Connected to peer {}", addr);
        Ok(PeerConnection::new(conn))
    }

    pub async fn accept(&self) -> Option<Result<PeerConnection>> {
        let incoming = self.endpoint.accept().await?;
        let addr = incoming.remote_address();

        match incoming.await {
            Ok(conn) => {
                info!("Accepted connection from {}", addr);
                Some(Ok(PeerConnection::new(conn)))
            }
            Err(e) => {
                warn!("Failed to accept connection from {}: {}", addr, e);
                Some(Err(TransportError::Connection(e)))
            }
        }
    }

    pub fn local_addr(&self) -> Result<SocketAddr> {
        Ok(self.endpoint.local_addr()?)
    }

    /// Devuelve el almacén TOFU para inspección o compartición.
    pub fn tofu_store(&self) -> &TofuStore {
        &self.tofu_store
    }

    pub async fn shutdown(&self) {
        self.endpoint.close(quinn::VarInt::from_u32(0), b"shutdown");
        self.endpoint.wait_idle().await;
    }
}
