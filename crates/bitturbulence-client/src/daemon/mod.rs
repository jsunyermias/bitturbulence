//! Daemon de descarga con multiplexación QUIC a nivel de bloque.
//!
//! ## Streams QUIC por conexión
//!
//! | Stream | Dirección | Contenido                                     |
//! |--------|-----------|-----------------------------------------------|
//! | 1      | ambos     | Hello / HelloAck (efímero)                    |
//! | 2      | control   | Have*, KeepAlive, Bye, HavePiece              |
//! | 3…N    | datos     | Request (1 bloque) / Piece / Reject por loop  |
//!
//! ## Módulos
//!
//! | Módulo          | Responsabilidad                                      |
//! |-----------------|------------------------------------------------------|
//! | `context`       | `TorrentCtx` — estado compartido de un BitFlow       |
//! | `stream`        | Workers QUIC, tipos de stream, helpers de bitmap     |
//! | `drainer`       | Bucle drainer (conexión saliente)                    |
//! | `filler`        | Bucle filler (conexión entrante)                     |
//! | `peer`          | Punto de entrada por peer                            |
//! | `accept`        | Accept loop QUIC + handshake inbound                 |
//! | `tracker`       | Tracker announce loop                                |
//! | `state_loop`    | Persistencia periódica del estado                    |

pub mod context;
pub mod stream;
pub mod drainer;
pub mod filler;
pub mod peer;
pub mod accept;
pub mod tracker;
pub mod state_loop;

pub use context::TorrentCtx;
pub use peer::run_peer;

use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::Arc;

use anyhow::{Context, Result};
use rand::RngCore;
use serde_json;
use tokio::sync::Mutex;
use tracing::{info, warn};

use bitturbulence_protocol::Metainfo;
use bitturbulence_transport::QuicEndpoint;

use crate::config::Config;
use crate::state::{ClientState, DownloadState};

// ── Constantes ────────────────────────────────────────────────────────────────

pub const HELLO_TIMEOUT:       std::time::Duration = std::time::Duration::from_secs(10);
pub const KEEPALIVE_INTERVAL:  std::time::Duration = std::time::Duration::from_secs(60);
pub const ANNOUNCE_INTERVAL:   std::time::Duration = std::time::Duration::from_secs(300);
pub const STATE_SAVE_INTERVAL: std::time::Duration = std::time::Duration::from_secs(30);
pub const PROTOCOL_VERSION:    u8                  = bitturbulence_protocol::PROTOCOL_VERSION;

/// Streams de datos abiertos al conectar a un peer.
pub const DEFAULT_STREAMS: usize = 2;

/// Máximo de streams de datos simultáneos por peer.
pub const MAX_STREAMS: usize = 4;

// ── Punto de entrada del daemon ───────────────────────────────────────────────

pub async fn run_daemon(config: &Config, state_path: &Path) -> Result<()> {
    let mut our_peer_id = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut our_peer_id);

    let bind_addr: std::net::SocketAddr = format!("0.0.0.0:{}", config.listen_port)
        .parse()
        .context("parsing listen addr")?;
    let endpoint = Arc::new(QuicEndpoint::bind(bind_addr)?);

    let state = ClientState::load(state_path)?;
    let mut torrents:    HashMap<[u8; 32], Arc<TorrentCtx>> = HashMap::new();
    let mut flow_ids:    Vec<(String, Arc<TorrentCtx>)>     = Vec::new();

    for entry in state.flows.values() {
        let active = matches!(entry.state, DownloadState::Downloading | DownloadState::Seeding);
        if !active { continue; }

        if entry.metainfo_path.as_os_str().is_empty() {
            warn!("[{}] no metainfo_path — re-add the flow", entry.id);
            continue;
        }

        let meta_bytes = match std::fs::read(&entry.metainfo_path) {
            Ok(b)  => b,
            Err(e) => { warn!("[{}] read metainfo: {e}", entry.id); continue; }
        };
        let meta: Metainfo = match serde_json::from_slice(&meta_bytes) {
            Ok(m)  => m,
            Err(e) => { warn!("[{}] parse metainfo: {e}", entry.id); continue; }
        };

        let seeding = entry.state == DownloadState::Seeding;
        let ctx = match TorrentCtx::new(meta, &entry.save_path, seeding).await {
            Ok(c)  => c,
            Err(e) => { warn!("[{}] init ctx: {e}", entry.id); continue; }
        };

        info!("[{}] {} — {}", entry.id, entry.name,
            if seeding { "seeding" } else { "downloading" });

        torrents.insert(ctx.meta.info_hash, ctx.clone());
        flow_ids.push((entry.id.clone(), ctx));
    }

    let torrents = Arc::new(torrents);

    for (_, ctx) in &flow_ids {
        let known_peers = Arc::new(Mutex::new(HashSet::<String>::new()));
        for tracker_url in &ctx.meta.trackers {
            tokio::spawn(tracker::tracker_loop(
                tracker_url.clone(),
                ctx.clone(),
                endpoint.clone(),
                our_peer_id,
                config.listen_port,
                known_peers.clone(),
            ));
        }
    }

    if !flow_ids.is_empty() {
        tokio::spawn(state_loop::state_save_loop(state_path.to_path_buf(), flow_ids));
    }

    accept::accept_loop(endpoint, torrents, our_peer_id).await;

    Ok(())
}
