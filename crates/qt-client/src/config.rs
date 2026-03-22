use std::path::PathBuf;
use serde::{Deserialize, Serialize};
use anyhow::Result;

/// Configuración global del cliente.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    /// Directorio de descarga por defecto.
    pub download_dir: PathBuf,
    /// Puerto QUIC de escucha.
    pub listen_port:  u16,
    /// Trackers por defecto.
    pub trackers:     Vec<String>,
    /// Bootstrap nodes para la DHT.
    pub dht_bootstrap: Vec<String>,
    /// Token de autenticación para el tracker propio (opcional).
    pub tracker_auth_token: Option<String>,
    /// Ruta del fichero de estado.
    pub state_file:   PathBuf,
    /// Ruta de la tabla de routing DHT.
    pub dht_routing_table: PathBuf,
}

impl Default for Config {
    fn default() -> Self {
        let home = dirs_next::home_dir()
            .unwrap_or_else(|| PathBuf::from("."));
        let base = home.join(".quictorrent");
        Self {
            download_dir:      base.join("downloads"),
            listen_port:       6881,
            trackers:          vec![],
            dht_bootstrap:     vec![],
            tracker_auth_token: None,
            state_file:        base.join("state.json"),
            dht_routing_table: base.join("routing.json"),
        }
    }
}

impl Config {
    pub fn load(path: &std::path::Path) -> Result<Self> {
        if !path.exists() {
            return Ok(Self::default());
        }
        let json = std::fs::read_to_string(path)?;
        Ok(serde_json::from_str(&json)?)
    }

    pub fn save(&self, path: &std::path::Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let json = serde_json::to_string_pretty(self)?;
        std::fs::write(path, json)?;
        Ok(())
    }
}
