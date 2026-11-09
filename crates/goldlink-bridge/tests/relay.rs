//! Two Rust bridges in one world on the real relay, without a ROM: each "game" is a main RAM
//! with a published mailbox and a player standing in the overworld. Ignored by default (it needs
//! the relay binary):
//!
//!     cargo build --release --manifest-path coop-relay/Cargo.toml
//!     cargo test -p goldlink-bridge --test relay -- --ignored
//!
//! (`GOLDLINK_RELAY` points at another relay binary.)

use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use goldlink_bridge::layout::*;
use goldlink_bridge::{Bridge, LinkState, Machine};

const MB: usize = 0x3D8060;

struct FakeGame {
    ram: Vec<u8>,
    cart: Vec<u8>,
}

impl FakeGame {
    fn new(trainer: u32, map: u16) -> Self {
        let mut g = Self {
            ram: vec![0; 0x40_0000],
            cart: vec![0xFF; 0x8_0000],
        };
        let mb = &mut g.ram[MB..MB + MAILBOX_SIZE];
        put_u32(mb, MAGIC, goldlink_abi::COOP_MAGIC);
        mb[VERSION..VERSION + 2]
            .copy_from_slice(&(goldlink_abi::COOP_MAILBOX_VERSION as u16).to_le_bytes());
        mb[LIMIT_MAP_ID..LIMIT_MAP_ID + 2].copy_from_slice(&600u16.to_le_bytes());
        mb[LIMIT_SPECIES..LIMIT_SPECIES + 2].copy_from_slice(&1100u16.to_le_bytes());
        put_u32(mb, HEARTBEAT_ROM, 100);
        let lp = LOCAL_PLAYER;
        mb[lp + PS_MAP_ID..lp + PS_MAP_ID + 2].copy_from_slice(&map.to_le_bytes());
        mb[lp + PS_IS_ACTIVE] = 1;
        mb[lp + PS_ACTIVITY] = 1;
        mb[lp + PS_SAVE_STATUS] = 1;
        mb[lp + PS_MOVE_ACTION] = MOVE_ACTION_NONE;
        mb[lp + PS_FOLLOWER_MOVE_ACTION] = MOVE_ACTION_NONE;
        put_u32(mb, lp + PS_TRAINER_ID, trainer);
        g
    }

    fn mb(&self) -> &[u8] {
        &self.ram[MB..MB + MAILBOX_SIZE]
    }

    /// The ROM's side of a frame: its heartbeat moves.
    fn frame(&mut self) {
        let hb = u32_at(&self.ram, MB + HEARTBEAT_ROM) + 1;
        put_u32(&mut self.ram, MB + HEARTBEAT_ROM, hb);
    }
}

impl Machine for FakeGame {
    fn main_ram(&mut self) -> &mut [u8] {
        &mut self.ram
    }
    fn cart(&mut self) -> &mut [u8] {
        &mut self.cart
    }
    fn savestate(&mut self) -> Option<Vec<u8>> {
        Some(self.ram[..16].to_vec())
    }
    fn load_savestate(&mut self, _: &[u8]) -> Result<(), String> {
        Ok(())
    }
}

struct Relay(Child);

impl Drop for Relay {
    fn drop(&mut self) {
        self.0.kill().ok();
        self.0.wait().ok();
    }
}

fn start_relay(port: u16, dir: &std::path::Path) -> Relay {
    let bin = std::env::var_os("GOLDLINK_RELAY")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../../coop-relay/target/release/coop-relay")
        });
    let child = Command::new(&bin)
        .args(["--bind", &format!("127.0.0.1:{port}"), "--save-dir"])
        .arg(dir.join("campaigns"))
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap_or_else(|e| {
            panic!(
                "{}: {e} (build the relay, or set GOLDLINK_RELAY)",
                bin.display()
            )
        });
    std::thread::sleep(Duration::from_millis(300));
    Relay(child)
}

fn temp(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!(
        "goldlink-bridge-relay-{name}-{}",
        std::process::id()
    ));
    std::fs::remove_dir_all(&d).ok();
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// Runs both "games" at about 60 frames a second until `done` or the timeout.
fn run(
    pairs: &mut [(&mut Bridge, &mut FakeGame)],
    secs: u64,
    mut done: impl FnMut(&[(&mut Bridge, &mut FakeGame)]) -> bool,
) -> bool {
    let end = Instant::now() + Duration::from_secs(secs);
    while Instant::now() < end {
        for (b, g) in pairs.iter_mut() {
            g.frame();
            b.post_frame(*g, MB);
        }
        if done(pairs) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(16));
    }
    false
}

#[test]
#[ignore = "needs the coop-relay binary"]
fn two_bridges_share_a_world() {
    let port = 17_000 + (std::process::id() % 1000) as u16;
    let dir = temp("world");
    let _relay = start_relay(port, &dir);
    let server = format!("127.0.0.1:{port}");

    let (da, db) = (dir.join("a"), dir.join("b"));
    let mut a = Bridge::new(&da, &[]).unwrap();
    let mut b = Bridge::new(&db, &[]).unwrap();
    let (mut ga, mut gb) = (
        FakeGame::new(0x1111_2222, 100),
        FakeGame::new(0x3333_4444, 120),
    );

    // A creates the world (2 players), B joins once it exists.
    a.set_host_options(2, 0);
    a.join(&server, "RSTW01", true);
    assert!(
        run(&mut [(&mut a, &mut ga)], 10, |p| p[0].0.link()
            == LinkState::Connected),
        "A connects"
    );
    b.join(&server, "RSTW01", false);
    let both = run(&mut [(&mut a, &mut ga), (&mut b, &mut gb)], 10, |p| {
        p.iter().all(|(br, _)| br.link() == LinkState::Connected)
            // Each sees the other's trainer in a remote slot.
            && p.iter().enumerate().all(|(i, (_, g))| {
                let other = if i == 0 { 0x3333_4444 } else { 0x1111_2222 };
                (0..REMOTE_SLOTS).any(|s| {
                    u32_at(g.mb(), SERVER_PAYLOAD + s * PLAYER_STATE_SIZE + PS_TRAINER_ID) == other
                })
            })
    });
    assert!(both, "both connected and each sees the other");
    // The save slot is prepared on the frame after the first OK packet names the world.
    let end = Instant::now() + Duration::from_millis(300);
    run(&mut [(&mut a, &mut ga), (&mut b, &mut gb)], 1, |_| {
        Instant::now() > end
    });

    // What the ROM reads: connected, a slot, the save slot ready (a new world on a blank cart:
    // nothing to swap), the relay's gates (two players online).
    for (g, slot) in [(&ga, 0u8), (&gb, 1u8)] {
        let mb = g.mb();
        assert_eq!(mb[IS_CONNECTED_TO_ROOM], 1);
        assert_eq!(mb[ASSIGNED_PLAYER_ID], slot);
        assert_eq!(
            mb[SAVE_SLOT_READY],
            1,
            "slot {slot}: room_uid {:08X}, gates {:?}",
            u32_at(mb, GATES + GATE_ROOM_UID),
            &mb[GATES..GATES + GATES_SIZE]
        );
        assert_ne!(u32_at(mb, HEARTBEAT_BRIDGE), 0);
        assert_eq!(mb[GATES + GATE_ONLINE_PARTY], 2);
        assert_ne!(u32_at(mb, GATES + GATE_ROOM_UID), 0);
    }
    assert_eq!(a.worlds().entries()[0].code, "RSTW01");
    assert!(a.worlds().entries()[0].created);
    assert!(!b.worlds().entries()[0].created);
    assert_eq!(b.worlds().entries()[0].server, server);

    // A's game saves: the bridge keeps it for the world (save_counter moves).
    ga.cart.fill(0x5A);
    put_u32(&mut ga.ram, MB + SAVE_COUNTER, 1);
    run(&mut [(&mut a, &mut ga)], 1, |_| true);
    let uid = u32_at(ga.mb(), GATES + GATE_ROOM_UID);
    let file = da.join("coop_saves").join(format!("RSTW01-{uid:08X}.sav"));
    assert_eq!(std::fs::read(&file).unwrap(), vec![0x5A; 0x8_0000]);

    // B leaves: A's remote slot empties; B's game reads "not connected".
    b.leave();
    let gone = run(&mut [(&mut a, &mut ga), (&mut b, &mut gb)], 10, |p| {
        (0..REMOTE_SLOTS).all(|s| {
            u32_at(
                p[0].1.mb(),
                SERVER_PAYLOAD + s * PLAYER_STATE_SIZE + PS_TRAINER_ID,
            ) != 0x3333_4444
        })
    });
    assert!(gone, "A sees B leave");
    assert_eq!(gb.mb()[IS_CONNECTED_TO_ROOM], 0);
    assert_eq!(gb.mb()[ASSIGNED_PLAYER_ID], PLAYER_ID_NONE);
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
#[ignore = "needs the coop-relay binary"]
fn rejections_and_a_world_save_swap() {
    let port = 18_000 + (std::process::id() % 1000) as u16;
    let dir = temp("swap");
    let _relay = start_relay(port, &dir);
    let server = format!("127.0.0.1:{port}");
    let da = dir.join("a");

    // Joining a world that doesn't exist: the relay's message, offline again.
    let mut a = Bridge::new(&da, &[]).unwrap();
    let mut ga = FakeGame::new(0xAAAA_0001, 100);
    a.join(&server, "NOPE01", false);
    assert!(run(&mut [(&mut a, &mut ga)], 10, |p| p[0].0.link()
        == LinkState::Offline
        && p[0].0.view().message.contains("doesn't exist")));

    // The cart holds world X's save (owner X); creating world Y swaps a blank save in and asks
    // the ROM to restart; X's save is kept in its file.
    std::fs::create_dir_all(da.join("coop_saves")).unwrap();
    let x_save = vec![0x11u8; 0x8_0000];
    ga.cart.copy_from_slice(&x_save);
    std::fs::write(
        da.join("coop_saves/sram_owner.txt"),
        "XWORLD-00000001\nstale\n",
    )
    .unwrap();
    let mut a = Bridge::new(&da, &[]).unwrap();
    a.join(&server, "YWORLD", true);
    let swapped = run(&mut [(&mut a, &mut ga)], 10, |p| {
        p[0].1.mb()[RESET_REQUEST] == 1
    });
    assert!(swapped, "the bridge asked the ROM to restart");
    assert_eq!(
        ga.cart,
        vec![0xFF; 0x8_0000],
        "a blank save for the new world"
    );
    assert_eq!(
        std::fs::read(da.join("coop_saves/XWORLD-00000001.sav")).unwrap(),
        x_save
    );
    assert_eq!(
        ga.mb()[SAVE_SLOT_READY],
        0,
        "the gate waits for the restart"
    );
    // The ROM restarts (its heartbeat starts over): the slot is ready.
    put_u32(&mut ga.ram, MB + HEARTBEAT_ROM, 1);
    ga.ram[MB + RESET_REQUEST] = 0;
    assert!(run(&mut [(&mut a, &mut ga)], 5, |p| p[0].1.mb()
        [SAVE_SLOT_READY]
        == 1));
    std::fs::remove_dir_all(&dir).ok();
}

/// A relay of another protocol version answers with its 16-byte refusal (its version at bytes
/// 4..6): the bridge says who must update, as dev/test_protocol_mismatch.sh checks for the C#
/// bridge. A stand-in relay, so this runs without the binary.
#[test]
fn another_protocol_version_is_explained() {
    for (theirs, word) in [
        (goldlink_proto::PROTOCOL_VERSION + 1, "newer"),
        (goldlink_proto::PROTOCOL_VERSION - 1, "older"),
    ] {
        let relay = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
        relay
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let server = relay.local_addr().unwrap().to_string();
        let answer = std::thread::spawn(move || {
            let mut buf = [0u8; 2048];
            let (_, from) = relay.recv_from(&mut buf).unwrap();
            let mut refusal = goldlink_proto::ServerToClientPacket::create_error_response(
                goldlink_proto::ROOM_STATUS_ERR_PROTOCOL,
            );
            refusal[4..6].copy_from_slice(&theirs.to_le_bytes());
            relay.send_to(&refusal, from).unwrap();
        });
        let dir = temp(&format!("proto{theirs}"));
        let mut a = Bridge::new(&dir, &[]).unwrap();
        let mut g = FakeGame::new(1, 100);
        a.join(&server, "PROTO1", false);
        let refused = run(&mut [(&mut a, &mut g)], 5, |p| {
            p[0].0.link() == LinkState::Offline && p[0].0.view().message_is_error
        });
        answer.join().unwrap();
        assert!(refused, "the bridge gave up");
        let msg = a.view().message;
        assert!(
            msg.contains(word) && msg.contains(&format!("protocol {theirs}")),
            "{msg}"
        );
        std::fs::remove_dir_all(&dir).ok();
    }
}

/// The relay with its QUIC listener: returns it and the address with the fingerprint, read from
/// its startup line.
fn start_quic_relay(port: u16, qport: u16, dir: &std::path::Path) -> (Relay, String) {
    let bin = std::env::var_os("GOLDLINK_RELAY")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../../coop-relay/target/release/coop-relay")
        });
    let log = dir.join("relay.log");
    let child = Command::new(&bin)
        .args([
            "--bind",
            &format!("127.0.0.1:{port}"),
            "--quic",
            &format!("127.0.0.1:{qport}"),
            "--save-dir",
        ])
        .arg(dir.join("campaigns"))
        .env("NO_COLOR", "1")
        .stdout(std::fs::File::create(&log).unwrap())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let relay = Relay(child);
    for _ in 0..50 {
        std::thread::sleep(Duration::from_millis(100));
        let text = std::fs::read_to_string(&log).unwrap_or_default();
        if let Some(fp) = text
            .split('#')
            .nth(1)
            .map(|t| t.chars().take(64).collect::<String>())
            && fp.len() == 64
        {
            return (relay, format!("127.0.0.1:{qport}#{fp}"));
        }
    }
    panic!(
        "the relay printed no fingerprint: {}",
        std::fs::read_to_string(&log).unwrap_or_default()
    );
}

/// UC6: a player over QUIC and one over UDP share a world; the QUIC player's address changes
/// (a NAT rebinding) and they keep their seat; a wrong fingerprint is refused.
#[test]
#[ignore = "needs the coop-relay binary with --quic"]
fn quic_and_udp_share_a_world_and_survive_a_rebinding() {
    let port = 21_000 + (std::process::id() % 500) as u16;
    let dir = temp("quic");
    let (_relay, quic_server) = start_quic_relay(port, port + 1, &dir);
    let udp_server = format!("127.0.0.1:{port}");

    let mut a = Bridge::new(&dir.join("a"), &[]).unwrap();
    let mut b = Bridge::new(&dir.join("b"), &[]).unwrap();
    let (mut ga, mut gb) = (
        FakeGame::new(0x1111_0001, 100),
        FakeGame::new(0x2222_0002, 110),
    );
    a.join(&quic_server, "QUIC01", true);
    assert!(
        run(&mut [(&mut a, &mut ga)], 10, |p| p[0].0.link()
            == LinkState::Connected),
        "A connects over QUIC"
    );
    b.join(&udp_server, "QUIC01", false);
    let sees = |g: &FakeGame, tid: u32| {
        (0..REMOTE_SLOTS).any(|s| {
            u32_at(
                g.mb(),
                SERVER_PAYLOAD + s * PLAYER_STATE_SIZE + PS_TRAINER_ID,
            ) == tid
        })
    };
    assert!(
        run(&mut [(&mut a, &mut ga), (&mut b, &mut gb)], 10, |p| sees(
            p[0].1,
            0x2222_0002
        )
            && sees(p[1].1, 0x1111_0001)),
        "QUIC and UDP players see each other"
    );
    let slot = ga.mb()[ASSIGNED_PLAYER_ID];

    // A NAT rebinding: A's packets come from a new port from now on.
    assert!(a.rebind(), "rebound");
    // Longer than the relay's 5 s client timeout: a lost seat would show.
    let end = Instant::now() + Duration::from_secs(7);
    run(&mut [(&mut a, &mut ga), (&mut b, &mut gb)], 8, |_| {
        Instant::now() > end
    });
    assert_eq!(a.link(), LinkState::Connected);
    assert_eq!(ga.mb()[ASSIGNED_PLAYER_ID], slot, "the same seat");
    assert!(sees(&gb, 0x1111_0001), "B still sees A");
    let log = std::fs::read_to_string(dir.join("relay.log")).unwrap();
    // Two: B joined with the plain address and learned QUIC on first use (protocol 9); A's
    // stayed one connection, migrated.
    assert_eq!(
        log.matches("QUIC connection from").count(),
        2,
        "A's connection migrated, not a new one:\n{log}"
    );

    // The wrong certificate: never connects.
    let mut c = Bridge::new(&dir.join("c"), &[]).unwrap();
    let mut gc = FakeGame::new(0x3333_0003, 120);
    let wrong = format!(
        "{}#{}",
        &quic_server[..quic_server.find('#').unwrap()],
        "ab".repeat(32)
    );
    c.join(&wrong, "QUIC01", false);
    assert!(run(&mut [(&mut c, &mut gc)], 10, |p| p[0].0.link()
        == LinkState::Offline
        && p[0].0.view().message_is_error));
    assert!(
        c.view().message.contains("identity changed"),
        "{}",
        c.view().message
    );
    std::fs::remove_dir_all(&dir).ok();
}

/// A relay started with `args` (protocol 9: QUIC is on by default, on port + 1), its log in
/// `log`.
fn start_relay_with(
    port: u16,
    dir: &std::path::Path,
    args: &[&str],
    log: &std::path::Path,
) -> Relay {
    let bin = std::env::var_os("GOLDLINK_RELAY")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../../coop-relay/target/release/coop-relay")
        });
    let child = Command::new(&bin)
        .args(["--bind", &format!("127.0.0.1:{port}")])
        .args(args)
        .arg("--save-dir")
        .arg(dir.join("campaigns"))
        .env("NO_COLOR", "1")
        .stdout(std::fs::File::create(log).unwrap())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    std::thread::sleep(Duration::from_millis(400));
    Relay(child)
}

/// Protocol 9 (rework plan §7.3), QUIC by default: the first connection to a world learns the
/// relay's QUIC port and certificate from its challenge (the hello carries no secret), plays
/// over QUIC and pins the certificate with the world; later ones need it; a new certificate is
/// refused until trusted; a pinned world never falls back to UDP; a `--no-quic` relay plays
/// over UDP.
#[test]
#[ignore = "needs the coop-relay binary (protocol 9)"]
fn quic_by_default_trust_on_first_use() {
    let port = 22_000 + (std::process::id() % 500) as u16 * 2;
    let dir = temp("tofu");
    let server = format!("127.0.0.1:{port}");
    let connected = |b: &mut Bridge, g: &mut FakeGame| {
        run(&mut [(b, g)], 10, |p| p[0].0.link() == LinkState::Connected)
    };
    let offline_with = |b: &mut Bridge, g: &mut FakeGame, text: &str| {
        let ok = run(&mut [(b, g)], 15, |p| {
            p[0].0.link() == LinkState::Offline && p[0].0.view().message_is_error
        });
        ok && b.view().message.contains(text)
    };
    let leave = |b: &mut Bridge, g: &mut FakeGame| {
        b.leave();
        run(&mut [(b, g)], 3, |p| p[0].0.link() == LinkState::Offline);
    };

    // First use: QUIC, pinned with the world.
    let relay = start_relay_with(port, &dir, &[], &dir.join("relay1.log"));
    let mut a = Bridge::new(&dir.join("a"), &[]).unwrap();
    let mut ga = FakeGame::new(0x1111_0001, 100);
    a.join(&server, "TOFU01", true);
    assert!(connected(&mut a, &mut ga), "{}", a.view().message);
    assert!(a.quic_stats().is_some(), "over QUIC");
    let w = a.worlds().find("TOFU01").unwrap().clone();
    assert_eq!((w.pin.len(), w.quic_port), (64, port + 1), "pinned: {w:?}");
    let log = std::fs::read_to_string(dir.join("relay1.log")).unwrap();
    assert!(log.contains(&w.pin), "the relay's own fingerprint:\n{log}");

    // Again: straight to QUIC with the pin.
    leave(&mut a, &mut ga);
    a.join(&server, "TOFU01", false);
    assert!(connected(&mut a, &mut ga), "{}", a.view().message);
    assert!(a.quic_stats().is_some());

    // A new certificate (the host lost it): refused, until trusted in the world panel.
    leave(&mut a, &mut ga);
    drop(relay);
    for f in ["relay-quic-cert.der", "relay-quic-key.der"] {
        std::fs::remove_file(dir.join("campaigns").join(f)).unwrap();
    }
    let relay = start_relay_with(port, &dir, &[], &dir.join("relay2.log"));
    a.join(&server, "TOFU01", false);
    assert!(
        offline_with(&mut a, &mut ga, "identity changed"),
        "{}",
        a.view().message
    );
    let new_fp = a
        .view()
        .identity_change
        .expect("the new fingerprint to trust");
    assert_ne!(new_fp, w.pin);
    assert!(a.trust_new_identity());
    assert_eq!(a.worlds().find("TOFU01").unwrap().pin, new_fp);
    // (A room that never started isn't kept across a relay restart: create it again.)
    a.join(&server, "TOFU01", true);
    assert!(connected(&mut a, &mut ga), "{}", a.view().message);

    // The QUIC port blocked (here: the relay runs without it): no fallback to UDP.
    leave(&mut a, &mut ga);
    drop(relay);
    let _relay = start_relay_with(port, &dir, &["--no-quic"], &dir.join("relay3.log"));
    a.join(&server, "TOFU01", false);
    assert!(
        offline_with(
            &mut a,
            &mut ga,
            &format!("encrypted port, UDP {}", port + 1)
        ),
        "{}",
        a.view().message
    );

    // A --no-quic relay, first use: plain UDP, nothing pinned.
    let mut b = Bridge::new(&dir.join("b"), &[]).unwrap();
    let mut gb = FakeGame::new(0x2222_0002, 110);
    b.join(&server, "TOFU02", true);
    assert!(connected(&mut b, &mut gb), "{}", b.view().message);
    assert!(b.quic_stats().is_none(), "over UDP");
    assert!(b.worlds().find("TOFU02").unwrap().pin.is_empty());
    std::fs::remove_dir_all(&dir).ok();
}
