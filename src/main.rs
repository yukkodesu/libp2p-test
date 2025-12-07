use std::{error::Error, time::Duration};

use futures::StreamExt;
use libp2p::{
    Multiaddr, StreamProtocol, SwarmBuilder,
    request_response::{self, ProtocolSupport, cbor},
    swarm::SwarmEvent,
};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt};
use tracing_subscriber::EnvFilter;

#[derive(Debug, serde::Serialize, serde::Deserialize)]
struct StringRequest {
    len: usize,
    content: Vec<u8>,
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
struct StringResponse {
    ok: bool,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let _ = tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .try_init();

    let behaviour = cbor::Behaviour::<StringRequest, StringResponse>::new(
        [(StreamProtocol::new("/string/1.0.0"), ProtocolSupport::Full)],
        request_response::Config::default(),
    );
    let mut swarm = SwarmBuilder::with_new_identity()
        .with_tokio()
        .with_quic()
        .with_behaviour(|_| behaviour)?
        .with_swarm_config(|cfg| cfg.with_idle_connection_timeout(Duration::from_secs(u64::MAX)))
        .build();

    swarm.listen_on("/ip4/0.0.0.0/udp/10086/quic-v1".parse()?)?;

    for argument in std::env::args() {
        println!("{argument}");
    }

    if let Some(addr) = std::env::args().nth(1) {
        println!("Input address: {}", addr);
        let remote: Multiaddr = addr.parse()?;
        swarm.dial(remote)?;
        println!("Dialed {}", addr);
    }

    let mut buf_reader = tokio::io::BufReader::new(tokio::io::stdin());
    let mut buf_writer = tokio::io::BufWriter::new(tokio::io::stdout());
    let mut line = String::new();
    loop {
        tokio::select! {
            event = swarm.select_next_some() => {
                match event {
                    SwarmEvent::NewListenAddr { address, .. } => {
                        println!("Listening on {address}");
                    }
                    SwarmEvent::Behaviour(request_response::Event::Message { message:request_response::Message::Request { request, .. }, ..}) => {
                        buf_writer.write_all(&request.content[..request.len]).await?;
                        buf_writer.flush().await?;
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
                let input = line.as_bytes();
                // send to connected peers
                let peers: Vec<_> = swarm.connected_peers().copied().collect();
                for peer_id in peers {
                    swarm.behaviour_mut().send_request(&peer_id, StringRequest {
                        len: input.len(),
                        content: input.to_vec(),
                    });
                }
                line.clear();
            }
        }
    }
    Ok(())
}
