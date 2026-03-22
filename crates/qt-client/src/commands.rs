use std::path::{Path, PathBuf};
use anyhow::{anyhow, Context, Result};
use hex;

use qt_dht::{DhtConfig, DhtNode};
use qt_protocol::Priority;
use qt_tracker::server::{ServerConfig, TrackerServer};
use qt_tracker::PeerStore;

use crate::config::Config;
use crate::state::{ClientState, DownloadState, TorrentEntry};

/// Añade un torrent al cliente.
pub async fn cmd_add(
    meta_path: &Path,
    save_path: Option<PathBuf>,
    priority: Priority,
    state_path: &Path,
    config: &Config,
) -> Result<()> {
    let save_path = save_path.unwrap_or_else(|| config.download_dir.clone());
    std::fs::create_dir_all(&save_path)?;

    // Leer el metainfo
    let meta_bytes = std::fs::read(meta_path)
        .with_context(|| format!("cannot read {:?}", meta_path))?;
    let meta: qt_protocol::Metainfo = serde_json::from_slice(&meta_bytes)
        .with_context(|| "invalid metainfo file")?;

    let info_hash_hex = hex::encode(meta.info_hash);
    let id = info_hash_hex[..8].to_string();

    let mut state = ClientState::load(state_path)?;

    if state.get(&id).is_some() {
        println!("torrent {} already added", id);
        return Ok(());
    }

    let total_size = meta.total_size();
    let entry = TorrentEntry {
        id:         id.clone(),
        info_hash:  info_hash_hex,
        name:       meta.name.clone(),
        save_path,
        state:      DownloadState::Queued,
        downloaded: 0,
        total_size,
        peers:      0,
    };

    state.add(entry);
    state.save(state_path)?;

    println!("added torrent [{}] {} ({} bytes, {} files)",
        id, meta.name, total_size, meta.files.len());

    Ok(())
}

/// Muestra el estado de todos los torrents.
pub fn cmd_status(state_path: &Path) -> Result<()> {
    let state = ClientState::load(state_path)?;

    if state.torrents.is_empty() {
        println!("no torrents");
        return Ok(());
    }

    println!("{:<8}  {:<30}  {:>7}  {:>6}  {:>5}  {}",
        "ID", "NAME", "SIZE", "PROG%", "PEERS", "STATE");
    println!("{}", "-".repeat(72));

    let mut entries: Vec<_> = state.torrents.values().collect();
    entries.sort_by(|a, b| a.id.cmp(&b.id));

    for e in entries {
        let size = format_size(e.total_size);
        println!("{:<8}  {:<30}  {:>7}  {:>5.1}%  {:>5}  {}",
            e.id,
            truncate(&e.name, 30),
            size,
            e.progress(),
            e.peers,
            e.state,
        );
    }

    Ok(())
}

/// Inicia la descarga de un torrent.
pub fn cmd_start(id: &str, state_path: &Path) -> Result<()> {
    let mut state = ClientState::load(state_path)?;
    let entry = state.get_mut(id)
        .ok_or_else(|| anyhow!("torrent '{}' not found", id))?;

    match &entry.state {
        DownloadState::Seeding => {
            println!("[{}] already seeding", id);
        }
        DownloadState::Downloading => {
            println!("[{}] already downloading", id);
        }
        _ => {
            entry.state = DownloadState::Downloading;
            println!("[{}] {} → downloading", id, entry.name);
        }
    }

    state.save(state_path)?;
    Ok(())
}

/// Pausa la descarga de un torrent.
pub fn cmd_pause(id: &str, state_path: &Path) -> Result<()> {
    let mut state = ClientState::load(state_path)?;
    let entry = state.get_mut(id)
        .ok_or_else(|| anyhow!("torrent '{}' not found", id))?;

    entry.state = DownloadState::Paused;
    println!("[{}] {} → paused", id, entry.name);
    state.save(state_path)?;
    Ok(())
}

/// Detiene y elimina un torrent de la lista.
pub fn cmd_stop(id: &str, state_path: &Path) -> Result<()> {
    let mut state = ClientState::load(state_path)?;
    let name = state.get(id)
        .ok_or_else(|| anyhow!("torrent '{}' not found", id))?
        .name.clone();

    state.torrents.retain(|k, v| k != id && !v.info_hash.starts_with(id));
    state.save(state_path)?;
    println!("[{}] {} removed", id, name);
    Ok(())
}

/// Muestra los peers de un torrent.
pub fn cmd_peers(id: &str, state_path: &Path) -> Result<()> {
    let state = ClientState::load(state_path)?;
    let entry = state.get(id)
        .ok_or_else(|| anyhow!("torrent '{}' not found", id))?;

    println!("torrent [{}] {} — {} peer(s) connected",
        entry.id, entry.name, entry.peers);

    // En producción aquí haríamos IPC con el daemon para obtener la lista real.
    if entry.peers == 0 {
        println!("  (no peers)");
    }

    Ok(())
}

/// Arranca el servidor tracker + DHT + endpoint QUIC.
pub async fn cmd_serve(config: &Config, state_path: &Path) -> Result<()> {
    println!("starting quictorrent server...");

    // DHT
    let dht_config = DhtConfig {
        bind_addr:           format!("0.0.0.0:{}", config.listen_port),
        bootstrap_nodes:     config.dht_bootstrap.clone(),
        routing_table_path:  Some(config.dht_routing_table.clone()),
    };
    let dht = DhtNode::new(dht_config)?;
    println!("  DHT node: {}", dht.local_id);

    // Tracker
    let db_path = config.state_file.parent()
        .unwrap_or(Path::new("."))
        .join("tracker.db");
    let store = PeerStore::open(Some(&db_path))?;

    let tracker_config = ServerConfig {
        bind_addr:         format!("0.0.0.0:{}", config.listen_port + 1)
            .parse()
            .unwrap(),
        require_auth:      config.tracker_auth_token.is_some(),
        auth_token:        config.tracker_auth_token.clone(),
        announce_interval: 1800,
        min_interval:      60,
    };

    println!("  tracker:  http://0.0.0.0:{}/announce", config.listen_port + 1);
    println!("  QUIC:     quic://0.0.0.0:{}", config.listen_port);
    println!("\npress Ctrl+C to stop");

    let tracker = TrackerServer::new(tracker_config, store);

    // Señal de cierre
    tokio::select! {
        result = tracker.run() => {
            result.map_err(|e| anyhow!("tracker error: {}", e))?;
        }
        _ = tokio::signal::ctrl_c() => {
            println!("\nshutting down...");
            dht.save_routing_table()
                .map_err(|e| anyhow!("failed to save routing table: {}", e))?;
            println!("routing table saved");
        }
    }

    Ok(())
}

// ── Helpers de formato ────────────────────────────────────────────────────────

fn format_size(bytes: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = 1024 * KB;
    const GB: u64 = 1024 * MB;
    const TB: u64 = 1024 * GB;
    match bytes {
        b if b >= TB => format!("{:.1}T", b as f64 / TB as f64),
        b if b >= GB => format!("{:.1}G", b as f64 / GB as f64),
        b if b >= MB => format!("{:.1}M", b as f64 / MB as f64),
        b if b >= KB => format!("{:.1}K", b as f64 / KB as f64),
        b            => format!("{}B", b),
    }
}

fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max { return s.to_string(); }
    format!("{}…", &s[..max - 1])
}
