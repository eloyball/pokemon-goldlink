//! The relay over QUIC (UC6, RFC §5): the same packets as UDP, encrypted, the relay's
//! certificate pinned by the fingerprint in the server address. A thread runs a small tokio
//! runtime for quinn; packets go out as datagrams when they fit, else as a frame on a one-way
//! stream (u16 length, then the bytes), and come back into the same latest-wins slot as UDP.
//! The connection survives an address change (a NAT rebinding, Wi-Fi to cable): `rebind`.

use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::mpsc as std_mpsc;

use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{DigitallySignedStruct, SignatureScheme};
use sha2::{Digest, Sha256};
use tokio::sync::mpsc;

use crate::link::StreamCmd;
use crate::net::{Shared, accept};
use goldlink_proto as wire;

pub const ALPN: &[u8] = b"goldlink-relay";

enum Cmd {
    Send(Vec<u8>),
    Rebind(std_mpsc::Sender<bool>),
    /// Protocol 9: the link stream (one per link block, the rework plan §7.1).
    Link(StreamCmd),
    /// Protocol 9: the trade stream (one per trade exchange).
    Trade(TradeCmd),
}

/// What the bridge asks of the trade stream: one per trade exchange (protocol 9).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TradeCmd {
    /// The ROM opened a trade exchange.
    Open,
    /// The ROM's TradeOut changed.
    Send(Vec<u8>),
    /// The exchange closed.
    Close,
}

/// What went through the connection (dev tools print it).
#[derive(Default, Debug)]
pub struct Stats {
    pub sent_datagrams: std::sync::atomic::AtomicU64,
    pub sent_streams: std::sync::atomic::AtomicU64,
    pub send_failures: std::sync::atomic::AtomicU64,
    pub got_datagrams: std::sync::atomic::AtomicU64,
    pub got_streams: std::sync::atomic::AtomicU64,
    pub queued_before_connect: std::sync::atomic::AtomicU64,
}

impl Stats {
    pub fn line(&self) -> String {
        use std::sync::atomic::Ordering::Relaxed;
        format!(
            "sent {} datagrams, {} streams ({} failed, {} before connecting); got {} datagrams, {} streams",
            self.sent_datagrams.load(Relaxed),
            self.sent_streams.load(Relaxed),
            self.send_failures.load(Relaxed),
            self.queued_before_connect.load(Relaxed),
            self.got_datagrams.load(Relaxed),
            self.got_streams.load(Relaxed),
        )
    }
}

pub struct QuicClient {
    tx: mpsc::UnboundedSender<Cmd>,
    pub stats: Arc<Stats>,
    /// The connection's thread still runs (false once it ended: the relay gone, refused).
    alive: Arc<std::sync::atomic::AtomicBool>,
}

/// Accepts exactly the certificate with this SHA-256 (the relay's self-signed one).
#[derive(Debug)]
struct Pinned {
    pin: [u8; 32],
    provider: Arc<rustls::crypto::CryptoProvider>,
    /// A certificate that wasn't the pinned one (protocol 9: "its identity changed").
    other: Arc<std::sync::Mutex<Option<[u8; 32]>>>,
}

impl ServerCertVerifier for Pinned {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        let seen: [u8; 32] = Sha256::digest(end_entity.as_ref()).into();
        if seen == self.pin {
            Ok(ServerCertVerified::assertion())
        } else {
            *self.other.lock().unwrap_or_else(|e| e.into_inner()) = Some(seen);
            Err(rustls::Error::General(
                "the relay's certificate isn't the one in the server address".into(),
            ))
        }
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls12_signature(
            message,
            cert,
            dss,
            &self.provider.signature_verification_algorithms,
        )
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(
            message,
            cert,
            dss,
            &self.provider.signature_verification_algorithms,
        )
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.provider
            .signature_verification_algorithms
            .supported_schemes()
    }
}

fn client_config(
    pin: [u8; 32],
    other: Arc<std::sync::Mutex<Option<[u8; 32]>>>,
) -> Result<quinn::ClientConfig, String> {
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let mut tls = rustls::ClientConfig::builder_with_provider(provider.clone())
        .with_protocol_versions(&[&rustls::version::TLS13])
        .map_err(|e| e.to_string())?
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(Pinned {
            pin,
            provider,
            other,
        }))
        .with_no_client_auth();
    tls.alpn_protocols = vec![ALPN.to_vec()];
    let crypto =
        quinn::crypto::rustls::QuicClientConfig::try_from(tls).map_err(|e| e.to_string())?;
    let mut config = quinn::ClientConfig::new(Arc::new(crypto));
    let mut transport = quinn::TransportConfig::default();
    transport.keep_alive_interval(Some(std::time::Duration::from_secs(1)));
    // Past the relay's link deadline and the bridge's 65 s: a paused game's link stream
    // outlives the referee's decision (the relay's IDLE_TIMEOUT).
    transport.max_idle_timeout(Some(
        std::time::Duration::from_secs(75)
            .try_into()
            .map_err(|e| format!("{e}"))?,
    ));
    transport.datagram_receive_buffer_size(Some(1 << 20));
    // Real-time traffic: a stale packet is worth less than a fresh one. A small send buffer
    // drops the oldest instead of queueing seconds behind the congestion window, and BBR
    // doesn't read random loss (Wi-Fi, the internet) as congestion the way the default does.
    transport.datagram_send_buffer_size(16 * 1024);
    transport.congestion_controller_factory(Arc::new(quinn::congestion::BbrConfig::default()));
    config.transport_config(Arc::new(transport));
    Ok(config)
}

impl QuicClient {
    /// Connects in the background; packets sent before the handshake finishes wait for it.
    pub(crate) fn connect(server: SocketAddr, pin: [u8; 32], shared: Arc<Shared>) -> Self {
        let (tx, rx) = mpsc::unbounded_channel();
        let stats = Arc::new(Stats::default());
        let st = stats.clone();
        let alive = Arc::new(std::sync::atomic::AtomicBool::new(true));
        let al = alive.clone();
        std::thread::Builder::new()
            .name("relay quic".into())
            .spawn(move || {
                let rt = match tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                {
                    Ok(rt) => rt,
                    Err(_) => return,
                };
                rt.block_on(run(server, pin, shared, rx, st));
                al.store(false, std::sync::atomic::Ordering::Relaxed);
            })
            .ok();
        Self { tx, stats, alive }
    }

    /// The connection is still up (or connecting).
    pub fn is_alive(&self) -> bool {
        self.alive.load(std::sync::atomic::Ordering::Relaxed)
    }

    pub fn send(&self, packet: Vec<u8>) {
        self.tx.send(Cmd::Send(packet)).ok();
    }

    /// A link stream command (kept, in order, until the connection is up).
    pub(crate) fn link(&self, cmd: StreamCmd) {
        self.tx.send(Cmd::Link(cmd)).ok();
    }

    /// A trade stream command (kept, in order, until the connection is up).
    pub(crate) fn trade(&self, cmd: TradeCmd) {
        self.tx.send(Cmd::Trade(cmd)).ok();
    }

    /// Moves to a new local UDP port (a NAT rebinding). True once done.
    pub fn rebind(&self) -> bool {
        let (tx, rx) = std_mpsc::channel();
        self.tx.send(Cmd::Rebind(tx)).is_ok()
            && rx
                .recv_timeout(std::time::Duration::from_secs(5))
                .unwrap_or(false)
    }
}

async fn run(
    server: SocketAddr,
    pin: [u8; 32],
    shared: Arc<Shared>,
    mut rx: mpsc::UnboundedReceiver<Cmd>,
    stats: Arc<Stats>,
) {
    use std::sync::atomic::Ordering::Relaxed;
    let other = Arc::new(std::sync::Mutex::new(None));
    let Ok(config) = client_config(pin, other.clone()) else {
        return;
    };
    let bind: SocketAddr = if server.is_ipv4() {
        "0.0.0.0:0"
    } else {
        "[::]:0"
    }
    .parse()
    .unwrap();
    let Ok(mut endpoint) = quinn::Endpoint::client(bind) else {
        return;
    };
    endpoint.set_default_client_config(config);
    let Ok(connecting) = endpoint.connect(server, "goldlink-relay") else {
        return;
    };
    // Until connected, packets wait (the handshake is quick; the bridge resends every frame
    // anyway); link stream commands are kept, in order.
    let mut held = Vec::new();
    let conn = {
        tokio::pin!(connecting);
        loop {
            tokio::select! {
                c = &mut connecting => match c {
                    Ok(c) => break c,
                    Err(_) => {
                        // Another certificate than the pinned one: never played on (no fallback).
                        if let Some(seen) = *other.lock().unwrap_or_else(|e| e.into_inner()) {
                            *shared.quic_error.lock().unwrap_or_else(|e| e.into_inner()) =
                                Some(crate::net::QuicError::IdentityChanged(seen));
                        }
                        return;
                    }
                },
                cmd = rx.recv() => match cmd {
                    Some(Cmd::Link(l)) => held.push(Held::Link(l)),
                    Some(Cmd::Trade(t)) => held.push(Held::Trade(t)),
                    Some(_) => { stats.queued_before_connect.fetch_add(1, Relaxed); }
                    None => return,
                },
            }
        }
    };
    let mut link: Option<(u8, mpsc::UnboundedSender<Vec<u8>>)> = None;
    let mut trade: Option<mpsc::UnboundedSender<Vec<u8>>> = None;
    for h in held {
        match h {
            Held::Link(l) => link_cmd(&conn, &mut link, l, &shared).await,
            Held::Trade(t) => trade_cmd(&conn, &mut trade, t, &shared).await,
        }
    }
    loop {
        tokio::select! {
            cmd = rx.recv() => match cmd {
                Some(Cmd::Link(l)) => link_cmd(&conn, &mut link, l, &shared).await,
                Some(Cmd::Trade(t)) => trade_cmd(&conn, &mut trade, t, &shared).await,
                Some(Cmd::Send(p)) => send(&conn, p, &stats),
                Some(Cmd::Rebind(done)) => {
                    let ok = std::net::UdpSocket::bind(bind).and_then(|s| endpoint.rebind(s)).is_ok();
                    done.send(ok).ok();
                }
                None => {
                    conn.close(0u32.into(), b"bye");
                    return;
                }
            },
            d = conn.read_datagram() => match d {
                Ok(bytes) => {
                    stats.got_datagrams.fetch_add(1, Relaxed);
                    accept(&shared, &bytes)
                }
                Err(_) => return,
            },
            s = conn.accept_uni() => match s {
                Ok(mut s) => {
                    let shared = shared.clone();
                    stats.got_streams.fetch_add(1, Relaxed);
                    tokio::spawn(async move {
                        let mut len = [0u8; 2];
                        if s.read_exact(&mut len).await.is_err() { return; }
                        let mut buf = vec![0u8; u16::from_le_bytes(len) as usize];
                        if s.read_exact(&mut buf).await.is_ok() {
                            accept(&shared, &buf);
                        }
                    });
                }
                Err(_) => return,
            },
        }
    }
}

/// Stream commands that came before the connection was up.
enum Held {
    Link(StreamCmd),
    Trade(TradeCmd),
}

/// Opens, writes or finishes the trade stream. The relay's latest frame goes into
/// `shared.trade_down`.
async fn trade_cmd(
    conn: &quinn::Connection,
    trade: &mut Option<mpsc::UnboundedSender<Vec<u8>>>,
    cmd: TradeCmd,
    shared: &Arc<Shared>,
) {
    match cmd {
        TradeCmd::Open => {
            *trade = None;
            *shared.trade_down.lock().unwrap_or_else(|e| e.into_inner()) = None;
            let Ok((mut send, mut recv)) = conn.open_bi().await else {
                return;
            };
            let mut head = wire::TRADE_STREAM_MAGIC.to_le_bytes().to_vec();
            head.push(0);
            let (w_tx, mut w_rx) = mpsc::unbounded_channel::<Vec<u8>>();
            tokio::spawn(async move {
                if send.write_all(&head).await.is_err() {
                    return;
                }
                while let Some(frame) = w_rx.recv().await {
                    if send.write_all(&frame).await.is_err() {
                        return;
                    }
                }
                send.finish().ok();
            });
            let shared = shared.clone();
            tokio::spawn(async move {
                let mut frame = vec![0u8; wire::TRADE_STREAM_DOWN_SIZE];
                while recv.read_exact(&mut frame).await.is_ok() {
                    *shared.trade_down.lock().unwrap_or_else(|e| e.into_inner()) =
                        Some(frame.clone());
                }
            });
            *trade = Some(w_tx);
        }
        TradeCmd::Send(out) => {
            if let Some(w) = trade.as_ref() {
                w.send(out).ok();
            }
        }
        TradeCmd::Close => *trade = None,
    }
}

/// Opens, writes or finishes the link stream. Its frames come back into `shared.link_in`.
async fn link_cmd(
    conn: &quinn::Connection,
    link: &mut Option<(u8, mpsc::UnboundedSender<Vec<u8>>)>,
    cmd: StreamCmd,
    shared: &Arc<Shared>,
) {
    match cmd {
        StreamCmd::Open(epoch) => {
            *link = None; // an earlier block's writer finishes its stream
            let Ok((mut send, mut recv)) = conn.open_bi().await else {
                return;
            };
            let mut head = wire::LINK_STREAM_MAGIC.to_le_bytes().to_vec();
            head.push(epoch);
            let (w_tx, mut w_rx) = mpsc::unbounded_channel::<Vec<u8>>();
            tokio::spawn(async move {
                if send.write_all(&head).await.is_err() {
                    return;
                }
                while let Some(frame) = w_rx.recv().await {
                    if send.write_all(&frame).await.is_err() {
                        return;
                    }
                }
                send.finish().ok(); // the BYE
            });
            let shared = shared.clone();
            tokio::spawn(async move {
                loop {
                    let mut header = [0u8; 4];
                    if recv.read_exact(&mut header).await.is_err() {
                        return;
                    }
                    let Some(len) = wire::link_frame_len(header) else {
                        return;
                    };
                    let mut data = vec![0u8; len];
                    if recv.read_exact(&mut data).await.is_err() {
                        return;
                    }
                    let msg = wire::LinkMsg {
                        kind: header[0],
                        aid: header[1],
                        data,
                    };
                    shared
                        .link_in
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .push_back((epoch, msg));
                }
            });
            *link = Some((epoch, w_tx));
        }
        StreamCmd::Send(epoch, msg) => {
            if let Some((e, w)) = link.as_ref()
                && *e == epoch
            {
                w.send(wire::encode_link_frame(&msg)).ok();
            }
        }
        StreamCmd::Close(epoch) => {
            if link.as_ref().is_some_and(|(e, _)| *e == epoch) {
                *link = None;
            }
        }
    }
}

fn send(conn: &quinn::Connection, packet: Vec<u8>, stats: &Stats) {
    use std::sync::atomic::Ordering::Relaxed;
    if conn
        .max_datagram_size()
        .is_some_and(|max| packet.len() <= max)
    {
        match conn.send_datagram(packet.into()) {
            Ok(()) => stats.sent_datagrams.fetch_add(1, Relaxed),
            Err(_) => stats.send_failures.fetch_add(1, Relaxed),
        };
        return;
    }
    stats.sent_streams.fetch_add(1, Relaxed);
    let conn = conn.clone();
    tokio::spawn(async move {
        if let Ok(mut s) = conn.open_uni().await {
            let len = (packet.len() as u16).to_le_bytes();
            if s.write_all(&len).await.is_ok() && s.write_all(&packet).await.is_ok() {
                s.finish().ok();
            }
        }
    });
}
