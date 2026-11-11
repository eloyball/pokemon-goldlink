//! The in-process multiplayer harness (RFC §6.1, UC4's exit test): several DSes in this test
//! process, each with the real Rust bridge, all in one world on the relay. Each "game" runs on
//! its own thread at the DS's pace and follows the link e2e probe's script
//! (dev/bizhawk/link_e2e_probe.lua, its default mode): boot to the overworld through the
//! world's save, then one game opens a link session as the parent, another joins it as the
//! child through the relay's beacons, they link, sync and close the session.
//!
//! Ignored by default: it needs the co-op ROM (`GOLDLINK_TEST_ROM`, default hg-engine/test.nds,
//! a dev build with the debug commands), the regression's saves and the relay binary.
//!
//!     cargo build --release --manifest-path coop-relay/Cargo.toml
//!     cargo test -p goldlink-bridge --release --test harness -- --ignored --test-threads 1

use std::collections::HashSet;
use std::mem::offset_of;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use goldlink_abi::{CoopCommBlock, CoopDiagnostics, CoopPlayerState};
use goldlink_bridge::hook::{after_frame, before_frame, lock};
use goldlink_bridge::layout::*;
use goldlink_bridge::{Bridge, LinkState, SharedBridge};
use goldlink_core::Keys;
use goldlink_runtime::runner::{DS_FPS, Pacer};
use goldlink_runtime::{Emulator, GameSetup, Input};

const MB: usize = 0x3D8060;
const ROOM: &str = "HARNES";
const ROOM_UID: u32 = 0x4841_524E;

fn repo() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn rom() -> Arc<Vec<u8>> {
    let p = std::env::var_os("GOLDLINK_TEST_ROM")
        .map(PathBuf::from)
        .unwrap_or_else(|| repo().join("hg-engine/test.nds"));
    Arc::new(std::fs::read(&p).unwrap_or_else(|e| panic!("{}: {e}", p.display())))
}

/// The regression's saves, different trainers (dev/run_team_battle_e2e.sh's A..D).
fn saves() -> [PathBuf; 4] {
    let r = repo();
    [
        r.join("BizHawk/NDS/SaveRAM/test.SaveRAM"),
        r.join("BizHawk/coop_saves/unassigned-20261025-232946.sav"),
        r.join("instances/p2/coop_saves/TEST02-C7BE7B93.sav"),
        r.join("instances/p2/coop_saves/unassigned-20261025-233004.sav"),
    ]
}

/// One game: the DS, its bridge (None: an offline stand-in), its marks.
struct Game {
    role: char,
    emu: Emulator,
    bridge: Option<SharedBridge>,
    pacer: Pacer,
    held: Keys,
    seq: u8,
    hb: u32,
    marks: Arc<Mutex<HashSet<String>>>,
    log: Vec<String>,
    pass: u32,
    fail: u32,
    server: String,
    frames: u64,
}

impl Game {
    fn mb(&self) -> &[u8] {
        &self.emu.nds.main_ram()[MB..MB + MAILBOX_SIZE]
    }
    fn u8_at(&self, off: usize) -> u8 {
        self.mb()[off]
    }
    fn booted(&self) -> bool {
        self.emu.mailbox().is_some()
    }
    fn connected(&self) -> bool {
        self.booted() && self.u8_at(IS_CONNECTED_TO_ROOM) == 1
    }
    fn in_field(&self) -> bool {
        self.booted()
            && self.u8_at(
                offset_of!(CoopCommBlock, diag) + offset_of!(CoopDiagnostics, field_entry_status),
            ) == 3
            && self.u8_at(LOCAL_PLAYER + offset_of!(CoopPlayerState, is_active)) == 1
    }
    fn trainer_id(&self) -> u32 {
        u32_at(self.mb(), LOCAL_PLAYER + PS_TRAINER_ID)
    }

    fn frame(&mut self) {
        let input = Input {
            keys: self.held,
            touch: None,
        };
        match &self.bridge {
            Some(b) => {
                before_frame(b, &mut self.emu.nds, MB);
                self.emu.frame(input);
                // As the window's emulation thread (BridgeHook): straight into the game (UC9 §2).
                if after_frame(b, &mut self.emu.nds, MB).boot_into_game {
                    self.emu.boot_into_game().unwrap();
                }
                // COOP_BRIDGE_AUTOJOIN's behaviour: join, and again after a failed attempt.
                self.frames += 1;
                if self.frames % 60 == 0 {
                    let mut br = lock(b);
                    if br.link() == LinkState::Offline {
                        br.join(&self.server, ROOM, false);
                    }
                }
            }
            None => {
                // The offline stand-in (the probes' "play the bridge").
                self.emu.frame(input);
                if self.booted() {
                    self.hb += 1;
                    let hb = self.hb;
                    let ram = self.emu.nds.main_ram_mut();
                    put_u32(ram, MB + HEARTBEAT_BRIDGE, hb);
                    ram[MB + IS_CONNECTED_TO_ROOM] = 1;
                    ram[MB + SAVE_SLOT_READY] = 1;
                }
            }
        }
        self.pacer.wait(DS_FPS);
    }
    fn wait(&mut self, n: u32) {
        for _ in 0..n {
            self.frame();
        }
    }
    fn press(&mut self, k: Keys, n: u32) {
        self.held = k;
        self.wait(n);
        self.held = Keys::NONE;
        self.wait(6);
    }
    fn wait_for(&mut self, n: u32, cond: impl Fn(&Game) -> bool) -> bool {
        for _ in 0..n {
            if cond(self) {
                return true;
            }
            self.frame();
        }
        cond(self)
    }
    /// A debug command (COOP_DEBUG_*): its result, or None when the ROM never acknowledged it.
    fn command(&mut self, cmd: u8, arg: u8) -> Option<u8> {
        self.seq = self.seq % 255 + 1;
        let seq = self.seq;
        let ram = self.emu.nds.main_ram_mut();
        ram[MB + DEBUG_ARG] = arg;
        ram[MB + DEBUG_RESULT] = 0;
        ram[MB + DEBUG_CMD] = cmd;
        ram[MB + DEBUG_SEQ] = seq;
        let ack = DIAG + offset_of!(CoopDiagnostics, debug_ack);
        for _ in 0..600 {
            self.frame();
            if self.u8_at(ack) == seq {
                return Some(self.u8_at(DEBUG_RESULT));
            }
        }
        None
    }
    fn status(&mut self) -> u8 {
        self.command(13, 0x42).unwrap_or(0)
    }
    /// The open link block's main-RAM offset.
    fn block(&self) -> Option<usize> {
        let ram = self.emu.nds.main_ram();
        goldlink_bridge::link::find(ram, u32_at(ram, MB + EXCHANGE_PTR))
    }
    fn check(&mut self, name: &str, ok: bool) {
        if ok {
            self.pass += 1;
        } else {
            self.fail += 1;
        }
        let line = format!(
            "[{} {:6}] {} {name}",
            self.role,
            self.emu.frame_count(),
            if ok { "PASS" } else { "FAIL" }
        );
        eprintln!("{line}");
        self.log.push(line);
    }
    fn mark(&self, name: &str) {
        self.marks
            .lock()
            .unwrap()
            .insert(format!("{}_{name}", self.role));
    }
    fn wait_marks(&mut self, roles: &[char], name: &str, n: u32) -> bool {
        let marks = self.marks.clone();
        let want: Vec<String> = roles.iter().map(|r| format!("{r}_{name}")).collect();
        self.wait_for(n, move |_| {
            let m = marks.lock().unwrap();
            want.iter().all(|w| m.contains(w))
        })
    }
    /// Start or A now and then until the overworld.
    fn boot_to_field(&mut self, need_connected: bool) -> bool {
        for i in 0..6000u32 {
            if self.in_field() {
                return true;
            }
            if i % 60 == 20 && (!need_connected || self.connected()) {
                self.press(if i % 120 == 20 { Keys::START } else { Keys::A }, 3);
            } else {
                self.frame();
            }
        }
        self.in_field()
    }
}

fn game(
    role: char,
    rom: Arc<Vec<u8>>,
    cart: PathBuf,
    bridge: Option<SharedBridge>,
    marks: Arc<Mutex<HashSet<String>>>,
    server: &str,
) -> Game {
    let emu = Emulator::new(GameSetup {
        rom,
        cart_save: Some(cart),
        mailbox_offset: MB,
        rtc: Some(
            chrono::NaiveDate::from_ymd_opt(2026, 11, 8)
                .unwrap()
                .and_hms_opt(12, 0, 0)
                .unwrap(),
        ),
    })
    .unwrap();
    if let Some(b) = &bridge {
        lock(b).set_boots_into_game(true);
    }
    Game {
        role,
        emu,
        bridge,
        pacer: Pacer::new(),
        held: Keys::NONE,
        seq: 0,
        hb: 0,
        marks,
        log: Vec::new(),
        pass: 0,
        fail: 0,
        server: server.into(),
        frames: 0,
    }
}

/// Each save's trainer ID: boot it to the overworld with the stand-in (link_probe's
/// LINK_TID_ONLY).
fn trainer_ids(rom: &Arc<Vec<u8>>, saves: &[PathBuf], dir: &Path) -> Vec<u32> {
    let handles: Vec<_> = saves
        .iter()
        .enumerate()
        .map(|(i, s)| {
            let cart = dir.join(format!("tid-{i}.sav"));
            std::fs::create_dir_all(dir).unwrap();
            std::fs::copy(s, &cart).unwrap();
            let rom = rom.clone();
            std::thread::spawn(move || {
                let mut g = game('T', rom, cart, None, Arc::default(), "");
                g.pacer = Pacer::new();
                // Uncapped: no relay here.
                let mut ok = false;
                for i in 0..6000u32 {
                    if g.in_field() {
                        ok = true;
                        break;
                    }
                    g.held = if i % 60 >= 20 && i % 60 < 23 {
                        if i % 120 < 60 { Keys::START } else { Keys::A }
                    } else {
                        Keys::NONE
                    };
                    let input = Input {
                        keys: g.held,
                        touch: None,
                    };
                    g.emu.frame(input);
                    if g.booted() {
                        g.hb += 1;
                        let hb = g.hb;
                        let ram = g.emu.nds.main_ram_mut();
                        put_u32(ram, MB + HEARTBEAT_BRIDGE, hb);
                        ram[MB + IS_CONNECTED_TO_ROOM] = 1;
                        ram[MB + SAVE_SLOT_READY] = 1;
                    }
                }
                assert!(ok, "save {i} never reached the overworld");
                g.trainer_id()
            })
        })
        .collect();
    handles.into_iter().map(|h| h.join().unwrap()).collect()
}

struct Relay(Child);

impl Drop for Relay {
    fn drop(&mut self) {
        self.0.kill().ok();
        self.0.wait().ok();
    }
}

/// The test's folder, the relay, each game's role and folder, the ROM.
type World = (PathBuf, Relay, Vec<(char, PathBuf)>, Arc<Vec<u8>>);

fn world(n: usize, port: u16) -> World {
    let dir = std::env::temp_dir().join(format!("goldlink-harness-{n}-{}", std::process::id()));
    std::fs::remove_dir_all(&dir).ok();
    std::fs::create_dir_all(dir.join("campaigns")).unwrap();
    let rom = rom();
    let saves = &saves()[..n];
    let tids = trainer_ids(&rom, saves, &dir.join("tid"));
    // A started campaign listing every trainer (dev/run_link_e2e.sh step 2).
    let roster: Vec<String> = tids
        .iter()
        .enumerate()
        .map(|(i, t)| {
            format!(
                "{{ \"slot_id\": {i}, \"trainer_id\": {t}, \"last_badge_mask\": 0, \"assigned_monolocke\": 18, \"player_secret\": 0 }}"
            )
        })
        .collect();
    std::fs::write(
        dir.join("campaigns").join(format!("{ROOM}.json")),
        format!(
            "{{ \"room_code\": \"{ROOM}\", \"room_uid\": {ROOM_UID}, \"host_token\": 1, \"campaign_started\": true, \"expected_party_size\": {n}, \"rules\": 0, \"roster\": [{}], \"teams\": [] }}",
            roster.join(", ")
        ),
    )
    .unwrap();
    // Each game's folder: its save as the cart and as the world's save.
    let mut games = Vec::new();
    for (i, s) in saves.iter().enumerate() {
        let role = (b'A' + i as u8) as char;
        let gd = dir.join(role.to_string());
        std::fs::create_dir_all(gd.join("coop_saves")).unwrap();
        std::fs::copy(s, gd.join("cart.sav")).unwrap();
        std::fs::copy(
            s,
            gd.join("coop_saves")
                .join(format!("{ROOM}-{ROOM_UID:08X}.sav")),
        )
        .unwrap();
        games.push((role, gd));
    }
    let bin = std::env::var_os("GOLDLINK_RELAY")
        .map(PathBuf::from)
        .unwrap_or_else(|| repo().join("coop-relay/target/release/coop-relay"));
    let relay = Command::new(&bin)
        .args(["--bind", &format!("127.0.0.1:{port}"), "--save-dir"])
        .arg(dir.join("campaigns"))
        .stdout(Stdio::null())
        .stderr(std::fs::File::create(dir.join("relay.log")).unwrap())
        .spawn()
        .unwrap_or_else(|e| panic!("{}: {e}", bin.display()));
    std::thread::sleep(Duration::from_millis(500));
    (dir, Relay(relay), games, rom)
}

/// The probe's default script for one game. `parent`: open the session; `child_of`: join it.
fn link_script(g: &mut Game, all: &[char], partner: Option<(char, bool)>) {
    let ok = g.wait_for(6000, Game::connected);
    g.check("bridge connected", ok);
    let field = g.boot_to_field(true);
    g.check("in the field", field);
    if !field {
        return;
    }
    g.wait(60);
    g.press(Keys::B, 3);
    g.wait(30);
    g.mark("ready");
    let ok = g.wait_marks(all, "ready", 9000);
    g.check("everyone ready", ok);
    // Everyone sees everyone else in a remote slot.
    let others = (0..REMOTE_SLOTS)
        .filter(|s| {
            u32_at(
                g.mb(),
                SERVER_PAYLOAD + s * PLAYER_STATE_SIZE + PS_TRAINER_ID,
            ) != 0
        })
        .count();
    let name = format!("sees the {} others", all.len() - 1);
    g.check(&name, others == all.len() - 1);
    let Some((peer, parent)) = partner else {
        // A bystander: stays in the field until the link pair is done.
        let ok = g.wait_marks(&['A', 'B'], "done", 20000);
        let still = ok && g.connected() && g.in_field();
        g.check("still connected while the others linked", still);
        return;
    };
    if parent {
        let r = g.command(12, 1);
        g.check("parent session started", r == Some(1));
        let ok = g.wait_for(300, |g| g.block().is_some());
        g.check("link block published", ok);
    } else {
        let r = g.command(12, 2);
        g.check("child session started", r == Some(1));
        let ok = g.wait_for(300, |g| g.block().is_some());
        g.check("link block published", ok);
        let ok = g.wait_for(1800, |g| {
            g.block()
                .is_some_and(|b| g.emu.nds.main_ram()[b + 0x88 + 4] > 0)
        });
        g.check("parent's beacon seen (through the relay)", ok);
        g.wait(120);
        let r = g.command(12, 3);
        g.check("connect requested", r == Some(1));
    }
    let mut linked = false;
    for _ in 0..60 {
        if g.status() & 0x43 == 0x43 {
            linked = true;
            break;
        }
        g.wait(60);
    }
    g.check("linked", linked);
    g.mark("linked");
    g.wait_marks(&[peer], "linked", 3600);
    let r = g.command(12, 4);
    g.check("sync requested", r == Some(1));
    let mut synced = false;
    for _ in 0..60 {
        if g.status() & 0x47 == 0x47 {
            synced = true;
            break;
        }
        g.wait(60);
    }
    g.check("LinkSync round trip", synced);
    g.mark("synced");
    g.wait_marks(&[peer], "synced", 3600);
    g.wait(60);
    let r = g.command(12, 5);
    g.check("shutdown requested", r == Some(1));
    let mut ended = false;
    for _ in 0..60 {
        if g.status() & 1 == 0 && u32_at(g.mb(), EXCHANGE_PTR) == 0 {
            ended = true;
            break;
        }
        g.wait(60);
    }
    g.check("session ended, block retired", ended);
    let still = g.connected() && g.in_field();
    g.check("still connected and in the field", still);
    g.mark("done");
    g.wait_marks(&[peer], "done", 1800);
}

fn run_world(n: usize, port: u16) {
    let (dir, _relay, games, rom) = world(n, port);
    let server = format!("127.0.0.1:{port}");
    let marks: Arc<Mutex<HashSet<String>>> = Arc::default();
    let all: Vec<char> = games.iter().map(|(r, _)| *r).collect();
    let started = Instant::now();
    let handles: Vec<_> = games
        .into_iter()
        .map(|(role, gd)| {
            let (rom, marks, all, server) =
                (rom.clone(), marks.clone(), all.clone(), server.clone());
            std::thread::spawn(move || {
                let bridge = Bridge::new(&gd, &[]).unwrap();
                let bridge: SharedBridge = Arc::new(Mutex::new(bridge));
                let mut g = game(
                    role,
                    rom,
                    gd.join("cart.sav"),
                    Some(bridge.clone()),
                    marks,
                    &server,
                );
                lock(&bridge).join(&server, ROOM, false);
                let partner = match role {
                    'A' => Some(('B', true)),
                    'B' => Some(('A', false)),
                    _ => None,
                };
                link_script(&mut g, &all, partner);
                lock(&bridge).leave();
                (g.pass, g.fail, g.log)
            })
        })
        .collect();
    let mut failed = Vec::new();
    for h in handles {
        let (pass, fail, log) = h.join().unwrap();
        assert!(pass > 0);
        if fail > 0 {
            failed.extend(log.into_iter().filter(|l| l.contains("FAIL")));
        }
    }
    eprintln!("{n} games: {:.0} s", started.elapsed().as_secs_f64());
    assert!(failed.is_empty(), "{failed:#?}");
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
#[ignore = "needs the co-op ROM, the regression's saves and the relay"]
fn two_games_link_through_the_relay() {
    run_world(2, 19_000 + (std::process::id() % 500) as u16);
}

#[test]
#[ignore = "needs the co-op ROM, the regression's saves and the relay"]
fn four_games_in_one_world_two_of_them_link() {
    run_world(4, 19_500 + (std::process::id() % 500) as u16);
}

// Straight into the game (dev/docs/unified-client/UC9_PROPOSAL.md §2): pret main.c's work struct
// (_02111868: the running app's manager at +4) and Oak's speech (gApplication_OakSpeech).
const MAIN_WORK: usize = 0x0211_1868;
const APP_OAK_SPEECH: usize = 0x0210_6068;

/// Whether the running app is Oak's speech: its manager starts with a copy of its template, so
/// both begin with the same init function.
fn app_is_oak_speech(g: &Game) -> bool {
    let ram = g.emu.nds.main_ram();
    let at = |addr: usize| -> Option<usize> {
        let o = addr.checked_sub(0x0200_0000)?;
        Some(u32::from_le_bytes(ram.get(o..o + 4)?.try_into().ok()?) as usize)
    };
    let running = at(MAIN_WORK + 4).filter(|&m| m != 0).and_then(at);
    running.is_some() && running == at(APP_OAK_SPEECH)
}

#[test]
#[ignore = "needs the co-op ROM, the regression's saves and the relay"]
fn a_join_goes_straight_into_the_game() {
    let port = 20_000 + (std::process::id() % 500) as u16;
    let (dir, _relay, games, rom) = world(1, port);
    let server = format!("127.0.0.1:{port}");
    let (role, gd) = games.into_iter().next().unwrap();
    let bridge: SharedBridge = Arc::new(Mutex::new(Bridge::new(&gd, &[]).unwrap()));
    let mut g = game(
        role,
        rom,
        gd.join("cart.sav"),
        Some(bridge.clone()),
        Arc::default(),
        &server,
    );
    lock(&bridge).join(&server, ROOM, false);
    // No key at all: the bridge's restart goes straight into Continue (no intro, title or menu).
    assert!(g.wait_for(1800, |g| g.connected()), "never connected");
    let connected = g.frames;
    assert!(g.wait_for(600, |g| lock(g.bridge.as_ref().unwrap()).view().slot_ready));
    let rebooted = g.frames;
    let ok = g.wait_for(1800, |g| g.in_field());
    assert!(
        ok,
        "not in the overworld after {} frames with no input",
        g.frames
    );
    g.frame();
    assert!(
        lock(&bridge).view().game_started,
        "the window would still wait"
    );
    eprintln!(
        "join → overworld: {} frames ({:.1} s), no input: connected at {connected}, rebooted \
         at {rebooted}",
        g.frames,
        g.frames as f64 / DS_FPS
    );
    lock(&bridge).leave();
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
#[ignore = "needs the co-op ROM and the relay"]
fn a_new_world_starts_a_new_game() {
    let port = 20_500 + (std::process::id() % 500) as u16;
    let dir = std::env::temp_dir().join(format!("goldlink-harness-new-{}", std::process::id()));
    std::fs::remove_dir_all(&dir).ok();
    std::fs::create_dir_all(dir.join("campaigns")).unwrap();
    std::fs::create_dir_all(dir.join("A")).unwrap();
    let bin = std::env::var_os("GOLDLINK_RELAY")
        .map(PathBuf::from)
        .unwrap_or_else(|| repo().join("coop-relay/target/release/coop-relay"));
    let _relay = Relay(
        Command::new(&bin)
            .args(["--bind", &format!("127.0.0.1:{port}"), "--save-dir"])
            .arg(dir.join("campaigns"))
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap_or_else(|e| panic!("{}: {e}", bin.display())),
    );
    std::thread::sleep(Duration::from_millis(500));
    let server = format!("127.0.0.1:{port}");
    let gd = dir.join("A");
    let bridge: SharedBridge = Arc::new(Mutex::new(Bridge::new(&gd, &[]).unwrap()));
    // A fresh install: no cart file yet, so the cart is blank.
    let mut g = game(
        'A',
        rom(),
        gd.join("cart.sav"),
        Some(bridge.clone()),
        Arc::default(),
        &server,
    );
    lock(&bridge).join(&server, "NEWGM1", true);
    let ok = g.wait_for(1800, app_is_oak_speech);
    assert!(
        ok,
        "Oak's speech didn't start within 1800 frames with no input (link {:?})",
        lock(&bridge).link()
    );
    g.frame();
    assert!(
        lock(&bridge).view().game_started,
        "the window would still wait"
    );
    eprintln!(
        "new world → Oak's speech: {} frames ({:.1} s), no input",
        g.frames,
        g.frames as f64 / DS_FPS
    );
    lock(&bridge).leave();
    // Leave in Oak's speech: nothing to save yet, so the window doesn't wait for one.
    for _ in 0..60 {
        g.frame();
    }
    assert!(
        !lock(&bridge).view().game_in_session,
        "Leave would wait for a save in Oak's speech"
    );
    std::fs::remove_dir_all(&dir).ok();
}

/// Where each game really was, frame by frame for the last 2 s: role → (when, trainer, map, x,
/// z), written by the game itself.
type Truth = Arc<
    Mutex<
        std::collections::HashMap<char, std::collections::VecDeque<(Instant, u32, u16, i16, i16)>>,
    >,
>;

fn six_saves() -> [PathBuf; 6] {
    let r = repo();
    let four = saves();
    [
        four[0].clone(),
        four[1].clone(),
        four[2].clone(),
        four[3].clone(),
        r.join("BizHawk/coop_saves/TEST03-BC71B831.sav"),
        r.join("instances/p2/coop_saves/TEST03-BC71B831.sav"),
    ]
}

/// One game of the soak: into the world, warp to Goldenrod (dense), then walk at random until
/// `until`, checking every second that it's still connected, sees the five others, and that
/// what it sees of each matches where that game says it is (within 2 tiles, on the same map;
/// five checks in a row off, about 3 to 5 s, is a desync).
fn soak_script(g: &mut Game, all: &[char], truth: &Truth, until: Instant) -> (u32, u32, u32) {
    let ok = g.wait_for(6000, Game::connected);
    g.check("bridge connected", ok);
    let field = g.boot_to_field(true);
    g.check("in the field", field);
    if !field {
        return (0, 0, 0);
    }
    g.wait(60);
    g.press(Keys::B, 3);
    // COOP_DEBUG_WARP, COOP_WARP_GOLDENROD; "not now" (3) while a step or message runs: again.
    let mut r = None;
    for _ in 0..5 {
        r = g.command(5, 3);
        if r == Some(1) {
            break;
        }
        g.press(Keys::B, 3);
        g.wait(120);
    }
    g.check(&format!("warped to Goldenrod (result {r:?})"), r == Some(1));
    g.wait(240);
    g.mark("ready");
    let ok = g.wait_marks(all, "ready", 9000);
    g.check("everyone ready", ok);
    let (mut seconds, mut blind, mut desyncs) = (0u32, 0u32, 0u32);
    let mut behind: std::collections::HashMap<u32, u32> = Default::default();
    let dirs = [Keys::UP, Keys::DOWN, Keys::LEFT, Keys::RIGHT];
    let mut rng = 0x9E37_79B9u32 ^ g.role as u32;
    let mut next = || {
        rng ^= rng << 13;
        rng ^= rng >> 17;
        rng ^= rng << 5;
        rng
    };
    while Instant::now() < until {
        g.held = dirs[(next() % 4) as usize];
        let steps = 16 + next() % 48;
        for i in 0..steps + 4 {
            if i == steps {
                g.held = Keys::NONE;
            }
            g.frame();
            // Publish where I am, every frame.
            let lp = LOCAL_PLAYER;
            let now = Instant::now();
            let me = (
                now,
                g.trainer_id(),
                u16_at(g.mb(), lp + PS_MAP_ID),
                u16_at(g.mb(), lp + PS_X) as i16,
                u16_at(g.mb(), lp + PS_Z) as i16,
            );
            let mut t = truth.lock().unwrap();
            let h = t.entry(g.role).or_default();
            h.push_back(me);
            while h
                .front()
                .is_some_and(|f| now.duration_since(f.0) > Duration::from_secs(2))
            {
                h.pop_front();
            }
        }
        // One check per walk (about every 0.5 to 1 s).
        seconds += 1;
        if !g.connected() {
            g.check("still connected", false);
            break;
        }
        let seen: Vec<(u32, u16, i16, i16)> = (0..REMOTE_SLOTS)
            .map(|s| {
                let o = SERVER_PAYLOAD + s * PLAYER_STATE_SIZE;
                let mb = g.mb();
                (
                    u32_at(mb, o + PS_TRAINER_ID),
                    u16_at(mb, o + PS_MAP_ID),
                    u16_at(mb, o + PS_X) as i16,
                    u16_at(mb, o + PS_Z) as i16,
                )
            })
            .filter(|p| p.0 != 0)
            .collect();
        if seen.len() != all.len() - 1 {
            blind += 1;
        }
        // In step: what I see of a player is where they were at some point in the last second
        // (the relay's tick and the network in between), within a tile.
        let now = Instant::now();
        let histories: Vec<Vec<(u32, u16, i16, i16)>> = truth
            .lock()
            .unwrap()
            .iter()
            .filter(|(r, _)| **r != g.role)
            .map(|(_, h)| {
                h.iter()
                    .filter(|e| now.duration_since(e.0) <= Duration::from_secs(1))
                    .map(|e| (e.1, e.2, e.3, e.4))
                    .collect()
            })
            .collect();
        for hist in histories {
            let Some(t) = hist.last().copied() else {
                continue;
            };
            let close = seen.iter().any(|s| {
                s.0 == t.0
                    && hist
                        .iter()
                        .any(|h| s.1 == h.1 && (s.2 - h.2).abs() <= 1 && (s.3 - h.3).abs() <= 1)
            });
            let n = behind.entry(t.0).or_default();
            *n = if close { 0 } else { *n + 1 };
            if *n == 5 {
                desyncs += 1;
                eprintln!(
                    "[{}] out of step with trainer {:08X} for 5 checks: sees {:?}, it is at {:?}",
                    g.role,
                    t.0,
                    seen.iter().find(|s| s.0 == t.0),
                    t
                );
            }
        }
    }
    (seconds, blind, desyncs)
}

/// The RFC M5 soak with six games (SOAK_SECS, default 300; the exit criterion is 2 hours).
#[test]
#[ignore = "needs the co-op ROM, the regression's saves and the relay"]
fn six_games_soak() {
    let secs: u64 = std::env::var("SOAK_SECS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(300);
    let port = 20_000 + (std::process::id() % 500) as u16;
    let dir = std::env::temp_dir().join(format!("goldlink-soak-{}", std::process::id()));
    std::fs::remove_dir_all(&dir).ok();
    std::fs::create_dir_all(dir.join("campaigns")).unwrap();
    let rom = rom();
    let saves = six_saves();
    let tids = trainer_ids(&rom, &saves, &dir.join("tid"));
    let unique: HashSet<u32> = tids.iter().copied().collect();
    assert_eq!(unique.len(), 6, "six different trainers: {tids:08X?}");
    let roster: Vec<String> = tids
        .iter()
        .enumerate()
        .map(|(i, t)| format!("{{ \"slot_id\": {i}, \"trainer_id\": {t}, \"last_badge_mask\": 0, \"assigned_monolocke\": 18, \"player_secret\": 0 }}"))
        .collect();
    std::fs::write(
        dir.join("campaigns").join(format!("{ROOM}.json")),
        format!("{{ \"room_code\": \"{ROOM}\", \"room_uid\": {ROOM_UID}, \"host_token\": 1, \"campaign_started\": true, \"expected_party_size\": 6, \"rules\": 0, \"roster\": [{}], \"teams\": [] }}", roster.join(", ")),
    )
    .unwrap();
    let bin = std::env::var_os("GOLDLINK_RELAY")
        .map(PathBuf::from)
        .unwrap_or_else(|| repo().join("coop-relay/target/release/coop-relay"));
    let _relay = Relay(
        Command::new(&bin)
            .args(["--bind", &format!("127.0.0.1:{port}"), "--save-dir"])
            .arg(dir.join("campaigns"))
            .stdout(Stdio::null())
            .stderr(std::fs::File::create(dir.join("relay.log")).unwrap())
            .spawn()
            .unwrap(),
    );
    std::thread::sleep(Duration::from_millis(500));
    let server = format!("127.0.0.1:{port}");
    let marks: Arc<Mutex<HashSet<String>>> = Arc::default();
    let truth: Truth = Arc::default();
    let all: Vec<char> = "ABCDEF".chars().collect();
    let until = Instant::now() + Duration::from_secs(120 + secs);
    let handles: Vec<_> = saves
        .iter()
        .enumerate()
        .map(|(i, s)| {
            let role = all[i];
            let gd = dir.join(role.to_string());
            std::fs::create_dir_all(gd.join("coop_saves")).unwrap();
            std::fs::copy(s, gd.join("cart.sav")).unwrap();
            std::fs::copy(
                s,
                gd.join("coop_saves")
                    .join(format!("{ROOM}-{ROOM_UID:08X}.sav")),
            )
            .unwrap();
            let (rom, marks, all, server, truth) = (
                rom.clone(),
                marks.clone(),
                all.clone(),
                server.clone(),
                truth.clone(),
            );
            std::thread::spawn(move || {
                let bridge: SharedBridge = Arc::new(Mutex::new(Bridge::new(&gd, &[]).unwrap()));
                let mut g = game(
                    role,
                    rom,
                    gd.join("cart.sav"),
                    Some(bridge.clone()),
                    marks,
                    &server,
                );
                lock(&bridge).join(&server, ROOM, false);
                let r = soak_script(&mut g, &all, &truth, until);
                lock(&bridge).leave();
                (role, g.fail, r)
            })
        })
        .collect();
    let mut problems = Vec::new();
    for h in handles {
        let (role, fail, (seconds, blind, desyncs)) = h.join().unwrap();
        eprintln!(
            "{role}: {seconds} checks, {blind} not seeing all five, {desyncs} desyncs, {fail} failed checks"
        );
        if fail > 0 || desyncs > 0 || seconds == 0 || blind * 20 > seconds {
            problems.push(role);
        }
    }
    assert!(problems.is_empty(), "games with problems: {problems:?}");
    std::fs::remove_dir_all(&dir).ok();
}

/// The battle HUD's reading of the game (UC9 §3.2) through a wild battle's first turn: the
/// command menu waits with "What will … do?", FIGHT lists the moves with their PP, the key
/// cursor follows the keys, and once a move is chosen nothing waits.
#[test]
#[ignore = "needs the co-op ROM, the regression's saves and the relay"]
fn the_battle_hud_reads_a_wild_battle() {
    use goldlink_bridge::battle_ui::{self, menu};
    let port = 21_000 + (std::process::id() % 500) as u16;
    let (dir, _relay, games, rom) = world(1, port);
    let server = format!("127.0.0.1:{port}");
    let (role, gd) = games.into_iter().next().unwrap();
    let bridge: SharedBridge = Arc::new(Mutex::new(Bridge::new(&gd, &[]).unwrap()));
    let charmap = goldlink_rom::nitro::Charmap::new();
    let moves = goldlink_rom::nitro::MoveInfo::read(&rom);
    let mut g = game(
        role,
        rom,
        gd.join("cart.sav"),
        Some(bridge.clone()),
        Arc::default(),
        &server,
    );
    lock(&bridge).join(&server, ROOM, false);
    assert!(g.wait_for(3600, |g| g.in_field()), "never in the overworld");
    g.wait(60);
    assert_eq!(g.command(15, 5), Some(1), "the wild battle debug command");
    let read = |g: &Game| battle_ui::read(g.emu.nds.main_ram());
    assert!(g.wait_for(600, |g| read(g).is_some()), "no battle found");
    let waiting_in = |m: i8| move |g: &Game| read(g).is_some_and(|b| b.waiting && b.menu == m);
    assert!(
        g.wait_for(60 * 40, |g| read(g).is_some_and(
            |b| b.waiting && (b.menu == menu::MAIN_INITIAL || b.menu == menu::MAIN)
        )),
        "the command menu never waited: {:?}",
        read(&g)
    );
    let b = read(&g).unwrap();
    let text = charmap.decode(&b.message);
    eprintln!("command menu: {b:?}\n  message {text:?}");
    assert!(text.starts_with("What will"), "the message: {text:?}");
    // The first A shows the cursor on FIGHT (once the menu has slid in), the second opens it.
    g.wait(45);
    g.press(Keys::A, 6);
    g.wait(20);
    assert_eq!(
        read(&g).unwrap().cursor,
        Some((0, 0)),
        "the cursor on FIGHT"
    );
    g.press(Keys::A, 6);
    assert!(
        g.wait_for(120, waiting_in(menu::FIGHT)),
        "FIGHT never opened: {:?}",
        read(&g)
    );
    let b = read(&g).unwrap();
    for m in b.moves.iter().flatten() {
        eprintln!(
            "  {} {}/{} ({})",
            moves.name(m.id),
            m.pp,
            m.max,
            moves.type_of(m.id).unwrap_or(0)
        );
    }
    assert!(b.moves[0].is_some_and(|m| m.max > 0 && !moves.name(m.id).is_empty()));
    g.press(Keys::RIGHT, 6);
    g.wait(10);
    eprintln!("after Right: cursor {:?}", read(&g).unwrap().cursor);
    g.press(Keys::A, 6);
    g.wait(30);
    let b = read(&g).unwrap();
    eprintln!("after choosing: {b:?}");
    assert!(!b.waiting, "still waiting after a move was chosen");
    lock(&bridge).leave();
    std::fs::remove_dir_all(&dir).ok();
}
