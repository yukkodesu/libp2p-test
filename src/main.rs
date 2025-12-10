mod behaviour;
mod network;
mod peer_manager;

use std::error::Error;

use futures::{AsyncReadExt, AsyncWriteExt, StreamExt};
use libp2p::{Multiaddr, StreamProtocol, identity, mdns, swarm::SwarmEvent};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt as TokioAsyncWriteExt};
use tracing_subscriber::EnvFilter;

use crate::{behaviour::StrandsBehaviourEvent, network::create_swarm};

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
    let mut incoming = control
        .accept(StreamProtocol::new("/stream/1.0.0"))
        .unwrap();

    loop {
        if (tokio::signal::ctrl_c().await).is_ok() {
            println!("Ctrl-C received, shutting down.");
            break;
        }
        tokio::select! {
            event = swarm.select_next_some() => {
                match event {
                    SwarmEvent::NewListenAddr { address, .. } => {
                        println!("Listening on {address}");
                    }
                    SwarmEvent::Behaviour(StrandsBehaviourEvent::Mdns(event)) => {
                        match event {
                            mdns::Event::Discovered(list) => {
                                for (peer_id, multiaddr) in list {
                                    swarm.behaviour_mut().kad.add_address(&peer_id, multiaddr.clone());
                                    let mut peer_manager = peer_manager.write().await;
                                    peer_manager.add_peer(peer_id, multiaddr);
                                }
                            }
                            mdns::Event::Expired(list) => {
                                for (peer_id, _multiaddr) in list {
                                    let mut peer_manager = peer_manager.write().await;
                                    peer_manager.remove_peer(&peer_id);
                                }
                            }
                        }
                    }
                    // handle disconnect
                    SwarmEvent::ConnectionClosed { peer_id, .. } => {
                        println!("❌ 与节点 {} 的连接已关闭", peer_id);
                        let mut peer_manager = peer_manager.write().await;
                        peer_manager.remove_stream(&peer_id);
                        peer_manager.remove_peer(&peer_id);
                    }
                    SwarmEvent::Behaviour(event) => {
                        println!("event: {event:?}");
                    }
                    _ => {}
                }
            }
            Some((peer_id, stream)) = incoming.next() => {
                println!("✅ 接受来自节点 {} 的连接", peer_id);
                let mut peer_manager_write = peer_manager.write().await;
                peer_manager_write.add_stream(peer_id, stream);
            }
        }

        tokio::select! {
            _ = async {
                let mut to_remove = Vec::new();
                let mut pm = peer_manager.write().await;
                for (peer_id, stream) in pm.stream_iter() {
                    let mut buf = vec![0u8; 4096];

                    match stream.read(&mut buf).await {
                        Ok(n) if n > 0 => {
                            let _ = buf_writer.write_all(&buf[..n]).await;
                        }
                        Ok(_) => {}
                        Err(e) => {
                            eprintln!("❌ 读取节点 {} 的流时出错: {}", peer_id, e);
                            to_remove.push(*peer_id);
                        }
                    }
                }

                for peer_id in to_remove {
                    pm.remove_stream(&peer_id);
                }
                drop(pm);
            } => {}
            result = buf_reader.read_line(&mut line) => {
                let n = result?;
                if n == 0 {
                    continue;
                }

                // 处理命令
                let input = line.trim();
                match input {
                    "list" | "peers" => {
                        let peer_manager = peer_manager.read().await;
                        peer_manager.list_peers();
                        line.clear();
                        continue;
                    }
                    "connected" => {
                        let peers: Vec<_> = swarm.connected_peers().copied().collect();
                        if peers.is_empty() {
                            println!("📭 当前没有连接的节点");
                        } else {
                            println!("\n📋 当前连接的节点列表 (共 {} 个):", peers.len());
                            let peer_manager = peer_manager.read().await;
                            for peer_id in peers {
                                println!("🔹 节点ID: {}", peer_id);
                                if let Some(addrs) = peer_manager.get_peer_addrs(&peer_id) {
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
                    let mut peer_manager = peer_manager.write().await;
                    let stream = peer_manager.get_or_insert_stream(peer_id, || async {
                        control
                            .open_stream(peer_id, StreamProtocol::new("/stream/1.0.0"))
                            .await
                            .unwrap()
                    }).await;
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
