//! The relay connection (the C# bridge's UdpClient, ReceiveLoop and SendPacket): one UDP socket,
//! a receive thread that keeps the latest server packet (latest wins, as the bridge's snapshot
//! semantics need) and the relay's return-routability cookie.

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
struct Shared {
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

pub struct RelayClient {
    socket: Arc<UdpSocket>,
    shared: Arc<Shared>,
    thread: Option<JoinHandle<()>>,
}

impl RelayClient {
    pub fn new() -> std::io::Result<Self> {
        let socket = UdpSocket::bind("0.0.0.0:0")?;
        socket.set_read_timeout(Some(Duration::from_millis(200)))?;
        Ok(Self {
            socket: Arc::new(socket),
            shared: Arc::new(Shared::default()),
            thread: None,
        })
    }

    /// Talks to `server` from now on (a new server forgets the old cookie) and starts listening.
    pub fn set_server(&mut self, server: SocketAddr) {
        let mut s = lock(&self.shared.server);
        if *s != Some(server) {
            self.shared.cookie.store(0, Ordering::Relaxed);
        }
        *s = Some(server);
        drop(s);
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
        if let Some(server) = *lock(&self.shared.server) {
            self.socket.send_to(packet, server).ok();
        }
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
        let data = &buf[..n];
        // Only the relay we're talking to, only this protocol (except its "other version"
        // refusal, which carries its own version).
        if Some(from) != *lock(&shared.server) || n < wire::ERROR_PACKET_SIZE {
            continue;
        }
        if u32_at(data, 0) != wire::COOP_MAGIC {
            continue;
        }
        let status = data[12];
        if u16_at(data, 4) != wire::PROTOCOL_VERSION && status != wire::ROOM_STATUS_ERR_PROTOCOL {
            continue;
        }
        if status == wire::ROOM_STATUS_CHALLENGE {
            if n >= wire::CHALLENGE_PACKET_SIZE {
                let cookie = u64::from_le_bytes(data[16..24].try_into().unwrap());
                shared.cookie.store(cookie, Ordering::Relaxed);
                *lock(&shared.last_challenge) = Some(Instant::now());
            }
            continue;
        }
        if status == wire::ROOM_STATUS_OK && n < wire::SERVER_PACKET_SIZE {
            continue;
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

/// `host:port` (port 7777 when left out) to an address: IPv4 first, as the C# bridge. DNS can
/// take seconds; call it off the emulation thread.
pub fn resolve(host_port: &str) -> Option<SocketAddr> {
    let text = host_port.trim();
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
        assert_eq!(
            resolve("127.0.0.1:7795"),
            Some("127.0.0.1:7795".parse().unwrap())
        );
        assert_eq!(
            resolve("127.0.0.1"),
            Some("127.0.0.1:7777".parse().unwrap())
        );
        assert_eq!(resolve(" localhost:1234 ").map(|a| a.port()), Some(1234));
        assert_eq!(resolve("127.0.0.1:0"), None);
        assert_eq!(resolve("127.0.0.1:99999"), None);
        assert_eq!(resolve(""), None);
    }
}
