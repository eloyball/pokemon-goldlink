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

use crate::net::{Shared, accept};

pub const ALPN: &[u8] = b"goldlink-relay";

enum Cmd {
    Send(Vec<u8>),
    Rebind(std_mpsc::Sender<bool>),
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
}

/// Accepts exactly the certificate with this SHA-256 (the relay's self-signed one).
#[derive(Debug)]
struct Pinned {
    pin: [u8; 32],
    provider: Arc<rustls::crypto::CryptoProvider>,
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
        if Sha256::digest(end_entity.as_ref()).as_slice() == self.pin {
            Ok(ServerCertVerified::assertion())
        } else {
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

fn client_config(pin: [u8; 32]) -> Result<quinn::ClientConfig, String> {
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let mut tls = rustls::ClientConfig::builder_with_provider(provider.clone())
        .with_protocol_versions(&[&rustls::version::TLS13])
        .map_err(|e| e.to_string())?
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(Pinned { pin, provider }))
        .with_no_client_auth();
    tls.alpn_protocols = vec![ALPN.to_vec()];
    let crypto =
        quinn::crypto::rustls::QuicClientConfig::try_from(tls).map_err(|e| e.to_string())?;
    let mut config = quinn::ClientConfig::new(Arc::new(crypto));
    let mut transport = quinn::TransportConfig::default();
    transport.keep_alive_interval(Some(std::time::Duration::from_secs(1)));
    transport.max_idle_timeout(Some(
        std::time::Duration::from_secs(10)
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
            })
            .ok();
        Self { tx, stats }
    }

    pub fn send(&self, packet: Vec<u8>) {
        self.tx.send(Cmd::Send(packet)).ok();
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
    let Ok(config) = client_config(pin) else {
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
    // Until connected, keep what the bridge sends (the handshake is quick; the bridge resends
    // every frame anyway, so the newest few are enough).
    let conn = tokio::select! {
        c = connecting => match c { Ok(c) => c, Err(_) => return },
        _ = async { while rx.recv().await.is_some() { stats.queued_before_connect.fetch_add(1, Relaxed); } } => return,
    };
    loop {
        tokio::select! {
            cmd = rx.recv() => match cmd {
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
