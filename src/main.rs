use std::net::SocketAddr;
use std::path::PathBuf;
use std::time::Duration;

use clap::Parser;
use tokio::net::UdpSocket;
use tokio::time::{interval, MissedTickBehavior};
use tracing::{debug, info, warn};

use coop_relay::lobby::RoomManager;
use coop_relay::protocol::ClientToServerPacket;

#[derive(Parser, Debug)]
#[command(name = "coop-relay", about = "Pokemon HeartGold 6-Player Co-op Multi-Room Relay Server")]
struct Args {
    /// Address and UDP port to bind
    #[arg(short, long, default_value = "0.0.0.0:7777")]
    bind: SocketAddr,

    /// Broadcast tick frequency in Hz
    #[arg(short, long, default_value_t = 30)]
    tick_rate: u32,

    /// Fallback party size if room creator does not specify one (2..6)
    #[arg(short = 'm', long = "min-players", default_value_t = 2)]
    default_party_size: usize,

    /// Directory where locked campaign rosters (<ROOM>.json) are persisted
    #[arg(long, default_value = "campaigns")]
    save_dir: PathBuf,

    /// Log every incoming UDP packet even when coordinates have not changed
    #[arg(long, default_value_t = false)]
    inspect_all: bool,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .with_target(false)
        .init();

    let args = Args::parse();
    let socket = UdpSocket::bind(args.bind).await?;
    info!(
        "HGSS Multi-Room Co-op Relay listening on UDP {} (Tick: {} Hz | Saves: {:?})",
        args.bind, args.tick_rate, args.save_dir
    );

    let mut manager = RoomManager::new(
        Duration::from_secs(5),
        args.default_party_size,
        args.inspect_all,
        &args.save_dir,
    );

    let tick_period = Duration::from_micros(1_000_000 / u64::from(args.tick_rate.max(1)));
    let mut broadcast_timer = interval(tick_period);
    broadcast_timer.set_missed_tick_behavior(MissedTickBehavior::Skip);

    let mut summary_timer = interval(Duration::from_secs(5));
    summary_timer.set_missed_tick_behavior(MissedTickBehavior::Skip);

    let mut recv_buf = [0u8; 512];

    loop {
        tokio::select! {
            recv_res = socket.recv_from(&mut recv_buf) => {
                match recv_res {
                    Ok((len, peer_addr)) => {
                        match ClientToServerPacket::decode_le(&recv_buf[..len]) {
                            Ok(pkt) => {
                                if let Err(err_pkt) = manager.handle_client_packet(peer_addr, pkt) {
                                    let _ = socket.send_to(&err_pkt, peer_addr).await;
                                }
                            }
                            Err(err) => {
                                debug!("Dropped malformed packet from {peer_addr}: {err}");
                            }
                        }
                    }
                    Err(err) => {
                        warn!("UDP socket receive error: {err}");
                    }
                }
            }

            _ = broadcast_timer.tick() => {
                let batch = manager.tick_all_rooms();
                for (addr, payload) in batch {
                    if let Err(err) = socket.send_to(&payload, addr).await {
                        debug!("Failed sending broadcast to {addr}: {err}");
                    }
                }
            }

            _ = summary_timer.tick() => {
                manager.print_all_summaries();
            }

            _ = tokio::signal::ctrl_c() => {
                info!("Shutting down HGSS Co-op Relay Server...");
                break;
            }
        }
    }

    Ok(())
}
