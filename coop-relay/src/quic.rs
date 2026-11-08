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

use std::collections::HashMap;
use std::net::{Ipv6Addr, SocketAddr, SocketAddrV6};
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use quinn::{Connection, Endpoint};
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
use sha2::{Digest, Sha256};
use tokio::sync::mpsc;
use tracing::{debug, info};

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

pub fn fingerprint(cert: &[u8]) -> String {
    Sha256::digest(cert)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
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

/// What a QUIC connection sent: a packet, under its stand-in address.
pub type Incoming = (SocketAddr, Vec<u8>);

#[derive(Clone)]
pub struct QuicServer {
    conns: Arc<Mutex<HashMap<SocketAddr, Connection>>>,
}

impl QuicServer {
    /// Listens on `bind`. Packets arrive on the returned channel.
    pub fn start(
        bind: SocketAddr,
        save_dir: &Path,
        max_packet: usize,
    ) -> Result<(Self, mpsc::Receiver<Incoming>, String), Box<dyn std::error::Error>> {
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
        transport.max_idle_timeout(Some(std::time::Duration::from_secs(10).try_into()?));
        transport.datagram_receive_buffer_size(Some(1 << 20));
        config.transport_config(Arc::new(transport));
        let endpoint = Endpoint::server(config, bind)?;
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
                    conns.lock().unwrap().insert(addr, conn.clone());
                    serve(conn.clone(), addr, tx, max_packet).await;
                    conns.lock().unwrap().remove(&addr);
                    debug!("QUIC connection {addr} closed");
                });
            }
        });
        Ok((server, rx, fp))
    }

    /// Sends a packet to a QUIC player (dropped if the connection is gone).
    pub fn send(&self, addr: SocketAddr, payload: Vec<u8>) {
        let Some(conn) = self.conns.lock().unwrap().get(&addr).cloned() else {
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

/// One connection's packets, from datagrams and streams, until it closes.
async fn serve(conn: Connection, addr: SocketAddr, tx: mpsc::Sender<Incoming>, max_packet: usize) {
    loop {
        tokio::select! {
            d = conn.read_datagram() => match d {
                Ok(bytes) if bytes.len() <= max_packet => { tx.send((addr, bytes.to_vec())).await.ok(); }
                Ok(_) => {}
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
                            tx.send((addr, buf)).await.ok();
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
        assert_eq!(fingerprint(&a).len(), 64);
        std::fs::remove_dir_all(&dir).ok();
    }
}
