use std::net::SocketAddr;
use std::path::PathBuf;
use std::time::Duration;

use clap::Parser;
use tokio::net::UdpSocket;
use tokio::time::{interval, MissedTickBehavior};
use tracing::{debug, info, warn};

use coop_relay::lobby::{Rejection, RoomManager, DEFAULT_MAX_ROOMS};
use coop_relay::protocol::{
    ClientToServerPacket, ProtocolError, ServerToClientPacket, CLIENT_PACKET_MAX,
    ERROR_PACKET_SIZE, PROTOCOL_VERSION, ROOM_STATUS_ERR_PROTOCOL,
};
use coop_relay::quic::{self, QuicServer};

#[derive(Parser, Debug)]
#[command(
    name = "coop-relay",
    version,
    about = "Pokemon HeartGold 6-Player Co-op Multi-Room Relay Server"
)]
struct Args {
    /// Address and UDP port to bind
    #[arg(short, long, default_value = "0.0.0.0:7777")]
    bind: SocketAddr,

    /// Broadcast tick frequency in Hz. The ROM samples telemetry at 30 Hz;
    /// 60 Hz halves the relay's queueing delay (about 22 KB/s per client).
    #[arg(short, long, default_value_t = 60)]
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

    /// Seconds without packets before a player's slot is freed
    #[arg(long, default_value_t = 5)]
    client_timeout_secs: u64,

    /// Seconds an empty room stays in memory before it is closed
    #[arg(long, default_value_t = 120)]
    room_idle_secs: u64,

    /// Maximum number of rooms held in memory at once
    #[arg(long, default_value_t = DEFAULT_MAX_ROOMS)]
    max_rooms: usize,

    /// Also listen for QUIC (encrypted; the unified client) on this address and UDP port, e.g.
    /// 0.0.0.0:7778. Players add the fingerprint printed at startup to the server address.
    #[arg(long)]
    quic: Option<SocketAddr>,
}

/// Sends a packet to a player: over QUIC to a QUIC player's stand-in address, else UDP.
async fn send(socket: &UdpSocket, quic: Option<&QuicServer>, addr: SocketAddr, payload: Vec<u8>) {
    if quic::is_virtual(&addr) {
        if let Some(q) = quic {
            q.send(addr, payload);
        }
    } else if let Err(err) = socket.send_to(&payload, addr).await {
        debug!("Failed sending to {addr}: {err}");
    }
}

/// One packet from a player, by UDP or QUIC: the replies to send.
fn handle(
    manager: &mut RoomManager,
    data: &[u8],
    peer_addr: SocketAddr,
) -> Vec<(SocketAddr, Vec<u8>)> {
    match ClientToServerPacket::decode_le(data) {
        Ok(pkt) => match manager.handle_client_packet(peer_addr, pkt) {
            Ok(_) => manager.take_immediate(),
            // Both replies are smaller than the request (no amplification).
            Err(Rejection::Challenge(cookie)) => {
                vec![(
                    peer_addr,
                    ServerToClientPacket::create_challenge_response(cookie).to_vec(),
                )]
            }
            Err(Rejection::Status(status)) => {
                vec![(
                    peer_addr,
                    ServerToClientPacket::create_error_response(status).to_vec(),
                )]
            }
        },
        // Another protocol version: say so (in the frozen 16-byte header, never more bytes than
        // the request), so its bridge can tell the player which versions differ.
        Err(ProtocolError::UnsupportedVersion(version)) if data.len() >= ERROR_PACKET_SIZE => {
            info!("Refused {peer_addr}: relay protocol {version}, this relay speaks {PROTOCOL_VERSION}");
            vec![(
                peer_addr,
                ServerToClientPacket::create_error_response(ROOM_STATUS_ERR_PROTOCOL).to_vec(),
            )]
        }
        Err(err) => {
            debug!("Dropped malformed packet from {peer_addr}: {err}");
            Vec::new()
        }
    }
}

/// SIGTERM (systemd, `docker stop`): as Ctrl+C. Inside a container the relay is PID 1, which
/// the kernel never kills for an unhandled SIGTERM. Elsewhere it never fires.
async fn terminate_signal() {
    #[cfg(unix)]
    if let Ok(mut term) = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
    {
        term.recv().await;
        return;
    }
    std::future::pending::<()>().await
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .with_target(false)
        // Colours for a terminal only: journald and `docker logs` would show the escape codes.
        .with_ansi(std::io::IsTerminal::is_terminal(&std::io::stdout()))
        .init();

    let args = Args::parse();
    let socket = UdpSocket::bind(args.bind).await?;
    info!(
        "HGSS Multi-Room Co-op Relay {} (relay protocol {}) listening on UDP {} (Tick: {} Hz | Saves: {:?})",
        env!("CARGO_PKG_VERSION"),
        PROTOCOL_VERSION,
        args.bind,
        args.tick_rate,
        args.save_dir
    );

    let (quic_server, mut quic_rx) = match args.quic {
        Some(addr) => {
            let (server, rx, fp) = QuicServer::start(addr, &args.save_dir, CLIENT_PACKET_MAX)?;
            info!(
                "QUIC on UDP {addr}: players join with <host>:{}#{fp}",
                addr.port()
            );
            (Some(server), Some(rx))
        }
        None => (None, None),
    };

    let mut manager = RoomManager::new(
        Duration::from_secs(args.client_timeout_secs),
        args.default_party_size,
        args.inspect_all,
        &args.save_dir,
    )
    .with_limits(Duration::from_secs(args.room_idle_secs), args.max_rooms);

    let tick_period = Duration::from_micros(1_000_000 / u64::from(args.tick_rate.max(1)));
    let mut broadcast_timer = interval(tick_period);
    broadcast_timer.set_missed_tick_behavior(MissedTickBehavior::Skip);

    let mut summary_timer = interval(Duration::from_secs(5));
    summary_timer.set_missed_tick_behavior(MissedTickBehavior::Skip);

    // One byte over the largest client packet: a datagram that fills it was
    // cut short (recv_from truncates) and is dropped, never half-decoded.
    let mut recv_buf = [0u8; CLIENT_PACKET_MAX + 1];

    // Created once: re-creating ctrl_c() every loop iteration could drop a
    // signal that arrives between iterations.
    let shutdown = tokio::signal::ctrl_c();
    tokio::pin!(shutdown);
    let terminate = terminate_signal();
    tokio::pin!(terminate);

    loop {
        tokio::select! {
            recv_res = socket.recv_from(&mut recv_buf) => {
                match recv_res {
                    Ok((len, peer_addr)) if len > CLIENT_PACKET_MAX => {
                        warn!("Dropped an oversized packet from {peer_addr} ({len}+ bytes)");
                    }
                    Ok((len, peer_addr)) => {
                        for (addr, payload) in handle(&mut manager, &recv_buf[..len], peer_addr) {
                            send(&socket, quic_server.as_ref(), addr, payload).await;
                        }
                    }
                    Err(err) => {
                        warn!("UDP socket receive error: {err}");
                    }
                }
            }

            Some((peer_addr, data)) = async { quic_rx.as_mut()?.recv().await }, if quic_rx.is_some() => {
                for (addr, payload) in handle(&mut manager, &data, peer_addr) {
                    send(&socket, quic_server.as_ref(), addr, payload).await;
                }
            }

            _ = broadcast_timer.tick() => {
                let batch = manager.tick_all_rooms();
                for (addr, payload) in batch {
                    send(&socket, quic_server.as_ref(), addr, payload).await;
                }
            }

            _ = summary_timer.tick() => {
                manager.print_all_summaries();
            }

            _ = &mut shutdown => {
                info!("Shutting down HGSS Co-op Relay Server...");
                break;
            }
            _ = &mut terminate => {
                info!("Shutting down HGSS Co-op Relay Server (SIGTERM)...");
                break;
            }
        }
    }

    Ok(())
}
