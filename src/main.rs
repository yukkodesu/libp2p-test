mod behaviour;
mod network;
mod peer_manager;

use core::panic;
use std::{
    error::Error,
    sync::{Arc, RwLock},
};

use bytes::Bytes;
use futures::{AsyncReadExt, AsyncWriteExt, StreamExt, stream::FuturesUnordered, task};
use libp2p::{Multiaddr, StreamProtocol, identity};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt as TokioAsyncWriteExt},
    sync::Mutex,
};
use tracing_subscriber::EnvFilter;

use crate::{
    behaviour::handle_swarm_event,
    network::{create_swarm, handle_stdin, handle_stream},
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
    let (stream_tx, mut stdin_rx) = tokio::sync::mpsc::channel::<Bytes>(100);
    let stdin_txs = Arc::new(Mutex::new(Vec::<tokio::sync::mpsc::Sender<Bytes>>::new()));
    stdin_txs.lock().await.push(stream_tx);
    let control = swarm.behaviour().stream.new_control();
    let mut control_clone = control.clone();
    let stdin_txs_clone = stdin_txs.clone();
    let peer_manager_clone = peer_manager.clone();
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
                    for peer_id in peer_manager_clone.peer_iter().await {
                        let stdout_tx = stdout_tx.clone();
                        let (stdin_tx, stream_stdin_rx) = tokio::sync::mpsc::channel::<Bytes>(100);
                        stdin_tx.send(data.clone()).await.unwrap();
                        stdin_txs_clone.lock().await.push(stdin_tx);
                        if let Ok(stream) = control_clone.open_stream(peer_id, StreamProtocol::new("/stream/1.0.0")).await {
                            tasks.push(handle_stream(stream, stream_stdin_rx, stdout_tx.clone(), peer_id));
                        }
                    }

                }
                Some((peer_id, stream)) = incoming.next() => {
                    let stdout_tx = stdout_tx.clone();
                    let (stdin_tx, stream_stdin_rx) = tokio::sync::mpsc::channel::<Bytes>(100);
                    stdin_txs_clone.lock().await.push(stdin_tx);
                    tasks.push(handle_stream(stream, stream_stdin_rx, stdout_tx, peer_id));
                }
            }
        }
    });

    let stdin_txs_clone = stdin_txs.clone();
    tokio::spawn(async move {
        handle_stdin(stdin_txs_clone, &mut swarm_rx).await;
    });
    let (swarm_loop_tx, mut swarm_loop_stdin_rx) = tokio::sync::mpsc::channel::<Bytes>(1);
    stdin_txs.lock().await.push(swarm_loop_tx);
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
            Some(data) = swarm_loop_stdin_rx.recv() => {
                let input = String::from_utf8_lossy(&data).to_string();
                let input = input.trim();
                match input {
                    "list" | "peers" => {
                        peer_manager.list_peers().await;
                        continue;
                    }
                    "connected" => {
                        let peers: Vec<_> = swarm.connected_peers().copied().collect();
                        if peers.is_empty() {
                            println!("📭 当前没有连接的节点");
                        } else {
                            println!("\n📋 当前连接的节点列表 (共 {} 个):", peers.len());
                            for peer_id in peers {
                                println!("🔹 节点ID: {}", peer_id);
                                if let Some(addrs) = peer_manager.get_peer_addrs(&peer_id).await {
                                    for addr in addrs {
                                        println!("   地址: {}", addr);
                                    }
                                }
                                println!();
                            }
                        }
                        continue;
                    }
                    _ => {}
                }

                if input.starts_with("dial") {
                    let parts: Vec<&str> = input.split_whitespace().collect();
                    if parts.len() != 2 {
                        println!("用法: dial <multiaddr>");
                        continue;
                    }
                    swarm.dial(parts[1].parse::<Multiaddr>()?)?;
                    println!("正在连接到 {}", parts[1]);
                    continue;
                }
            }
        }
    }
    Ok(())
}
