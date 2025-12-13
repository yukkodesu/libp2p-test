use std::sync::Arc;

use futures::StreamExt;
use libp2p::{
    StreamProtocol, gossipsub,
    kad::store::MemoryStore,
    mdns,
    swarm::{NetworkBehaviour, SwarmEvent},
};
use libp2p_stream::{Behaviour as Stream, Control};
use tokio::sync::RwLock;

#[derive(NetworkBehaviour)]
pub struct StrandsBehaviour {
    pub gossipsub: gossipsub::Behaviour,
    pub mdns: mdns::tokio::Behaviour,
    pub stream: Stream,
    pub kad: libp2p::kad::Behaviour<MemoryStore>,
}

impl StrandsBehaviour {
    pub fn new(
        gossipsub: gossipsub::Behaviour,
        mdns: mdns::tokio::Behaviour,
        stream: Stream,
        kad: libp2p::kad::Behaviour<MemoryStore>,
    ) -> Self {
        Self {
            gossipsub,
            mdns,
            stream,
            kad,
        }
    }
}

pub async fn handle_behaviour_event(
    peer_manager: &crate::peer_manager::SharedPeerManager,
    swarm: Arc<RwLock<libp2p::Swarm<StrandsBehaviour>>>,
    mut control: Control
) {
    let mut incoming = control
        .accept(StreamProtocol::new("/stream/1.0.0"))
        .unwrap();
    tokio::select! {
        (mut swarm, event) = async {
            let mut swarm = swarm.write().await;
            let event = swarm.select_next_some().await;
            (swarm, event)
        } => {
            match event {
                SwarmEvent::NewListenAddr { address, .. } => {
                    println!("Listening on {address}");
                }
                SwarmEvent::Behaviour(StrandsBehaviourEvent::Mdns(event)) => {
                    match event {
                        mdns::Event::Discovered(list) => {
                            for (peer_id, multiaddr) in list {
                                swarm.behaviour_mut().kad.add_address(&peer_id, multiaddr.clone());
                                peer_manager.add_peer(peer_id, multiaddr).await;
                            }
                        }
                        mdns::Event::Expired(list) => {
                            for (peer_id, _multiaddr) in list {
                                peer_manager.remove_peer(&peer_id).await;
                            }
                        }
                    }
                }
                // handle disconnect
                SwarmEvent::ConnectionClosed { peer_id, .. } => {
                    println!("❌ 与节点 {} 的连接已关闭", peer_id);
                    peer_manager.remove_stream(&peer_id).await;
                    peer_manager.remove_peer(&peer_id).await;
                }
                SwarmEvent::Behaviour(event) => {
                    println!("event: {event:?}");
                }
                _ => {}
            }
        }
        Some((peer_id, stream)) = incoming.next() => {
            println!("✅ 接受来自节点 {} 的连接", peer_id);
            peer_manager.add_stream(peer_id, stream).await;
        }
    }
}
