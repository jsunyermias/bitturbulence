use serde::{Deserialize, Serialize};
use hex;

/// Mensaje que el cliente envía al tracker sobre QUIC.
#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum TrackerMessage {
    Announce {
        #[serde(skip_serializing_if = "Option::is_none")]
        auth_token: Option<String>,
        #[serde(flatten)]
        req: AnnounceRequest,
    },
    Scrape {
        #[serde(skip_serializing_if = "Option::is_none")]
        auth_token: Option<String>,
        info_hash: String,
    },
}

/// Respuesta del tracker al cliente.
#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum TrackerResponse {
    Announce(AnnounceResponse),
    Scrape(ScrapeResponse),
    Error { message: String },
}

/// Evento de announce.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AnnounceEvent {
    Started,
    Stopped,
    Completed,
    #[default]
    Update,
}

/// Request de announce enviada por un cliente al tracker.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnnounceRequest {
    /// Info hash SHA-256 del torrent (hex, 64 chars).
    pub info_hash:  String,
    /// Peer ID del cliente (hex, 64 chars).
    pub peer_id:    String,
    /// Dirección pública del peer (ip:port).
    pub addr:       String,
    /// Evento de ciclo de vida.
    #[serde(default)]
    pub event:      AnnounceEvent,
    /// Bytes subidos desde el último announce.
    #[serde(default)]
    pub uploaded:   u64,
    /// Bytes descargados desde el último announce.
    #[serde(default)]
    pub downloaded: u64,
    /// Bytes que faltan para completar.
    #[serde(default)]
    pub left:       u64,
    /// Número máximo de peers que quiere recibir.
    #[serde(default = "default_num_want")]
    pub num_want:   u32,
}

fn default_num_want() -> u32 { 50 }

impl AnnounceRequest {
    /// Constructor ergonómico: acepta hashes en bytes y se encarga de la codificación hex.
    pub fn new(
        info_hash:  &[u8; 32],
        peer_id:    &[u8; 32],
        addr:       impl Into<String>,
        event:      AnnounceEvent,
        uploaded:   u64,
        downloaded: u64,
        left:       u64,
    ) -> Self {
        Self {
            info_hash:  hex::encode(info_hash),
            peer_id:    hex::encode(peer_id),
            addr:       addr.into(),
            event,
            uploaded,
            downloaded,
            left,
            num_want:   50,
        }
    }

    /// Sobreescribe el número de peers pedidos (por defecto 50).
    pub fn with_num_want(mut self, num_want: u32) -> Self {
        self.num_want = num_want;
        self
    }
}

/// Peer tal como lo devuelve el tracker.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PeerInfo {
    pub peer_id: String,
    pub addr:    String,
}

/// Respuesta del tracker a un announce.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnnounceResponse {
    /// Segundos hasta el próximo announce obligatorio.
    pub interval:   u32,
    /// Segundos mínimos entre announces (opcional).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub min_interval: Option<u32>,
    /// Lista de peers para este torrent.
    pub peers:      Vec<PeerInfo>,
    /// Número de fillers (tienen el archivo completo).
    pub complete:   u32,
    /// Número de drainers (descargando).
    pub incomplete: u32,
}

/// Respuesta del tracker a un scrape.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScrapeResponse {
    pub complete:   u32,
    pub incomplete: u32,
    pub downloaded: u32,
}

/// Registro interno de un peer en el store.
#[derive(Debug, Clone)]
pub struct PeerRecord {
    pub info_hash:  [u8; 32],
    pub peer_id:    [u8; 32],
    pub addr:       String,
    pub uploaded:   u64,
    pub downloaded: u64,
    pub left:       u64,
    pub last_seen:  i64,   // Unix timestamp
    pub completed:  bool,
}

impl PeerRecord {
    pub fn is_filler(&self) -> bool {
        self.left == 0 || self.completed
    }
}
