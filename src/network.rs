use std::{error::Error, time::Duration};

use crate::behaviour::StrandsBehaviour;
use bytes::Bytes;
use futures::stream::FuturesUnordered;
use libp2p::{
    Swarm, SwarmBuilder, identity,
    kad::{self, store::MemoryStore},
};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt};

pub fn create_swarm(keypair: identity::Keypair) -> Result<Swarm<StrandsBehaviour>, Box<dyn Error>> {
    let peer_id = identity::PeerId::from(keypair.public());
    let gossipsub = libp2p::gossipsub::Behaviour::new(
        libp2p::gossipsub::MessageAuthenticity::Signed(keypair.clone()),
        libp2p::gossipsub::Config::default(),
    )?;

    let mdns = libp2p::mdns::tokio::Behaviour::new(libp2p::mdns::Config::default(), peer_id)?;

    let stream = libp2p_stream::Behaviour::new();

    let kad = kad::Behaviour::new(peer_id, MemoryStore::new(peer_id));

    let behaviour = StrandsBehaviour::new(gossipsub, mdns, stream, kad);
    let swarm = SwarmBuilder::with_existing_identity(keypair)
        .with_tokio()
        .with_quic()
        .with_behaviour(|_| behaviour)?
        .with_swarm_config(|cfg| cfg.with_idle_connection_timeout(Duration::from_secs(u64::MAX)))
        .build();

    Ok(swarm)
}

pub type RecvTask = FuturesUnordered<tokio::task::JoinHandle<Vec<u8>>>;

pub async fn handle_stdin(
    tx: tokio::sync::mpsc::Sender<Bytes>,
    rx: &mut tokio::sync::mpsc::Receiver<Bytes>,
) {
    let mut buf_reader = tokio::io::BufReader::new(tokio::io::stdin());
    let mut buf_writer = tokio::io::BufWriter::new(tokio::io::stdout());
    loop {
        let mut buffer = String::new();
        tokio::select! {
            Ok(n) = buf_reader.read_line(&mut buffer) => {
                if n == 0 {
                    break;
                };
                if let Err(e) = tx.send(buffer.into()).await {
                    println!("❌ 发送数据失败: {}", e);
                }
            }
            Some(data) = rx.recv() => {
                buf_writer.write_all(&data).await.unwrap();
                buf_writer.flush().await.unwrap();
            }
        }
    }
}
