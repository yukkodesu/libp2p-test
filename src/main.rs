mod behaviour;
mod network;
mod peer_manager;

use core::panic;
use std::{error::Error, sync::{Arc, RwLock}};

use bytes::Bytes;
use futures::{AsyncReadExt, AsyncWriteExt, StreamExt, stream::FuturesUnordered};
use libp2p::{Multiaddr, StreamProtocol, identity};
use tokio::{io::{AsyncBufReadExt, AsyncWriteExt as TokioAsyncWriteExt}, sync::Mutex};
use tracing_subscriber::EnvFilter;

use crate::{
    behaviour::handle_swarm_event,
    network::{create_swarm, handle_stdin},
};

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let _ = tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .try_init();

    let identity = identity::Keypair::generate_ed25519();

    let mut swarm = create_swarm(identity)?;
    let peer_manager = peer_manager::PeerManager::new_shared();
    // parse listen address from command line or use default
    // --port <port>
    let args = std::env::args().collect::<Vec<String>>();
    if args.len() < 3 {
        println!("Usage: {} --port <port>", args[0]);
        return Err("Not enough arguments".into());
    }
    let port = args[2].parse::<u16>()?;
    let listen_addr = format!("/ip4/0.0.0.0/udp/{}/quic-v1", port);
    swarm.listen_on(listen_addr.parse()?)?;

    let (stdout_tx, mut swarm_rx) = tokio::sync::mpsc::channel::<Bytes>(100);
    let (swarm_tx, mut stdin_rx) = tokio::sync::mpsc::channel::<Bytes>(100);
    let stdin_txs = Arc::new(Mutex::new(Vec::<tokio::sync::mpsc::Sender<Bytes>>::new()));

    let control = swarm.behaviour().stream.new_control();
    let mut control_clone = control.clone();
    tokio::spawn(async move {
        let mut incoming = control_clone
            .accept(StreamProtocol::new("/stream/1.0.0"))
            .unwrap();
        let mut tasks: FuturesUnordered<tokio::task::JoinHandle<()>> = FuturesUnordered::new();
        loop {
            tokio::select! {
                // handle all recv_tasks futureUnordered in peer_manager
                Some(_) = tasks.next() => {}
                Some(data) = stdin_rx.recv() => {
                    let txs = stdin_txs.lock().await;
                    for tx in txs.iter() {
                        if let Err(e) = tx.send(data.clone()).await {
                            println!("❌ 发送数据到节点失败: {}", e);
                        }
                    }
                }
                Some((peer_id, stream)) = incoming.next() => {
                    let stdout_tx = stdout_tx.clone();
                    let (stdin_tx, mut stdin_rx) = tokio::sync::mpsc::channel::<Bytes>(100);
                    stdin_txs.lock().await.push(stdin_tx);
                    tasks.push(tokio::spawn(async move {
                        let mut stream = stream;
                        let read_buf = &mut [0u8; 1024];
                        tokio::select! {
                            Some(data) = stdin_rx.recv() => {
                                stream.write_all(&data).await.unwrap();
                            }
                            result = stream.read(read_buf) => {
                                match result {
                                    Ok(0) => {
                                        println!("🔌 连接关闭: {}", peer_id);
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
                    }));
                }
            }
        }
    });

    tokio::spawn(async move {
        handle_stdin(swarm_tx, &mut swarm_rx).await;
    });
    loop {
        tokio::select! {
            // control+c to exit
            _ = tokio::signal::ctrl_c() => {
                println!("Received Ctrl+C, shutting down.");
                break;
            }
            event = swarm.select_next_some() => {
                handle_swarm_event(event, &peer_manager, &mut swarm).await;
            }
        }
    }
    Ok(())
}
