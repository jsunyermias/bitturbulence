use std::time::Instant;
use serde::{Deserialize, Serialize};
use crate::node_id::NodeId;

/// Número máximo de nodos por bucket (parámetro k de Kademlia).
pub const K: usize = 20;

/// Información de un nodo en la tabla de routing.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeInfo {
    pub id:        NodeId,
    pub addr:      String,
    #[serde(skip)]
    pub last_seen: Option<Instant>,
}

impl NodeInfo {
    pub fn new(id: NodeId, addr: String) -> Self {
        Self { id, addr, last_seen: Some(Instant::now()) }
    }

    pub fn touch(&mut self) {
        self.last_seen = Some(Instant::now());
    }
}

impl PartialEq for NodeInfo {
    fn eq(&self, other: &Self) -> bool { self.id == other.id }
}

/// Un k-bucket: lista de hasta K nodos ordenados por antigüedad (LRU).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct KBucket {
    nodes: Vec<NodeInfo>,
}

impl KBucket {
    pub fn new() -> Self { Self { nodes: Vec::new() } }

    /// Añade o actualiza un nodo. Si el bucket está lleno, descarta el
    /// candidato (el nodo más antiguo se mantiene — política Kademlia).
    pub fn upsert(&mut self, node: NodeInfo) -> bool {
        if let Some(idx) = self.nodes.iter().position(|n| n.id == node.id) {
            self.nodes[idx].touch();
            // Mover al final (más reciente)
            let n = self.nodes.remove(idx);
            self.nodes.push(n);
            return true;
        }
        if self.nodes.len() < K {
            self.nodes.push(node);
            return true;
        }
        // Bucket lleno — descartamos (en producción haría ping al más antiguo)
        false
    }

    pub fn remove(&mut self, id: &NodeId) {
        self.nodes.retain(|n| &n.id != id);
    }

    pub fn nodes(&self) -> &[NodeInfo] { &self.nodes }

    pub fn len(&self) -> usize { self.nodes.len() }

    pub fn is_empty(&self) -> bool { self.nodes.is_empty() }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node(seed: &[u8]) -> NodeInfo {
        NodeInfo::new(NodeId::from_seed(seed), "127.0.0.1:0".into())
    }

    #[test]
    fn upsert_and_len() {
        let mut b = KBucket::new();
        assert!(b.upsert(node(b"a")));
        assert!(b.upsert(node(b"b")));
        assert_eq!(b.len(), 2);
    }

    #[test]
    fn upsert_duplicate_does_not_grow() {
        let mut b = KBucket::new();
        b.upsert(node(b"a"));
        b.upsert(node(b"a"));
        assert_eq!(b.len(), 1);
    }

    #[test]
    fn full_bucket_rejects_new() {
        let mut b = KBucket::new();
        for i in 0..K {
            b.upsert(node(format!("node{}", i).as_bytes()));
        }
        assert_eq!(b.len(), K);
        assert!(!b.upsert(node(b"overflow")));
    }

    #[test]
    fn remove_works() {
        let mut b = KBucket::new();
        let n = node(b"a");
        let id = n.id;
        b.upsert(n);
        b.remove(&id);
        assert!(b.is_empty());
    }
}
