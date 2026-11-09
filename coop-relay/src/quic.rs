//! The relay over QUIC (unified-client UC6, RFC §5): an optional second listener carrying the
//! same packets as UDP, encrypted and authenticated (TLS 1.3), with connection migration (a
//! player whose address changes keeps their seat). The lobby doesn't know the difference: each
//! QUIC connection gets a stable stand-in address (`fd00::<n>`, port 1) that it sees instead of
//! the player's real one.
//!
//! Packets travel as QUIC datagrams when they fit the path, else as a frame on a one-way stream
//! (u16 length, then the bytes). The certificate is self-signed, made on the first start and
//! kept in the save directory; players pin it by its SHA-256 fingerprint in the server address
//! (`host:port#<fingerprint>`), shown at startup.
//!
//! Protocol 9 (dev/docs/LINK_BATTLE_REWORK.md §7.1): a player's link messages travel on one
//! bidirectional stream per link block (`LINK_STREAM_MAGIC`, then the epoch, then frames both
//! ways), QUIC's retransmission and ordering instead of the trailer's go-back-N.
//!
//! By default QUIC shares the relay's UDP port (`start_shared`): QUIC owns the socket, and every
//! datagram that starts with the relay's own packet marker (`COOP_MAGIC`) goes to the relay
//! instead. No QUIC packet sent to the relay can start with it: long headers have the top bit
//! set, and short headers start with a connection ID this relay picked (`NoMarkerCids`).

use std::collections::HashMap;
use std::io::{self, IoSliceMut};
use std::net::{Ipv6Addr, SocketAddr, SocketAddrV6};
use std::path::Path;
use std::pin::Pin;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll, ready};

use quinn::udp::{RecvMeta, Transmit};
use quinn::{
    AsyncUdpSocket, Connection, ConnectionId, ConnectionIdGenerator, Endpoint, EndpointConfig,
    RecvStream, Runtime, SendStream, TokioRuntime, UdpPoller,
};
use rustls::crypto::SecureRandom;
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
use sha2::{Digest, Sha256};
use tokio::sync::mpsc;
use tracing::{debug, info};

use crate::lobby::{LinkStreamEvent, TradeStreamEvent};
use crate::protocol::{
    COOP_MAGIC, LINK_STREAM_MAGIC, LinkMsg, TRADE_OUT_SIZE, TRADE_STREAM_MAGIC, TradeOut,
    encode_link_frame, link_frame_len,
};

/// ALPN: the relay protocol over QUIC (the packets inside carry their own version).
pub const ALPN: &[u8] = b"goldlink-relay";
const CERT_FILE: &str = "relay-quic-cert.der";
const KEY_FILE: &str = "relay-quic-key.der";

/// A stand-in address names a QUIC connection to the lobby.
pub fn is_virtual(addr: &SocketAddr) -> bool {
    matches!(addr, SocketAddr::V6(v6) if v6.ip().segments()[0] == 0xfd00 && v6.port() == 1)
}

fn virtual_addr(n: u64) -> SocketAddr {
    let s = [
        0xfd00,
        0,
        0,
        0,
        (n >> 48) as u16,
        (n >> 32) as u16,
        (n >> 16) as u16,
        n as u16,
    ];
    SocketAddr::V6(SocketAddrV6::new(
        Ipv6Addr::new(s[0], s[1], s[2], s[3], s[4], s[5], s[6], s[7]),
        1,
        0,
        0,
    ))
}

/// The certificate's SHA-256: what players pin.
pub fn fingerprint(cert: &[u8]) -> [u8; 32] {
    Sha256::digest(cert).into()
}

pub fn fingerprint_hex(fp: &[u8; 32]) -> String {
    fp.iter().map(|b| format!("{b:02x}")).collect()
}

/// The relay's certificate and key: kept in `dir`, made on the first start.
pub fn load_or_create_cert(
    dir: &Path,
) -> std::io::Result<(CertificateDer<'static>, PrivateKeyDer<'static>)> {
    let (cert_path, key_path) = (dir.join(CERT_FILE), dir.join(KEY_FILE));
    if let (Ok(c), Ok(k)) = (std::fs::read(&cert_path), std::fs::read(&key_path)) {
        return Ok((
            CertificateDer::from(c),
            PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(k)),
        ));
    }
    let made = rcgen::generate_simple_self_signed(vec!["goldlink-relay".to_string()])
        .map_err(|e| std::io::Error::other(e.to_string()))?;
    let cert = made.cert.der().to_vec();
    let key = made.key_pair.serialize_der();
    std::fs::create_dir_all(dir)?;
    std::fs::write(&cert_path, &cert)?;
    std::fs::write(&key_path, &key)?;
    Ok((
        CertificateDer::from(cert),
        PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(key)),
    ))
}

/// How long a quiet connection lives: past the relay's link deadline (30 s), its slot hold and
/// the bridge's 65 s, so a paused player's link stream outlives the referee's decision.
pub const IDLE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(75);

/// A relay packet (not QUIC) on the shared port: it starts with the relay's marker.
pub fn is_relay_packet(data: &[u8]) -> bool {
    data.len() >= 4 && data[..4] == COOP_MAGIC.to_le_bytes()
}

/// A datagram for the relay itself, from the shared port: (sender, bytes).
pub type PlainPacket = (SocketAddr, Vec<u8>);

/// Connection IDs that never start with the marker's last three bytes. A short-header QUIC
/// packet is one byte (which can be the marker's first, `P`) and then the connection ID the
/// relay picked, so on the shared port it never reads as a relay packet.
struct NoMarkerCids {
    rng: &'static dyn SecureRandom,
}

const CID_LEN: usize = 8;

impl ConnectionIdGenerator for NoMarkerCids {
    fn generate_cid(&mut self) -> ConnectionId {
        let marker = COOP_MAGIC.to_le_bytes();
        loop {
            let mut b = [0u8; CID_LEN];
            self.rng
                .fill(&mut b)
                .expect("the system's random number generator failed");
            if b[..3] != marker[1..] {
                return ConnectionId::new(&b);
            }
        }
    }

    fn cid_len(&self) -> usize {
        CID_LEN
    }

    fn cid_lifetime(&self) -> Option<std::time::Duration> {
        None
    }
}

fn endpoint_config() -> EndpointConfig {
    let mut config = EndpointConfig::default();
    config.cid_generator(|| {
        Box::new(NoMarkerCids {
            rng: rustls::crypto::ring::default_provider().secure_random,
        })
    });
    config
}

/// The shared port's socket as QUIC sees it: the real one, minus the relay's own packets, which
/// go to `plain` (dropped if the relay is that far behind, as a full socket buffer would).
#[derive(Debug)]
struct SharedSocket {
    inner: Arc<dyn AsyncUdpSocket>,
    plain: mpsc::Sender<PlainPacket>,
}

impl AsyncUdpSocket for SharedSocket {
    fn create_io_poller(self: Arc<Self>) -> Pin<Box<dyn UdpPoller>> {
        self.inner.clone().create_io_poller()
    }

    fn try_send(&self, transmit: &Transmit) -> io::Result<()> {
        self.inner.try_send(transmit)
    }

    fn poll_recv(
        &self,
        cx: &mut Context,
        bufs: &mut [IoSliceMut<'_>],
        meta: &mut [RecvMeta],
    ) -> Poll<io::Result<usize>> {
        loop {
            let n = ready!(self.inner.poll_recv(cx, bufs, meta))?;
            let mut kept = 0;
            for i in 0..n {
                let m = meta[i];
                let data = &bufs[i][..m.len];
                if is_relay_packet(data) {
                    // Coalesced datagrams (GRO) come from one sender, `stride` bytes each.
                    let stride = if m.stride == 0 { m.len } else { m.stride };
                    for d in data.chunks(stride) {
                        let _ = self.plain.try_send((m.addr, d.to_vec()));
                    }
                    continue;
                }
                if kept != i {
                    let (head, tail) = bufs.split_at_mut(i);
                    if head[kept].len() < m.len {
                        continue;
                    }
                    head[kept][..m.len].copy_from_slice(&tail[0][..m.len]);
                    meta[kept] = m;
                }
                kept += 1;
            }
            if kept > 0 {
                return Poll::Ready(Ok(kept));
            }
        }
    }

    fn local_addr(&self) -> io::Result<SocketAddr> {
        self.inner.local_addr()
    }

    fn max_transmit_segments(&self) -> usize {
        self.inner.max_transmit_segments()
    }

    fn max_receive_segments(&self) -> usize {
        self.inner.max_receive_segments()
    }

    fn may_fragment(&self) -> bool {
        self.inner.may_fragment()
    }
}

/// On a shared port: the relay's own packets, and a handle on that port to send them.
pub struct SharedPort {
    pub plain: mpsc::Receiver<PlainPacket>,
    pub socket: tokio::net::UdpSocket,
}

/// What a QUIC connection did, under its stand-in address.
#[derive(Debug)]
pub enum Incoming {
    Packet(SocketAddr, Vec<u8>),
    Link(SocketAddr, LinkStreamEvent),
    Trade(SocketAddr, TradeStreamEvent),
}

struct Conn {
    conn: Connection,
    /// The current link stream's epoch and its writer.
    link: Option<(u8, mpsc::UnboundedSender<Vec<u8>>)>,
    /// The current trade stream's writer.
    trade: Option<mpsc::UnboundedSender<Vec<u8>>>,
}

/// TLS 1.3 with the relay's certificate, and the transport settings for real-time packets.
fn server_config(
    save_dir: &Path,
) -> Result<(quinn::ServerConfig, [u8; 32]), Box<dyn std::error::Error>> {
    let (cert, key) = load_or_create_cert(save_dir)?;
    let fp = fingerprint(&cert);
    let mut tls = rustls::ServerConfig::builder_with_provider(Arc::new(
        rustls::crypto::ring::default_provider(),
    ))
    .with_protocol_versions(&[&rustls::version::TLS13])?
    .with_no_client_auth()
    .with_single_cert(vec![cert], key)?;
    tls.alpn_protocols = vec![ALPN.to_vec()];
    let mut config = quinn::ServerConfig::with_crypto(Arc::new(
        quinn::crypto::rustls::QuicServerConfig::try_from(tls)?,
    ));
    let mut transport = quinn::TransportConfig::default();
    transport.max_idle_timeout(Some(IDLE_TIMEOUT.try_into()?));
    transport.datagram_receive_buffer_size(Some(1 << 20));
    // Real-time traffic: a stale packet is worth less than a fresh one. A small send buffer
    // drops the oldest instead of queueing seconds behind the congestion window, and BBR
    // doesn't read random loss (Wi-Fi, the internet) as congestion the way the default does.
    transport.datagram_send_buffer_size(16 * 1024);
    transport.congestion_controller_factory(Arc::new(quinn::congestion::BbrConfig::default()));
    config.transport_config(Arc::new(transport));
    Ok((config, fp))
}

/// A started QUIC listener: the server, what its connections did, its certificate's
/// fingerprint, and on a shared port the relay's own packets and socket.
pub struct Started {
    pub server: QuicServer,
    pub incoming: mpsc::Receiver<Incoming>,
    pub fingerprint: [u8; 32],
    pub shared: Option<SharedPort>,
}

#[derive(Clone)]
pub struct QuicServer {
    conns: Arc<Mutex<HashMap<SocketAddr, Conn>>>,
}

impl QuicServer {
    /// Listens on its own port, `bind`. Packets arrive on `Started::incoming`.
    pub fn start(
        bind: SocketAddr,
        save_dir: &Path,
        max_packet: usize,
    ) -> Result<Started, Box<dyn std::error::Error>> {
        let (config, fp) = server_config(save_dir)?;
        let endpoint = Endpoint::new(
            endpoint_config(),
            Some(config),
            std::net::UdpSocket::bind(bind)?,
            Arc::new(TokioRuntime),
        )?;
        let (server, incoming) = Self::accept(endpoint, max_packet);
        Ok(Started {
            server,
            incoming,
            fingerprint: fp,
            shared: None,
        })
    }

    /// Listens on the relay's own UDP port, `bind`, shared with the relay's packets (the
    /// default): those come back in `Started::shared`.
    pub fn start_shared(
        bind: SocketAddr,
        save_dir: &Path,
        max_packet: usize,
    ) -> Result<Started, Box<dyn std::error::Error>> {
        let (config, fp) = server_config(save_dir)?;
        let std_socket = std::net::UdpSocket::bind(bind)?;
        // A second handle on the same socket, for the relay's sends; it never reads.
        let send = std_socket.try_clone()?;
        send.set_nonblocking(true)?;
        let socket = tokio::net::UdpSocket::from_std(send)?;
        let (plain_tx, plain) = mpsc::channel(4096);
        let shared = SharedSocket {
            inner: TokioRuntime.wrap_udp_socket(std_socket)?,
            plain: plain_tx,
        };
        let endpoint = Endpoint::new_with_abstract_socket(
            endpoint_config(),
            Some(config),
            Arc::new(shared),
            Arc::new(TokioRuntime),
        )?;
        let (server, incoming) = Self::accept(endpoint, max_packet);
        Ok(Started {
            server,
            incoming,
            fingerprint: fp,
            shared: Some(SharedPort { plain, socket }),
        })
    }

    fn accept(endpoint: Endpoint, max_packet: usize) -> (Self, mpsc::Receiver<Incoming>) {
        let (tx, rx) = mpsc::channel(4096);
        let server = Self {
            conns: Arc::default(),
        };
        let conns = server.conns.clone();
        tokio::spawn(async move {
            let next = AtomicU64::new(1);
            while let Some(incoming) = endpoint.accept().await {
                let addr = virtual_addr(next.fetch_add(1, Ordering::Relaxed));
                let (conns, tx) = (conns.clone(), tx.clone());
                tokio::spawn(async move {
                    let Ok(conn) = incoming.await else { return };
                    info!("QUIC connection from {} as {addr}", conn.remote_address());
                    conns.lock().unwrap().insert(
                        addr,
                        Conn {
                            conn: conn.clone(),
                            link: None,
                            trade: None,
                        },
                    );
                    serve(conn.clone(), addr, tx, max_packet, conns.clone()).await;
                    conns.lock().unwrap().remove(&addr);
                    debug!("QUIC connection {addr} closed");
                });
            }
        });
        (server, rx)
    }

    /// Sends a packet to a QUIC player (dropped if the connection is gone).
    pub fn send(&self, addr: SocketAddr, payload: Vec<u8>) {
        let Some(conn) = self
            .conns
            .lock()
            .unwrap()
            .get(&addr)
            .map(|c| c.conn.clone())
        else {
            return;
        };
        if conn
            .max_datagram_size()
            .is_some_and(|max| payload.len() <= max)
        {
            if conn.send_datagram(payload.into()).is_ok() {
                return;
            }
            return;
        }
        tokio::spawn(async move {
            if let Ok(mut s) = conn.open_uni().await {
                let len = (payload.len() as u16).to_le_bytes();
                if s.write_all(&len).await.is_ok() && s.write_all(&payload).await.is_ok() {
                    s.finish().ok();
                }
            }
        });
    }
}

impl QuicServer {
    /// A link message for a player's link stream of block `epoch` (dropped if that stream is
    /// gone: a newer block's, or the connection's end).
    pub fn send_link(&self, addr: SocketAddr, epoch: u8, msg: &LinkMsg) {
        if let Some(Conn {
            link: Some((e, w)), ..
        }) = self.conns.lock().unwrap().get(&addr)
            && *e == epoch
        {
            w.send(encode_link_frame(msg)).ok();
        }
    }
}

impl QuicServer {
    /// A trade frame for a player's trade stream (dropped if none is open).
    pub fn send_trade(&self, addr: SocketAddr, frame: Vec<u8>) {
        if let Some(Conn { trade: Some(w), .. }) = self.conns.lock().unwrap().get(&addr) {
            w.send(frame).ok();
        }
    }
}

type Conns = Arc<Mutex<HashMap<SocketAddr, Conn>>>;

/// A stream's writer task: frames from the channel, in order, until it closes (then FIN).
fn spawn_writer(mut send: SendStream) -> mpsc::UnboundedSender<Vec<u8>> {
    let (w_tx, mut w_rx) = mpsc::unbounded_channel::<Vec<u8>>();
    tokio::spawn(async move {
        while let Some(frame) = w_rx.recv().await {
            if send.write_all(&frame).await.is_err() {
                return;
            }
        }
        send.finish().ok();
    });
    w_tx
}

/// A trade stream: one per trade exchange. Up frames are the ROM's TradeOut.
async fn serve_trade(
    addr: SocketAddr,
    send: SendStream,
    mut recv: RecvStream,
    tx: mpsc::Sender<Incoming>,
    conns: Conns,
) {
    let w_tx = spawn_writer(send);
    if let Some(c) = conns.lock().unwrap().get_mut(&addr) {
        c.trade = Some(w_tx);
    }
    tx.send(Incoming::Trade(addr, TradeStreamEvent::Open))
        .await
        .ok();
    loop {
        let mut frame = vec![0u8; TRADE_OUT_SIZE];
        match recv.read_exact(&mut frame).await {
            Ok(()) => {
                let out = TradeOut::decode_le(&frame);
                tx.send(Incoming::Trade(addr, TradeStreamEvent::Up(Box::new(out))))
                    .await
                    .ok();
            }
            Err(_) => {
                // Finished (the exchange closed) or lost: no trade from this stream any more.
                tx.send(Incoming::Trade(addr, TradeStreamEvent::Closed))
                    .await
                    .ok();
                return;
            }
        }
    }
}

/// A link stream: its writer becomes the connection's, its frames events, until it ends.
async fn serve_link(
    addr: SocketAddr,
    epoch: u8,
    send: SendStream,
    mut recv: RecvStream,
    tx: mpsc::Sender<Incoming>,
    conns: Conns,
) {
    let w_tx = spawn_writer(send);
    if let Some(c) = conns.lock().unwrap().get_mut(&addr) {
        c.link = Some((epoch, w_tx)); // an earlier block's writer ends with its sender
    }
    tx.send(Incoming::Link(addr, LinkStreamEvent::Open(epoch)))
        .await
        .ok();
    loop {
        let mut header = [0u8; 4];
        match recv.read_exact(&mut header).await {
            Ok(()) => {}
            // Finished by the client between frames: its BYE. Anything else (the connection
            // lost, a reset) is silence, for the referee.
            Err(quinn::ReadExactError::FinishedEarly(0)) => {
                tx.send(Incoming::Link(addr, LinkStreamEvent::Closed(epoch)))
                    .await
                    .ok();
                return;
            }
            Err(_) => return,
        }
        let Some(len) = link_frame_len(header) else {
            return;
        };
        let mut data = vec![0u8; len];
        if recv.read_exact(&mut data).await.is_err() {
            return;
        }
        let msg = LinkMsg {
            kind: header[0],
            aid: header[1],
            data,
        };
        tx.send(Incoming::Link(addr, LinkStreamEvent::Msg(epoch, msg)))
            .await
            .ok();
    }
}

/// One connection's packets, from datagrams and streams, until it closes.
async fn serve(
    conn: Connection,
    addr: SocketAddr,
    tx: mpsc::Sender<Incoming>,
    max_packet: usize,
    conns: Conns,
) {
    loop {
        tokio::select! {
            d = conn.read_datagram() => match d {
                Ok(bytes) if bytes.len() <= max_packet => { tx.send(Incoming::Packet(addr, bytes.to_vec())).await.ok(); }
                Ok(_) => {}
                Err(_) => return,
            },
            b = conn.accept_bi() => match b {
                Ok((send, mut recv)) => {
                    let (tx, conns) = (tx.clone(), conns.clone());
                    tokio::spawn(async move {
                        let mut head = [0u8; 5];
                        if recv.read_exact(&mut head).await.is_err() { return; }
                        match u32::from_le_bytes([head[0], head[1], head[2], head[3]]) {
                            LINK_STREAM_MAGIC => serve_link(addr, head[4], send, recv, tx, conns).await,
                            TRADE_STREAM_MAGIC => serve_trade(addr, send, recv, tx, conns).await,
                            _ => {}
                        }
                    });
                }
                Err(_) => return,
            },
            s = conn.accept_uni() => match s {
                Ok(mut s) => {
                    let tx = tx.clone();
                    tokio::spawn(async move {
                        let mut len = [0u8; 2];
                        if s.read_exact(&mut len).await.is_err() { return; }
                        let len = u16::from_le_bytes(len) as usize;
                        if len > max_packet { return; }
                        let mut buf = vec![0u8; len];
                        if s.read_exact(&mut buf).await.is_ok() {
                            tx.send(Incoming::Packet(addr, buf)).await.ok();
                        }
                    });
                }
                Err(_) => return,
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_shared_port_sorts_by_the_marker() {
        let mut relay = COOP_MAGIC.to_le_bytes().to_vec();
        relay.extend_from_slice(&[0; 12]);
        assert!(is_relay_packet(&relay));
        assert!(!is_relay_packet(&relay[..3]));
        // QUIC long headers have the top bit set; a short header is 0x40.. then our CID.
        assert!(!is_relay_packet(&[0xC0, b'O', b'O', b'C']));
        let mut cids = NoMarkerCids {
            rng: rustls::crypto::ring::default_provider().secure_random,
        };
        for _ in 0..10_000 {
            let cid = cids.generate_cid();
            assert_eq!(cid.len(), CID_LEN);
            let mut short = vec![b'P'];
            short.extend_from_slice(&cid);
            assert!(!is_relay_packet(&short));
        }
    }

    #[test]
    fn stand_in_addresses() {
        let a = virtual_addr(1);
        let b = virtual_addr(0x1_0000_0002);
        assert!(is_virtual(&a) && is_virtual(&b) && a != b);
        assert!(!is_virtual(&"127.0.0.1:7777".parse().unwrap()));
        assert!(!is_virtual(&"[fd00::1]:7777".parse().unwrap()));
    }

    #[test]
    fn the_certificate_is_kept() {
        let dir = std::env::temp_dir().join(format!("relay-quic-cert-{}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();
        let (a, _) = load_or_create_cert(&dir).unwrap();
        let (b, _) = load_or_create_cert(&dir).unwrap();
        assert_eq!(fingerprint(&a), fingerprint(&b));
        assert_eq!(fingerprint_hex(&fingerprint(&a)).len(), 64);
        std::fs::remove_dir_all(&dir).ok();
    }
}
