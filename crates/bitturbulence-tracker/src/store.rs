use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::{params, Connection};
use tracing::{debug, info};

use crate::error::{Result, TrackerError};
use crate::types::{AnnounceEvent, AnnounceRequest, PeerRecord};

/// TTL de un peer en segundos. Si no anuncia en este tiempo, se elimina.
const PEER_TTL_SECS: i64 = 3600;

/// Mapa interno: info_hash → peer_id → PeerRecord.
type PeerMap = HashMap<[u8; 32], HashMap<[u8; 32], PeerRecord>>;

/// Intervalo de flush a disco en segundos.
const FLUSH_INTERVAL_SECS: u64 = 300;

fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}

/// Store en memoria con persistencia a SQLite.
#[derive(Clone)]
pub struct PeerStore {
    inner: Arc<Mutex<PeerMap>>,
    db: Arc<Mutex<Connection>>,
}

impl PeerStore {
    /// Abre o crea el store. Si `db_path` es None, usa `:memory:`.
    pub fn open(db_path: Option<&Path>) -> Result<Self> {
        let conn = match db_path {
            Some(p) => Connection::open(p)?,
            None => Connection::open_in_memory()?,
        };

        conn.execute_batch(
            "
            CREATE TABLE IF NOT EXISTS peers (
                info_hash  BLOB NOT NULL,
                peer_id    BLOB NOT NULL,
                addr       TEXT NOT NULL,
                uploaded   INTEGER NOT NULL DEFAULT 0,
                downloaded INTEGER NOT NULL DEFAULT 0,
                left_bytes INTEGER NOT NULL DEFAULT 0,
                last_seen  INTEGER NOT NULL,
                completed  INTEGER NOT NULL DEFAULT 0,
                PRIMARY KEY (info_hash, peer_id)
            );
            CREATE INDEX IF NOT EXISTS idx_info_hash ON peers(info_hash);
        ",
        )?;

        let store = Self {
            inner: Arc::new(Mutex::new(HashMap::new())),
            db: Arc::new(Mutex::new(conn)),
        };

        store.load_from_db()?;
        Ok(store)
    }

    /// Carga todos los peers activos desde SQLite a memoria.
    fn load_from_db(&self) -> Result<()> {
        // Orden de adquisición: siempre inner primero, luego db (igual que flush).
        let mut inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        let db = self.db.lock().unwrap_or_else(|e| e.into_inner());
        let cutoff = now() - PEER_TTL_SECS;

        let mut stmt = db.prepare(
            "SELECT info_hash, peer_id, addr, uploaded, downloaded, left_bytes, last_seen, completed
             FROM peers WHERE last_seen > ?"
        )?;
        let rows = stmt.query_map(params![cutoff], |row| {
            let ih: Vec<u8> = row.get(0)?;
            let pid: Vec<u8> = row.get(1)?;
            Ok((
                ih,
                pid,
                row.get::<_, String>(2)?,
                row.get::<_, u64>(3)?,
                row.get::<_, u64>(4)?,
                row.get::<_, u64>(5)?,
                row.get::<_, i64>(6)?,
                row.get::<_, bool>(7)?,
            ))
        })?;

        for row in rows {
            let (ih, pid, addr, up, down, left, last_seen, completed) = row?;
            if ih.len() != 32 || pid.len() != 32 {
                continue;
            }
            let mut info_hash = [0u8; 32];
            let mut peer_id = [0u8; 32];
            info_hash.copy_from_slice(&ih);
            peer_id.copy_from_slice(&pid);

            let record = PeerRecord {
                info_hash,
                peer_id,
                addr,
                uploaded: up,
                downloaded: down,
                left,
                last_seen,
                completed,
            };
            inner.entry(info_hash).or_default().insert(peer_id, record);
        }

        info!("loaded {} torrents from db", inner.len());
        Ok(())
    }

    /// Procesa un announce y devuelve la lista de peers actualizada.
    pub fn announce(&self, req: &AnnounceRequest) -> Result<Vec<PeerRecord>> {
        let info_hash = parse_hex32(&req.info_hash)
            .map_err(|e| TrackerError::InvalidInfoHash(e.to_string()))?;
        let peer_id =
            parse_hex32(&req.peer_id).map_err(|e| TrackerError::InvalidInfoHash(e.to_string()))?;

        let mut inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());

        {
            let peers = inner.entry(info_hash).or_default();
            if req.event == AnnounceEvent::Stopped {
                peers.remove(&peer_id);
                debug!("peer removed: {}", &req.peer_id[..8]);
            } else {
                let record = PeerRecord {
                    info_hash,
                    peer_id,
                    addr: req.addr.clone(),
                    uploaded: req.uploaded,
                    downloaded: req.downloaded,
                    left: req.left,
                    last_seen: now(),
                    completed: req.event == AnnounceEvent::Completed || req.left == 0,
                };
                peers.insert(peer_id, record);
                debug!("peer upserted: {}", &req.peer_id[..8]);
            }

            // Purgar peers expirados.
            let cutoff = now() - PEER_TTL_SECS;
            peers.retain(|_, r| r.last_seen > cutoff);
        }
        // Limpiar entradas vacías: torrents abandonados no acumulan memoria.
        inner.retain(|_, peers| !peers.is_empty());

        // Devolver hasta num_want peers (excluyendo al solicitante)
        let result: Vec<PeerRecord> = inner
            .get(&info_hash)
            .map(|peers| {
                peers
                    .values()
                    .filter(|r| r.peer_id != peer_id)
                    .take(req.num_want as usize)
                    .cloned()
                    .collect()
            })
            .unwrap_or_default();

        Ok(result)
    }

    /// Estadísticas de un torrent (fillers, drainers, completados históricos).
    pub fn scrape(&self, info_hash: &[u8; 32]) -> (u32, u32, u32) {
        let inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        let peers = match inner.get(info_hash) {
            None => return (0, 0, 0),
            Some(p) => p,
        };
        let cutoff = now() - PEER_TTL_SECS;
        let mut fillers = 0u32;
        let mut drainers = 0u32;
        let mut completed = 0u32;
        for r in peers.values() {
            if r.last_seen <= cutoff {
                continue;
            }
            if r.is_filler() {
                fillers += 1;
            } else {
                drainers += 1;
            }
            if r.completed {
                completed += 1;
            }
        }
        (fillers, drainers, completed)
    }

    /// Persiste el estado actual a SQLite.
    ///
    /// El lock de `inner` se libera antes de escribir a disco para no bloquear
    /// announce/scrape durante la escritura.
    pub fn flush(&self) -> Result<()> {
        // Orden de adquisición: inner primero (coherente con el resto).
        // Clonar los registros para liberar el lock antes de tocar SQLite.
        let snapshot: Vec<PeerRecord> = {
            let inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
            inner.values().flat_map(|peers| peers.values().cloned()).collect()
        };
        let num_torrents = {
            let inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
            inner.len()
        };

        let db = self.db.lock().unwrap_or_else(|e| e.into_inner());
        let tx = db.unchecked_transaction()?;
        tx.execute("DELETE FROM peers", [])?;
        for r in &snapshot {
            tx.execute(
                "INSERT OR REPLACE INTO peers
                 (info_hash, peer_id, addr, uploaded, downloaded, left_bytes, last_seen, completed)
                 VALUES (?1,?2,?3,?4,?5,?6,?7,?8)",
                params![
                    r.info_hash.as_slice(), r.peer_id.as_slice(),
                    r.addr, r.uploaded, r.downloaded, r.left,
                    r.last_seen, r.completed as i32,
                ],
            )?;
        }
        tx.commit()?;
        debug!("flushed {} torrents to db", num_torrents);
        Ok(())
    }

    /// Inicia un task de flush periódico.
    pub fn start_flush_task(&self) {
        let store = self.clone();
        tokio::spawn(async move {
            let mut interval =
                tokio::time::interval(std::time::Duration::from_secs(FLUSH_INTERVAL_SECS));
            loop {
                interval.tick().await;
                if let Err(e) = store.flush() {
                    tracing::error!("flush error: {}", e);
                }
            }
        });
    }
}

fn parse_hex32(s: &str) -> std::result::Result<[u8; 32], String> {
    if s.len() != 64 {
        return Err(format!("expected 64 hex chars, got {}", s.len()));
    }
    let bytes = hex::decode(s).map_err(|e| e.to_string())?;
    let mut arr = [0u8; 32];
    arr.copy_from_slice(&bytes);
    Ok(arr)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::AnnounceEvent;

    fn make_req(peer_id: &str, event: AnnounceEvent, left: u64) -> AnnounceRequest {
        AnnounceRequest {
            info_hash: "ab".repeat(32),
            peer_id: peer_id.to_string(),
            addr: "127.0.0.1:6881".into(),
            event,
            uploaded: 0,
            downloaded: 0,
            left,
            num_want: 50,
        }
    }

    #[test]
    fn announce_and_scrape() {
        let store = PeerStore::open(None).unwrap();
        let ih = parse_hex32(&"ab".repeat(32)).unwrap();

        store
            .announce(&make_req(&"aa".repeat(32), AnnounceEvent::Started, 1000))
            .unwrap();
        store
            .announce(&make_req(&"bb".repeat(32), AnnounceEvent::Started, 0))
            .unwrap();

        let (fillers, drainers, _) = store.scrape(&ih);
        assert_eq!(fillers, 1);
        assert_eq!(drainers, 1);
    }

    #[test]
    fn stopped_removes_peer() {
        let store = PeerStore::open(None).unwrap();
        let ih = parse_hex32(&"ab".repeat(32)).unwrap();

        store
            .announce(&make_req(&"aa".repeat(32), AnnounceEvent::Started, 100))
            .unwrap();
        store
            .announce(&make_req(&"aa".repeat(32), AnnounceEvent::Stopped, 100))
            .unwrap();

        let (s, l, _) = store.scrape(&ih);
        assert_eq!(s + l, 0);
    }

    #[test]
    fn peers_exclude_self() {
        let store = PeerStore::open(None).unwrap();
        let pid = "aa".repeat(32);

        store
            .announce(&make_req(&pid, AnnounceEvent::Started, 100))
            .unwrap();
        store
            .announce(&make_req(&"bb".repeat(32), AnnounceEvent::Started, 100))
            .unwrap();

        let peers = store
            .announce(&make_req(&pid, AnnounceEvent::Update, 100))
            .unwrap();
        assert!(peers
            .iter()
            .all(|p| p.peer_id != parse_hex32(&pid).unwrap()));
    }

    #[test]
    fn flush_and_reload() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("tracker.db");

        {
            let store = PeerStore::open(Some(&db_path)).unwrap();
            store
                .announce(&make_req(&"aa".repeat(32), AnnounceEvent::Started, 500))
                .unwrap();
            store.flush().unwrap();
        }

        let store2 = PeerStore::open(Some(&db_path)).unwrap();
        let ih = parse_hex32(&"ab".repeat(32)).unwrap();
        let (_, drainers, _) = store2.scrape(&ih);
        assert_eq!(drainers, 1);
    }
}
