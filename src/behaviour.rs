use libp2p::{gossipsub, kad::store::MemoryStore, mdns, swarm::NetworkBehaviour};
use libp2p_stream::Behaviour as Stream;

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
