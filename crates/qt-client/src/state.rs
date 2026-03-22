use std::collections::HashMap;
use std::path::{Path, PathBuf};
use serde::{Deserialize, Serialize};
use anyhow::Result;

/// Estado de una descarga.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DownloadState {
    Queued,
    Downloading,
    Paused,
    Seeding,
    Error(String),
}

impl std::fmt::Display for DownloadState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Queued      => write!(f, "queued"),
            Self::Downloading => write!(f, "downloading"),
            Self::Paused      => write!(f, "paused"),
            Self::Seeding     => write!(f, "seeding"),
            Self::Error(e)    => write!(f, "error: {}", e),
        }
    }
}

/// Entrada en la lista de torrents gestionados.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TorrentEntry {
    /// ID corto (primeros 8 hex del info_hash).
    pub id:          String,
    /// Info hash completo (64 hex).
    pub info_hash:   String,
    /// Nombre del torrent.
    pub name:        String,
    /// Directorio de descarga.
    pub save_path:   PathBuf,
    /// Estado actual.
    pub state:       DownloadState,
    /// Bytes descargados.
    pub downloaded:  u64,
    /// Tamaño total.
    pub total_size:  u64,
    /// Número de peers conectados.
    pub peers:       usize,
}

impl TorrentEntry {
    pub fn progress(&self) -> f32 {
        if self.total_size == 0 { return 0.0; }
        self.downloaded as f32 / self.total_size as f32 * 100.0
    }
}

/// Estado persistente del cliente.
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct ClientState {
    pub torrents: HashMap<String, TorrentEntry>,
}

impl ClientState {
    pub fn load(path: &Path) -> Result<Self> {
        if !path.exists() {
            return Ok(Self::default());
        }
        let json = std::fs::read_to_string(path)?;
        Ok(serde_json::from_str(&json)?)
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let json = serde_json::to_string_pretty(self)?;
        std::fs::write(path, json)?;
        Ok(())
    }

    pub fn add(&mut self, entry: TorrentEntry) {
        self.torrents.insert(entry.id.clone(), entry);
    }

    pub fn get(&self, id: &str) -> Option<&TorrentEntry> {
        // Buscar por ID corto o por prefijo del info_hash
        self.torrents.get(id)
            .or_else(|| self.torrents.values().find(|e| e.info_hash.starts_with(id)))
    }

    pub fn get_mut(&mut self, id: &str) -> Option<&mut TorrentEntry> {
        if self.torrents.contains_key(id) {
            return self.torrents.get_mut(id);
        }
        let key = self.torrents.keys()
            .find(|k| self.torrents[*k].info_hash.starts_with(id))
            .cloned();
        key.and_then(|k| self.torrents.get_mut(&k))
    }
}
