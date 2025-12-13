mod behaviour;
mod network;
mod peer_manager;

use std::{error::Error, sync::Arc};

use futures::{AsyncReadExt, AsyncWriteExt, FutureExt, StreamExt};
use libp2p::{Multiaddr, StreamProtocol, identity, swarm};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt as TokioAsyncWriteExt},
    pin,
    sync::RwLock,
};
use tracing_subscriber::EnvFilter;

use crate::{behaviour::handle_swarm_event, network::create_swarm};

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

    let mut buf_reader = tokio::io::BufReader::new(tokio::io::stdin());
    let mut buf_writer = tokio::io::BufWriter::new(tokio::io::stdout());
    let mut line = String::new();
    let mut control = swarm.behaviour().stream.new_control();

    let peer_manager_clone = peer_manager.clone();
    let mut control_clone = control.clone();
    tokio::spawn(async move {
        let mut incoming = control_clone
            .accept(StreamProtocol::new("/stream/1.0.0"))
            .unwrap();
        loop {
            tokio::select! {
                // handle all recv_task futureUnordered in peer_manager
                // Some(_) = peer_manager_clone.recv_task.write().await.next() => {

                // }
                Some((peer_id, stream)) = incoming.next() => {
                    println!("✅ 接受来自节点 {} 的连接", peer_id);
                }
            }
        }
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
            result = buf_reader.read_line(&mut line) => {
                let n = result?;
                if n == 0 {
                    continue;
                }

                // 处理命令
                let input = line.trim();
                match input {
                    "list" | "peers" => {
                        peer_manager.list_peers().await;
                        line.clear();
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
                        line.clear();
                        continue;
                    }
                    _ => {}
                }

                if input.starts_with("dial") {
                    let parts: Vec<&str> = input.split_whitespace().collect();
                    if parts.len() != 2 {
                        println!("用法: dial <multiaddr>");
                        line.clear();
                        continue;
                    }
                    swarm.dial(parts[1].parse::<Multiaddr>()?)?;
                    println!("正在连接到 {}", parts[1]);
                    line.clear();
                    continue;
                }

                // 发送消息给连接的节点
                let input = line.as_bytes();
                // send to connected peers
                let peers: Vec<_> = swarm.connected_peers().copied().collect();
                for peer_id in peers {
                    let stream = control
                            .open_stream(peer_id, StreamProtocol::new("/stream/1.0.0"))
                            .await
                            .unwrap();
                    let (tx, rx) = stream.split();
                    // let stream = peer_manager.get_or_insert_stream(peer_id, || async {

                    // }).await;
                    let mut stream = stream.write().await;
                    if let Err(e) = stream.write_all(input).await {
                        eprintln!("❌ 发送消息到 {} 失败: {}", peer_id, e);
                        continue;
                    }
                    println!("➡️  发送消息到 {}: {}", peer_id, String::from_utf8_lossy(input));
                }
                line.clear();
            }
        }
    }
    Ok(())
}
