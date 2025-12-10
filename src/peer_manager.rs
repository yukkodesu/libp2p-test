use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

use libp2p::Stream;


pub struct PeerManager {
    peers: HashMap<libp2p::PeerId, Vec<libp2p::Multiaddr>>,
    streams: HashMap<libp2p::PeerId, Stream>,
}

pub type SharedPeerManager = Arc<RwLock<PeerManager>>;

impl PeerManager {
    pub fn new() -> Self {
        Self {
            peers: HashMap::new(),
            streams: HashMap::new(),
        }
    }

    pub fn new_shared() -> SharedPeerManager {
        Arc::new(RwLock::new(Self::new()))
    }

    /// 添加或更新节点地址
    pub fn add_peer(&mut self, peer_id: libp2p::PeerId, addr: libp2p::Multiaddr) {
        self.peers.entry(peer_id)
            .or_default()
            .push(addr);
        println!("✅ 发现并添加节点: {} (地址: {})", peer_id, self.peers.get(&peer_id).unwrap().last().unwrap());
    }

    /// 移除节点
    pub fn remove_peer(&mut self, peer_id: &libp2p::PeerId) {
        if self.peers.remove(peer_id).is_some() {
            println!("🗑️  移除节点: {}", peer_id);
        }
    }
    pub fn get_peer_addrs(&self, peer_id: &libp2p::PeerId) -> Option<&Vec<libp2p::Multiaddr>> {
        self.peers.get(peer_id)
    }

    pub fn list_peers(&self) {
        if self.peers.is_empty() {
            println!("📭 暂无发现的节点");
            return;
        }
        println!("\n📋 已发现的节点列表 (共 {} 个):", self.peers.len());
        println!("{:-<80}", "");
        for (peer_id, addrs) in &self.peers {
            println!("🔹 节点ID: {}", peer_id);
            for addr in addrs {
                println!("   地址: {}", addr);
            }
            println!();
        }
        println!("{:-<80}", "");
    }

    pub fn add_stream(&mut self, peer_id: libp2p::PeerId, stream: Stream) {
        self.streams.insert(peer_id, stream);
    }
    pub fn get_stream(&mut self, peer_id: &libp2p::PeerId) -> Option<&mut Stream> {
        self.streams.get_mut(peer_id)
    }
    pub fn remove_stream(&mut self, peer_id: &libp2p::PeerId) {
        self.streams.remove(peer_id);
    }
    pub async fn get_or_insert_stream<F, Fut>(&mut self, peer_id: libp2p::PeerId, f: F) -> &mut Stream
    where
        F: FnOnce() -> Fut,
        Fut: std::future::Future<Output = Stream>,
    {
        if let std::collections::hash_map::Entry::Vacant(e) = self.streams.entry(peer_id) {
            let stream = f().await;
            e.insert(stream);
        }
        self.streams.get_mut(&peer_id).unwrap()
    }

    pub fn stream_iter(&mut self) -> impl Iterator<Item = (&libp2p::PeerId, &mut Stream)> {
        self.streams.iter_mut()
    }

    pub fn stream_count(&self) -> usize {
        self.streams.len()
    }
}