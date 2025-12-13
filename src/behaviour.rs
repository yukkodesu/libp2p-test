use libp2p::{
    gossipsub,
    kad::store::MemoryStore,
    mdns,
    swarm::{NetworkBehaviour, SwarmEvent},
};
use libp2p_stream::{Behaviour as Stream};

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

pub async fn handle_swarm_event(event: SwarmEvent<StrandsBehaviourEvent>, peer_manager: &crate::peer_manager::SharedPeerManager, swarm: &mut libp2p::Swarm<StrandsBehaviour>) {
    match event {
        SwarmEvent::NewListenAddr { address, .. } => {
            println!("Listening on {address}");
        }
        SwarmEvent::Behaviour(StrandsBehaviourEvent::Mdns(event)) => match event {
            mdns::Event::Discovered(list) => {
                for (peer_id, multiaddr) in list {
                    swarm
                        .behaviour_mut()
                        .kad
                        .add_address(&peer_id, multiaddr.clone());
                    peer_manager.add_peer(peer_id, multiaddr).await;
                }
            }
            mdns::Event::Expired(list) => {
                for (peer_id, _multiaddr) in list {
                    peer_manager.remove_peer(&peer_id).await;
                }
            }
        }
        // handle disconnect
        SwarmEvent::ConnectionClosed { peer_id, .. } => {
            println!("❌ 与节点 {} 的连接已关闭", peer_id);
            peer_manager.remove_peer(&peer_id).await;
        }
        SwarmEvent::Behaviour(event) => {
            println!("event: {event:?}");
        }
        _ => {}
    }
}
