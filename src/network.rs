use std::{error::Error, sync::Arc, time::Duration};

use crate::behaviour::StrandsBehaviour;
use bytes::Bytes;
use futures::{AsyncReadExt, AsyncWriteExt, stream::FuturesUnordered};
use libp2p::{
    Swarm, SwarmBuilder, identity,
    kad::{self, store::MemoryStore},
};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt as TokioAsyncWriteExt},
    sync::Mutex,
};

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
    stdin_txs: Arc<Mutex<Vec<tokio::sync::mpsc::Sender<Bytes>>>>,
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
                for tx in stdin_txs.lock().await.iter() {
                    if let Err(e) = tx.send(Bytes::from(buffer.clone())).await {
                        println!("❌ 发送数据到节点失败: {}", e);
                    }
                }
            }
            Some(data) = rx.recv() => {
                buf_writer.write_all(&data).await.unwrap();
                buf_writer.flush().await.unwrap();
            }
        }
    }
}

pub fn handle_stream(
    stream: libp2p::Stream,
    mut stdin_rx: tokio::sync::mpsc::Receiver<Bytes>,
    stdout_tx: tokio::sync::mpsc::Sender<Bytes>,
    peer_id: libp2p::PeerId,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut stream = stream;
        let read_buf = &mut [0u8; 1024];
        loop {
            tokio::select! {
                Some(data) = stdin_rx.recv() => {
                    stream.write_all(&data).await.unwrap();
                }
                result = stream.read(read_buf) => {
                    match result {
                        Ok(0) => {
                            println!("🔌 连接关闭: {}", peer_id);
                            break;
                        }
                        Ok(n) => {
                            let received_data = &read_buf[..n];
                            stdout_tx.send(Bytes::from(received_data.to_vec())).await.unwrap();
                        }
                        Err(e) => {
                            println!("❌ 读取数据失败 ({}): {}", peer_id, e);
                        }
                    }
                }
            }
        }
    })
}
