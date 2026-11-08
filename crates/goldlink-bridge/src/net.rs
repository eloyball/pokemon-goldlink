//! The relay connection (the C# bridge's UdpClient, ReceiveLoop and SendPacket): one UDP socket,
//! a receive thread that keeps the latest server packet (latest wins, as the bridge's snapshot
//! semantics need) and the relay's return-routability cookie. A server address with a
//! certificate fingerprint (`host:port#<sha256>`) goes over QUIC instead (`quic.rs`, UC6): the
//! same packets, encrypted, the relay's certificate pinned.

use std::collections::VecDeque;
use std::net::{SocketAddr, ToSocketAddrs, UdpSocket};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use goldlink_proto as wire;

use crate::layout::{u16_at, u32_at};

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

#[derive(Default)]
pub(crate) struct Shared {
    server: Mutex<Option<SocketAddr>>,
    cookie: AtomicU64,
    last_challenge: Mutex<Option<Instant>>,
    /// The latest packet that isn't a challenge, with the session it arrived in.
    latest: Mutex<Option<(Vec<u8>, u32)>>,
    session: AtomicU32,
    running: AtomicBool,
    /// Every OK packet, for the virtual link (each link trailer counts; UC4).
    ok_packets: Mutex<VecDeque<Vec<u8>>>,
}

/// Where the relay is, and how to reach it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Target {
    pub addr: SocketAddr,
    /// The relay certificate's SHA-256: QUIC, pinned. None: plain UDP.
    pub pin: Option<[u8; 32]>,
}

pub struct RelayClient {
    socket: Arc<UdpSocket>,
    shared: Arc<Shared>,
    thread: Option<JoinHandle<()>>,
    quic: Option<(Target, crate::quic::QuicClient)>,
}

impl RelayClient {
    pub fn new() -> std::io::Result<Self> {
        let socket = UdpSocket::bind("0.0.0.0:0")?;
        socket.set_read_timeout(Some(Duration::from_millis(200)))?;
        Ok(Self {
            socket: Arc::new(socket),
            shared: Arc::new(Shared::default()),
            thread: None,
            quic: None,
        })
    }

    /// Talks to `target` from now on (a new server forgets the old cookie) and starts listening.
    pub fn set_server(&mut self, target: Target) {
        let server = target.addr;
        let mut s = lock(&self.shared.server);
        if *s != Some(server) {
            self.shared.cookie.store(0, Ordering::Relaxed);
        }
        *s = Some(server);
        drop(s);
        match target.pin {
            Some(pin) => {
                if self.quic.as_ref().is_none_or(|(t, _)| *t != target) {
                    self.quic = Some((
                        target,
                        crate::quic::QuicClient::connect(server, pin, self.shared.clone()),
                    ));
                }
            }
            None => self.quic = None,
        }
        if self.thread.is_none() {
            self.shared.running.store(true, Ordering::Relaxed);
            let (sock, shared) = (self.socket.clone(), self.shared.clone());
            self.thread = std::thread::Builder::new()
                .name("relay receive".into())
                .spawn(move || receive_loop(&sock, &shared))
                .ok();
        }
    }

    /// A new connection attempt: packets from before it are dropped.
    pub fn begin_session(&self) {
        self.shared.session.fetch_add(1, Ordering::Relaxed);
        *lock(&self.shared.latest) = None;
        lock(&self.shared.ok_packets).clear();
    }

    /// The latest packet of this session, taken.
    pub fn take_latest(&self) -> Option<Vec<u8>> {
        let session = self.shared.session.load(Ordering::Relaxed);
        lock(&self.shared.latest)
            .take()
            .and_then(|(p, s)| (s == session).then_some(p))
    }

    /// Every OK packet since the last call (the link trailers).
    pub fn take_ok_packets(&self) -> Vec<Vec<u8>> {
        lock(&self.shared.ok_packets).drain(..).collect()
    }

    pub fn cookie(&self) -> u64 {
        self.shared.cookie.load(Ordering::Relaxed)
    }

    pub fn last_challenge(&self) -> Option<Instant> {
        *lock(&self.shared.last_challenge)
    }

    pub fn send(&self, packet: &[u8]) {
        if let Some((_, q)) = &self.quic {
            q.send(packet.to_vec());
        } else if let Some(server) = *lock(&self.shared.server) {
            self.socket.send_to(packet, server).ok();
        }
    }

    /// QUIC only: moves to a new local UDP port, as a NAT rebinding or a network change would.
    /// The connection (and the seat in the world) carries on. False over plain UDP.
    pub fn rebind(&self) -> bool {
        self.quic.as_ref().is_some_and(|(_, q)| q.rebind())
    }

    /// Talking to the relay over QUIC.
    pub fn is_quic(&self) -> bool {
        self.quic.is_some()
    }
}

impl Drop for RelayClient {
    fn drop(&mut self) {
        self.shared.running.store(false, Ordering::Relaxed);
        if let Some(t) = self.thread.take() {
            t.join().ok();
        }
    }
}

fn receive_loop(socket: &UdpSocket, shared: &Shared) {
    let mut buf = vec![0u8; 4096];
    while shared.running.load(Ordering::Relaxed) {
        let (n, from) = match socket.recv_from(&mut buf) {
            Ok(r) => r,
            // Timeouts, and ICMP "port unreachable" (relay not up yet): keep listening.
            Err(_) => continue,
        };
        // Only the relay we're talking to.
        if Some(from) != *lock(&shared.server) {
            continue;
        }
        accept(shared, &buf[..n]);
    }
}

/// A packet from the relay (UDP or QUIC): the cookie, or the latest packet and the link's.
pub(crate) fn accept(shared: &Shared, data: &[u8]) {
    let n = data.len();
    {
        // Only this protocol (except the relay's "other version" refusal, which carries its
        // own version).
        if n < wire::ERROR_PACKET_SIZE || u32_at(data, 0) != wire::COOP_MAGIC {
            return;
        }
        let status = data[12];
        if u16_at(data, 4) != wire::PROTOCOL_VERSION && status != wire::ROOM_STATUS_ERR_PROTOCOL {
            return;
        }
        if status == wire::ROOM_STATUS_CHALLENGE {
            if n >= wire::CHALLENGE_PACKET_SIZE {
                let cookie = u64::from_le_bytes(data[16..24].try_into().unwrap());
                shared.cookie.store(cookie, Ordering::Relaxed);
                *lock(&shared.last_challenge) = Some(Instant::now());
            }
            return;
        }
        if status == wire::ROOM_STATUS_OK && n < wire::SERVER_PACKET_SIZE {
            return;
        }
        let session = shared.session.load(Ordering::Relaxed);
        *lock(&shared.latest) = Some((data.to_vec(), session));
        if status == wire::ROOM_STATUS_OK {
            let mut q = lock(&shared.ok_packets);
            q.push_back(data.to_vec());
            while q.len() > 256 {
                q.pop_front();
            }
        }
    }
}

/// `host:port` (port 7777 when left out) to an address: IPv4 first, as the C# bridge. A
/// `#<64 hex digits>` suffix is the relay's QUIC certificate fingerprint. DNS can take seconds;
/// call it off the emulation thread.
pub fn resolve(host_port: &str) -> Option<Target> {
    let text = host_port.trim();
    let (text, pin) = match text.split_once('#') {
        Some((t, fp)) => (t.trim(), Some(parse_pin(fp.trim())?)),
        None => (text, None),
    };
    resolve_addr(text).map(|addr| Target { addr, pin })
}

fn parse_pin(hex: &str) -> Option<[u8; 32]> {
    if hex.len() != 64 {
        return None;
    }
    let mut out = [0u8; 32];
    for (i, b) in out.iter_mut().enumerate() {
        *b = u8::from_str_radix(hex.get(i * 2..i * 2 + 2)?, 16).ok()?;
    }
    Some(out)
}

fn resolve_addr(text: &str) -> Option<SocketAddr> {
    if text.is_empty() {
        return None;
    }
    let (host, port) = match text.rsplit_once(':') {
        Some((h, p)) => (h.trim(), p.trim().parse::<u16>().ok().filter(|&p| p > 0)?),
        None => (text, 7777),
    };
    let addrs: Vec<SocketAddr> = (host, port).to_socket_addrs().ok()?.collect();
    addrs
        .iter()
        .find(|a| a.is_ipv4())
        .or(addrs.first())
        .copied()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_forms() {
        let udp = |a: &str| {
            Some(Target {
                addr: a.parse().unwrap(),
                pin: None,
            })
        };
        assert_eq!(resolve("127.0.0.1:7795"), udp("127.0.0.1:7795"));
        assert_eq!(resolve("127.0.0.1"), udp("127.0.0.1:7777"));
        assert_eq!(
            resolve(" localhost:1234 ").map(|t| t.addr.port()),
            Some(1234)
        );
        let fp = "00112233445566778899aabbccddeeff00112233445566778899AABBCCDDEEFF";
        let t = resolve(&format!("127.0.0.1:7778#{fp}")).unwrap();
        assert_eq!(
            (t.addr.port(), t.pin.unwrap()[1], t.pin.unwrap()[31]),
            (7778, 0x11, 0xFF)
        );
        assert_eq!(
            resolve("127.0.0.1:7778#abc"),
            None,
            "a cut fingerprint is refused, never UDP"
        );
        assert_eq!(resolve("127.0.0.1:0"), None);
        assert_eq!(resolve("127.0.0.1:99999"), None);
        assert_eq!(resolve(""), None);
    }
}
