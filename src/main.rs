mod behaviour;
mod network;
mod peer_manager;

use std::error::Error;

use bytes::Bytes;
use futures::{AsyncReadExt, AsyncWriteExt, StreamExt, stream::FuturesUnordered};
use libp2p::{Multiaddr, StreamProtocol, identity};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt as TokioAsyncWriteExt};
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
    let mut control = swarm.behaviour().stream.new_control();

    let peer_manager_clone = peer_manager.clone();
    let mut control_clone = control.clone();
    tokio::spawn(async move {
        let mut incoming = control_clone
            .accept(StreamProtocol::new("/stream/1.0.0"))
            .unwrap();
        let mut tasks: FuturesUnordered<tokio::task::JoinHandle<Option<Vec<u8>>>> =
            FuturesUnordered::new();
        loop {
            tokio::select! {
                // handle all recv_tasks futureUnordered in peer_manager
                Some(data) = tasks.next() => {
                    // 任务完成，可能是连接关闭或出错
                    let _ = data.expect("Task panicked");
                }
                Some((peer_id, stream)) = incoming.next() => {
                    peer_manager_clone.add_stream(peer_id, stream).await;
                    let peer_manager_clone = peer_manager_clone.clone();
                    let mut control_clone = control_clone.clone();
                    tasks.push(tokio::spawn(async move {
                        let stream = peer_manager_clone.get_or_insert_stream(peer_id, || async {
                            control_clone
                                .open_stream(peer_id, StreamProtocol::new("/stream/1.0.0"))
                                .await
                                .unwrap()
                        }).await;
                        let mut stream = stream.lock().await;
                        let mut buf = vec![0u8; 1024];
                        // 持续循环读取，而不是只读取一次
                        loop {
                            match stream.read(&mut buf).await {
                                Ok(0) => {
                                    println!("❌ 连接已关闭: {}", peer_id);
                                    peer_manager_clone.remove_stream(&peer_id).await;
                                    break;
                                }
                                Ok(n) => {
                                    // 打印接收到的消息
                                    if let Ok(msg) = String::from_utf8(buf[..n].to_vec()) {
                                        println!("📩 收到来自 {} 的消息: {}", peer_id, msg.trim());
                                    }
                                    // 继续读取下一条消息
                                }
                                Err(e) => {
                                    eprintln!("❌ 从 {} 读取数据失败: {}", peer_id, e);
                                    peer_manager_clone.remove_stream(&peer_id).await;
                                    break;
                                }
                            }
                        }
                        None
                    }));
                }
            }
        }
    });

    loop {
        let mut line = String::new();
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
                let input = line.trim().to_string();
                match input.as_str() {
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
                // let input = line.as_bytes();
                // send to connected peers
                for peer_id in swarm.connected_peers() {
                    let stream = peer_manager.get_or_insert_stream(*peer_id, || async {
                        control
                            .open_stream(*peer_id, StreamProtocol::new("/stream/1.0.0"))
                            .await
                            .unwrap()
                    }).await;
                    let mut stream = stream.lock().await;
                    if let Err(e) = stream.write_all(input.as_bytes()).await {
                        eprintln!("❌ 发送消息到 {} 失败: {}", peer_id, e);
                        continue;
                    }
                    println!("✅ 已发送消息到 {}", peer_id);
                }
                line.clear();
            }
        }
    }
    Ok(())
}
