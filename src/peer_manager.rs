use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{Mutex, RwLock};

use libp2p::{Stream, multiaddr::Iter};

pub type SharedStream = Arc<Mutex<Stream>>;

pub struct PeerManager {
    peers: RwLock<HashMap<libp2p::PeerId, Vec<libp2p::Multiaddr>>>,
}

pub type SharedPeerManager = Arc<PeerManager>;

impl PeerManager {
    pub fn new() -> Self {
        Self {
            peers: RwLock::new(HashMap::new()),
        }
    }

    pub fn new_shared() -> SharedPeerManager {
        Arc::new(Self::new())
    }

    pub async fn add_peer(&self, peer_id: libp2p::PeerId, addr: libp2p::Multiaddr) {
        let mut peers = self.peers.write().await;
        let addrs = peers.entry(peer_id).or_default();
        addrs.push(addr.clone());
        println!("✅ 发现并添加节点: {} (地址: {})", peer_id, addr);
    }

    pub async fn remove_peer(&self, peer_id: &libp2p::PeerId) {
        let mut peers = self.peers.write().await;
        if peers.remove(peer_id).is_some() {
            println!("🗑️  移除节点: {}", peer_id);
        }
    }

    pub async fn get_peer_addrs(&self, peer_id: &libp2p::PeerId) -> Option<Vec<libp2p::Multiaddr>> {
        let peers = self.peers.read().await;
        peers.get(peer_id).cloned()
    }

    pub async fn peer_iter(&self) -> impl Iterator<Item = libp2p::PeerId> {
        let peers = self.peers.read().await;
        peers.keys().copied().collect::<Vec<_>>().into_iter()
    }

    pub async fn list_peers(&self) {
        let peers = self.peers.read().await;
        if peers.is_empty() {
            println!("📭 暂无发现的节点");
            return;
        }
        println!("\n📋 已发现的节点列表 (共 {} 个):", peers.len());
        println!("{:-<80}", "");
        for (peer_id, addrs) in peers.iter() {
            println!("🔹 节点ID: {}", peer_id);
            for addr in addrs {
                println!("   地址: {}", addr);
            }
            println!();
        }
        println!("{:-<80}", "");
    }
}
