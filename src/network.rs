use std::{error::Error, sync::Arc};

use futures::stream::FuturesUnordered;
use libp2p::{Swarm, SwarmBuilder, identity, kad::{self, store::MemoryStore}};
use tokio::sync::RwLock;
use crate::behaviour::StrandsBehaviour;

pub fn create_swarm(keypair: identity::Keypair) -> Result<Swarm<StrandsBehaviour>, Box<dyn Error>> {

    let peer_id = identity::PeerId::from(keypair.public());
    let gossipsub = libp2p::gossipsub::Behaviour::new(
        libp2p::gossipsub::MessageAuthenticity::Signed(keypair.clone()),
        libp2p::gossipsub::Config::default(),
    )?;

    let mdns = libp2p::mdns::tokio::Behaviour::new(
        libp2p::mdns::Config::default(),
        peer_id
    )?;

    let stream = libp2p_stream::Behaviour::new();

    let kad = kad::Behaviour::new(
        peer_id,
        MemoryStore::new(peer_id),
    );

    let behaviour = StrandsBehaviour::new(gossipsub, mdns, stream, kad);
    let swarm = SwarmBuilder::with_existing_identity(keypair)
        .with_tokio()
        .with_quic()
        .with_behaviour(|_| behaviour)?
        .build();

    Ok(swarm)
}



pub type RecvTask = FuturesUnordered<tokio::task::JoinHandle<Vec<u8>>>;
