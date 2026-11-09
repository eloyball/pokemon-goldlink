use std::net::SocketAddr;
use std::path::PathBuf;
use std::time::Duration;

use clap::Parser;
use tokio::net::UdpSocket;
use tokio::time::{MissedTickBehavior, interval};
use tracing::{debug, info, warn};

use coop_relay::lobby::{DEFAULT_MAX_ROOMS, Rejection, RoomManager};
use coop_relay::protocol::{
    CLIENT_PACKET_MAX, ClientToServerPacket, ERROR_PACKET_SIZE, PROTOCOL_VERSION, ProtocolError,
    QuicOffer, ROOM_STATUS_ERR_PROTOCOL, ServerToClientPacket, challenge_with_offer,
};
use coop_relay::quic::{self, PlainPacket, QuicServer};

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

    /// Listen for QUIC (encrypted; the unified client) on this separate address and UDP port.
    /// By default QUIC shares the bind address's port with the relay's plain packets.
    #[arg(long)]
    quic: Option<SocketAddr>,

    /// Don't listen for QUIC: players connect over plain UDP only.
    #[arg(long, conflicts_with = "quic")]
    no_quic: bool,
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

/// What the lobby queued for QUIC players' link and trade streams (protocol 9).
fn flush_streams(manager: &mut RoomManager, quic: Option<&QuicServer>) {
    let links = manager.take_link_stream_out();
    let trades = manager.take_trade_stream_out();
    if let Some(q) = quic {
        for o in links {
            q.send_link(o.addr, o.epoch, &o.msg);
        }
        for o in trades {
            q.send_trade(o.addr, o.frame);
        }
    }
}

/// One packet from a player, by UDP or QUIC: the replies to send.
fn handle(
    manager: &mut RoomManager,
    data: &[u8],
    peer_addr: SocketAddr,
    offer: Option<QuicOffer>,
) -> Vec<(SocketAddr, Vec<u8>)> {
    match ClientToServerPacket::decode_le(data) {
        Ok(pkt) => match manager.handle_client_packet(peer_addr, pkt) {
            Ok(_) => manager.take_immediate(),
            // Both replies are smaller than the request (no amplification).
            // Protocol 9: with the QUIC offer (port, fingerprint) before any secret is sent.
            Err(Rejection::Challenge(cookie)) => {
                vec![(peer_addr, challenge_with_offer(cookie, offer))]
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
            info!(
                "Refused {peer_addr}: relay protocol {version}, this relay speaks {PROTOCOL_VERSION}"
            );
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
    info!(
        "HGSS Multi-Room Co-op Relay {} (relay protocol {}) listening on UDP {} (Tick: {} Hz | Saves: {:?})",
        env!("CARGO_PKG_VERSION"),
        PROTOCOL_VERSION,
        args.bind,
        args.tick_rate,
        args.save_dir
    );

    // QUIC by default (protocol 9, rework plan §7.3), on the relay's own port: one port to
    // open. `--quic <addr>` puts it on a port of its own; `--no-quic` turns it off.
    let fp_line = |port: u16, fp: &[u8; 32]| {
        format!(
            "players join with <host>:{} and trust this certificate on first use, or pin it \
             with <host>:{port}#{}",
            args.bind.port(),
            quic::fingerprint_hex(fp)
        )
    };
    let mut plain_rx: Option<tokio::sync::mpsc::Receiver<PlainPacket>> = None;
    let (socket, quic_server, mut quic_rx, offer) = if args.no_quic {
        info!("QUIC off (--no-quic): players connect over plain UDP");
        (UdpSocket::bind(args.bind).await?, None, None, None)
    } else if let Some(addr) = args.quic {
        let started = QuicServer::start(addr, &args.save_dir, CLIENT_PACKET_MAX)?;
        let fp = started.fingerprint;
        info!(
            "QUIC on UDP {addr} (open this port too): {}",
            fp_line(addr.port(), &fp)
        );
        let offer = QuicOffer {
            port: addr.port(),
            fingerprint: fp,
        };
        let socket = UdpSocket::bind(args.bind).await?;
        (
            socket,
            Some(started.server),
            Some(started.incoming),
            Some(offer),
        )
    } else {
        let started = QuicServer::start_shared(args.bind, &args.save_dir, CLIENT_PACKET_MAX)?;
        let fp = started.fingerprint;
        let shared = started.shared.expect("a shared port");
        info!(
            "QUIC on the same UDP port {}: {}",
            args.bind.port(),
            fp_line(args.bind.port(), &fp)
        );
        plain_rx = Some(shared.plain);
        let offer = QuicOffer {
            port: args.bind.port(),
            fingerprint: fp,
        };
        (
            shared.socket,
            Some(started.server),
            Some(started.incoming),
            Some(offer),
        )
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
            // The relay's own packets: from the socket, or from QUIC's when they share it.
            recv_res = socket.recv_from(&mut recv_buf), if plain_rx.is_none() => {
                match recv_res {
                    Ok((len, peer_addr)) if len > CLIENT_PACKET_MAX => {
                        warn!("Dropped an oversized packet from {peer_addr} ({len}+ bytes)");
                    }
                    Ok((len, peer_addr)) => {
                        for (addr, payload) in handle(&mut manager, &recv_buf[..len], peer_addr, offer) {
                            send(&socket, quic_server.as_ref(), addr, payload).await;
                        }
                        flush_streams(&mut manager, quic_server.as_ref());
                    }
                    Err(err) => {
                        warn!("UDP socket receive error: {err}");
                    }
                }
            }

            Some((peer_addr, data)) = async { plain_rx.as_mut()?.recv().await }, if plain_rx.is_some() => {
                if data.len() > CLIENT_PACKET_MAX {
                    warn!("Dropped an oversized packet from {peer_addr} ({} bytes)", data.len());
                } else {
                    for (addr, payload) in handle(&mut manager, &data, peer_addr, offer) {
                        send(&socket, quic_server.as_ref(), addr, payload).await;
                    }
                    flush_streams(&mut manager, quic_server.as_ref());
                }
            }

            Some(event) = async { quic_rx.as_mut()?.recv().await }, if quic_rx.is_some() => {
                let replies = match event {
                    quic::Incoming::Packet(peer_addr, data) => handle(&mut manager, &data, peer_addr, offer),
                    quic::Incoming::Link(peer_addr, event) => {
                        manager.link_stream_event(peer_addr, event);
                        manager.take_immediate()
                    }
                    quic::Incoming::Trade(peer_addr, event) => {
                        manager.trade_stream_event(peer_addr, event);
                        Vec::new()
                    }
                };
                for (addr, payload) in replies {
                    send(&socket, quic_server.as_ref(), addr, payload).await;
                }
                flush_streams(&mut manager, quic_server.as_ref());
            }

            _ = broadcast_timer.tick() => {
                let batch = manager.tick_all_rooms();
                for (addr, payload) in batch {
                    send(&socket, quic_server.as_ref(), addr, payload).await;
                }
                flush_streams(&mut manager, quic_server.as_ref());
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
