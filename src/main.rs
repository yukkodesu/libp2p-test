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
    let mut peer_manager = peer_manager::PeerManager::new();

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
                                    peer_manager.add_peer(peer_id, multiaddr);
                                }
                            }
                            mdns::Event::Expired(list) => {
                                for (peer_id, _multiaddr) in list {
                                    peer_manager.remove_peer(&peer_id);
                                }
                            }
                        }
                    }
                    // handle disconnect
                    SwarmEvent::ConnectionClosed { peer_id, .. } => {
                        println!("❌ 与节点 {} 的连接已关闭", peer_id);
                        peer_manager.remove_stream(&peer_id);
                    }
                    SwarmEvent::Behaviour(event) => {
                        println!("event: {event:?}");
                    }
                    _ => {}
                }
            }
            res = tokio::signal::ctrl_c() => {
                res?;
                println!("Ctrl-C received, shutting down.");
                break;
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
                        peer_manager.list_peers();
                        line.clear();
                        continue;
                    }
                    "count" => {
                        println!("📊 当前发现的节点数量: {}", peer_manager.peer_count());
                        line.clear();
                        continue;
                    }
                    "help" => {
                        println!("\n可用命令:");
                        println!("  list/peers - 列出所有已发现的节点");
                        println!("  count      - 显示节点数量");
                        println!("  help       - 显示此帮助信息");
                        println!("  <其他输入> - 发送消息给所有连接的节点\n");
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
                    let stream = peer_manager.get_or_insert_stream(peer_id, || async {
                        control
                            .open_stream(peer_id, StreamProtocol::new("/stream/1.0.0"))
                            .await
                            .unwrap()
                    }).await;
                    stream.write_all(input).await.unwrap();
                    println!("➡️  发送消息到 {}: {}", peer_id, String::from_utf8_lossy(input));
                }
                line.clear();
            }
            Some((peer_id, stream)) = incoming.next() => {
                println!("✅ 接受来自节点 {} 的连接", peer_id);
                peer_manager.add_stream(peer_id, stream);
            }
            // poll every existing stream for incoming messages
            // iterate over peer_manager streams
            res = async {
                let mut buf: Vec<u8> = Vec::with_capacity(4096);
                for (peer_id, stream) in peer_manager.streams.iter_mut() {
                    match stream.read(&mut buf).await {
                        Ok(n) => {
                            let msg = String::from_utf8_lossy(&buf[..n]);
                            println!("⬅️  来自节点 {} 的消息: {}", peer_id, msg);
                        }
                        Err(e) => {
                            println!("⚠️  读取来自节点 {} 的消息时出错: {}", peer_id, e);
                        }
                    }

                }
                buf
            } => {
                buf_writer.write_all(&res).await?;
                buf_writer.flush().await?;
            }
        }
    }
    Ok(())
}
