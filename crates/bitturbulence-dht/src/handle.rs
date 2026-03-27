use std::collections::{HashMap, HashSet};
use std::net::SocketAddr;
use std::time::Duration;

use tokio::net::UdpSocket;
use tokio::sync::{broadcast, mpsc};
use tracing::{debug, warn};

use crate::{
    error::Result,
    message::DhtMessage,
    node::DhtNode,
    node_id::NodeId,
};

const DHT_QUERY_TIMEOUT: Duration = Duration::from_millis(500);

/// Handle clonable para interactuar con el [`DhtNode`] en ejecución.
#[derive(Clone)]
pub struct DhtHandle {
    pub(crate) node:        std::sync::Arc<DhtNode>,
    pub(crate) send_tx:     mpsc::Sender<(String, DhtMessage)>,
    pub(crate) incoming_tx: broadcast::Sender<(String, DhtMessage)>,
    local_addr:             SocketAddr,
}

impl DhtHandle {
    pub fn local_addr(&self) -> SocketAddr { self.local_addr }
    pub fn local_id(&self)   -> NodeId     { self.node.local_id }

    /// Ping a todos los `bootstrap_nodes` configurados y espera Pongs.
    pub async fn bootstrap(&self) {
        for addr in &self.node.config.bootstrap_nodes {
            let msg = DhtMessage::Ping { sender: self.node.local_id };
            let _ = self.send_tx.send((addr.clone(), msg)).await;
        }
        if !self.node.config.bootstrap_nodes.is_empty() {
            tokio::time::sleep(Duration::from_millis(300)).await;
        }
    }

    /// Consulta peers para un `info_hash` a los K nodos más cercanos conocidos.
    /// Devuelve las direcciones de peers recopiladas en `DHT_QUERY_TIMEOUT`.
    pub async fn get_peers(&self, info_hash: [u8; 32]) -> Vec<String> {
        let target  = NodeId::from_bytes(&info_hash).unwrap_or_else(|_| NodeId::random());
        let closest = self.node.closest_nodes(&target);
        if closest.is_empty() { return vec![]; }

        let mut rx = self.incoming_tx.subscribe();
        for n in &closest {
            let msg = DhtMessage::GetPeers { sender: self.node.local_id, info_hash };
            let _ = self.send_tx.send((n.addr.clone(), msg)).await;
        }

        let mut peers: HashSet<String> = HashSet::new();
        let deadline = tokio::time::Instant::now() + DHT_QUERY_TIMEOUT;
        loop {
            match tokio::time::timeout_at(deadline, rx.recv()).await {
                Ok(Ok((_, DhtMessage::GotPeers { peers: p, nodes: n, .. }))) => {
                    for peer in p { peers.insert(peer); }
                    for node in n { self.node.add_node(node.id, node.addr.clone()); }
                }
                _ => break,
            }
        }
        peers.into_iter().collect()
    }

    /// Anuncia nuestra presencia para un `info_hash`:
    /// 1. `GetPeers` a los nodos más cercanos para obtener tokens
    /// 2. `AnnouncePeer` con cada token recibido
    pub async fn announce(&self, info_hash: [u8; 32], port: u16) {
        let target  = NodeId::from_bytes(&info_hash).unwrap_or_else(|_| NodeId::random());
        let closest = self.node.closest_nodes(&target);
        if closest.is_empty() { return; }

        let mut rx = self.incoming_tx.subscribe();
        for n in &closest {
            let msg = DhtMessage::GetPeers { sender: self.node.local_id, info_hash };
            let _ = self.send_tx.send((n.addr.clone(), msg)).await;
        }

        let mut tokens: HashMap<String, [u8; 8]> = HashMap::new();
        let deadline = tokio::time::Instant::now() + DHT_QUERY_TIMEOUT;
        loop {
            match tokio::time::timeout_at(deadline, rx.recv()).await {
                Ok(Ok((addr, DhtMessage::GotPeers { token, nodes: n, .. }))) => {
                    tokens.insert(addr, token);
                    for node in n { self.node.add_node(node.id, node.addr.clone()); }
                }
                _ => break,
            }
        }

        let our_addr = format!("0.0.0.0:{port}");
        for (addr, token) in tokens {
            let msg = DhtMessage::AnnouncePeer {
                sender: self.node.local_id,
                info_hash,
                token,
                addr: our_addr.clone(),
            };
            let _ = self.send_tx.send((addr, msg)).await;
        }
    }

    /// Guarda la tabla de routing en disco si está configurada la ruta.
    pub fn save_routing_table(&self) -> Result<()> {
        self.node.save_routing_table()
    }
}

// ── Networking ────────────────────────────────────────────────────────────────

impl DhtNode {
    /// Enlaza el socket UDP y arranca el loop de mensajes DHT.
    /// Devuelve un [`DhtHandle`] para interactuar con el nodo.
    pub async fn bind(self) -> Result<DhtHandle> {
        let socket     = UdpSocket::bind(&self.config.bind_addr).await?;
        let local_addr = socket.local_addr()?;
        let (send_tx, send_rx)   = mpsc::channel::<(String, DhtMessage)>(256);
        let (incoming_tx, _)     = broadcast::channel::<(String, DhtMessage)>(256);
        let node = std::sync::Arc::new(self);
        let handle = DhtHandle {
            node: node.clone(),
            send_tx,
            incoming_tx: incoming_tx.clone(),
            local_addr,
        };
        tokio::spawn(udp_loop(socket, node, send_rx, incoming_tx));
        Ok(handle)
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{message::DhtMessage, node::{DhtConfig, DhtNode}, node_id::NodeId};

    fn make_config() -> DhtConfig {
        DhtConfig { bind_addr: "127.0.0.1:0".into(), bootstrap_nodes: vec![], routing_table_path: None }
    }

    #[tokio::test]
    async fn bind_and_ping() {
        let handle = DhtNode::new(make_config()).unwrap().bind().await.unwrap();
        let _ = handle.local_id();
        let _ = handle.local_addr();
    }

    #[tokio::test]
    async fn get_peers_returns_empty_when_no_nodes() {
        let handle = DhtNode::new(make_config()).unwrap().bind().await.unwrap();
        let peers  = handle.get_peers([0xABu8; 32]).await;
        assert!(peers.is_empty());
    }

    #[tokio::test]
    async fn two_nodes_discover_peers() {
        let info_hash = [0xABu8; 32];

        let node_a = DhtNode::new(make_config()).unwrap();
        {
            let fake_id = NodeId::random();
            let get     = DhtMessage::GetPeers { sender: fake_id, info_hash };
            let resp    = node_a.handle_message(get, "127.0.0.1:9999").unwrap();
            let token   = match resp { DhtMessage::GotPeers { token, .. } => token, _ => panic!() };
            let ann     = DhtMessage::AnnouncePeer {
                sender: fake_id, info_hash, token, addr: "10.0.0.1:6881".into(),
            };
            node_a.handle_message(ann, "127.0.0.1:9999");
        }
        let handle_a = node_a.bind().await.unwrap();
        let addr_a   = handle_a.local_addr().to_string();

        let node_b = DhtNode::new(make_config()).unwrap();
        node_b.add_node(handle_a.local_id(), addr_a);
        let handle_b = node_b.bind().await.unwrap();

        let peers = handle_b.get_peers(info_hash).await;
        assert!(peers.contains(&"10.0.0.1:6881".to_string()));
    }
}

async fn udp_loop(
    socket:      UdpSocket,
    node:        std::sync::Arc<DhtNode>,
    mut send_rx: mpsc::Receiver<(String, DhtMessage)>,
    incoming_tx: broadcast::Sender<(String, DhtMessage)>,
) {
    let mut buf = vec![0u8; 65535];
    loop {
        tokio::select! {
            result = socket.recv_from(&mut buf) => {
                let (len, from) = match result {
                    Ok(x)  => x,
                    Err(e) => { warn!("dht recv: {e}"); break; }
                };
                let bytes = bytes::Bytes::copy_from_slice(&buf[..len]);
                let msg = match DhtMessage::decode(bytes) {
                    Ok(m)  => m,
                    Err(e) => { debug!("dht decode: {e}"); continue; }
                };
                let addr = from.to_string();
                if let Some(resp) = node.handle_message(msg.clone(), &addr) {
                    let _ = socket.send_to(&resp.encode(), from).await;
                }
                let _ = incoming_tx.send((addr, msg));
            }
            Some((addr, msg)) = send_rx.recv() => {
                if let Err(e) = socket.send_to(&msg.encode(), &addr).await {
                    debug!("dht send to {addr}: {e}");
                }
            }
        }
    }
}
