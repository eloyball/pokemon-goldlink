//! The real bridge in dev runs, with the C# bridge's dev hooks (RFC_DELTA §7.2, DECISIONS D2:
//! they live here, never in the release program):
//!
//! - `COOP_BRIDGE_AUTOJOIN="host:port|CODE"` joins that world as soon as the game runs, and again
//!   after a failed attempt, until connected (dev/run_link_e2e.sh and friends);
//! - `COOP_BRIDGE_LEAVE_IN_BATTLE=<seconds>` leaves once the game has been in a battle that long,
//!   once (KNOWN_RISKS #52 repro);
//! - `COOP_BRIDGE_STATUS_LOG=<file>` appends every message the bridge shows (test_protocol_mismatch.sh).

use std::io::Write;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use goldlink::goldlink_bridge::{self as gb, Bridge, LinkState, SharedBridge};
use goldlink_runtime::{Emulator, FrameHook};

pub struct DevTicker {
    pub bridge: SharedBridge,
    autojoin: Option<(String, String)>,
    leave_in_battle: Option<f64>,
    battle_since: Option<Instant>,
    frames: u64,
}

/// The bridge with its config in `dir` (BizHawk's instance folder in EmuHawk's form).
pub fn new(dir: &Path) -> Result<DevTicker, String> {
    let mut bridge =
        Bridge::new(dir, &goldlink::avatar_files()).map_err(|e| format!("the network: {e}"))?;
    if let Some(log) = std::env::var_os("COOP_BRIDGE_STATUS_LOG").filter(|v| !v.is_empty()) {
        let log = std::path::PathBuf::from(log);
        bridge.status_listener = Some(Box::new(move |msg, is_error| {
            if let Ok(mut f) = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&log)
            {
                writeln!(f, "{}: {msg}", if is_error { "error" } else { "info" }).ok();
            }
        }));
    }
    let autojoin = std::env::var("COOP_BRIDGE_AUTOJOIN")
        .ok()
        .and_then(|v| v.split_once('|').map(|(s, c)| (s.to_owned(), c.to_owned())));
    let leave_in_battle = std::env::var("COOP_BRIDGE_LEAVE_IN_BATTLE")
        .ok()
        .and_then(|v| v.parse().ok());
    Ok(DevTicker {
        bridge: Arc::new(Mutex::new(bridge)),
        autojoin,
        leave_in_battle,
        battle_since: None,
        frames: 0,
    })
}

impl DevTicker {
    /// Once per frame, after the bridge's own work.
    pub fn tick(&mut self) {
        self.frames += 1;
        let mut b = gb::hook::lock(&self.bridge);
        if self.frames % 60 == 0
            && let Some((server, code)) = &self.autojoin
        {
            match b.link() {
                LinkState::Connected => self.autojoin = None,
                // COOP_BRIDGE_AUTOJOIN_CREATE=1: create the world (a new game) instead.
                LinkState::Offline => b.join(
                    server,
                    code,
                    std::env::var_os("COOP_BRIDGE_AUTOJOIN_CREATE").is_some(),
                ),
                _ => {}
            }
        }
        // GOLDLINK_QUIC_STATS: the QUIC connection's counters every 10 s.
        if self.frames % 600 == 0
            && std::env::var_os("GOLDLINK_QUIC_STATS").is_some()
            && let Some(line) = b.quic_stats()
        {
            eprintln!("[quic {}] {line}", self.frames);
        }
        if let Some(secs) = self.leave_in_battle {
            if b.local_activity() != 2 {
                self.battle_since = None;
            } else if self
                .battle_since
                .get_or_insert_with(Instant::now)
                .elapsed()
                .as_secs_f64()
                >= secs
            {
                self.leave_in_battle = None;
                b.leave_with("Left the world (COOP_BRIDGE_LEAVE_IN_BATTLE).");
            }
        }
    }
}

/// In the window: the ticker as a frame hook (the bridge itself runs as the window's own hook).
pub struct TickHook(pub DevTicker);

impl FrameHook for TickHook {
    fn post_frame(&mut self, _emu: &mut Emulator) {
        self.0.tick();
    }
}
