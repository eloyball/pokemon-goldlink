//! The bridge on the emulation thread: a [`FrameHook`] around a shared [`Bridge`] (the window
//! reads and drives it through the same lock).

use std::sync::{Arc, Mutex, MutexGuard};

use goldlink_core::Nds;
use goldlink_runtime::{Emulator, Event, FrameHook};

use crate::bridge::{Bridge, Machine};

struct NdsMachine<'a>(&'a mut Nds);

impl Machine for NdsMachine<'_> {
    fn main_ram(&mut self) -> &mut [u8] {
        self.0.main_ram_mut()
    }
    fn cart(&mut self) -> &mut [u8] {
        self.0.save_mut()
    }
    fn savestate(&mut self) -> Option<Vec<u8>> {
        self.0.savestate()
    }
    fn load_savestate(&mut self, state: &[u8]) -> Result<(), String> {
        self.0.load_savestate(state).map_err(|e| e.to_string())
    }
}

pub type SharedBridge = Arc<Mutex<Bridge>>;

pub fn lock(b: &SharedBridge) -> MutexGuard<'_, Bridge> {
    b.lock().unwrap_or_else(|e| e.into_inner())
}

/// Runs the bridge's before-frame work on a bare DS (the dev binary's Lua runs).
pub fn before_frame(bridge: &SharedBridge, nds: &mut Nds, mailbox: usize) {
    lock(bridge).pre_frame(nds.main_ram_mut(), mailbox);
}

/// Runs the bridge's after-frame work on a bare DS (the dev binary's Lua runs): what
/// [`BridgeHook`] does on the emulation thread.
pub fn after_frame(bridge: &SharedBridge, nds: &mut Nds, mailbox: usize) -> crate::Actions {
    lock(bridge).post_frame(&mut NdsMachine(nds), mailbox)
}

pub struct BridgeHook {
    bridge: SharedBridge,
}

impl BridgeHook {
    /// The bridge on this emulation thread, which power-cycles straight into the game when the
    /// bridge asks (UC9 §2).
    pub fn new(bridge: SharedBridge) -> Self {
        lock(&bridge).set_boots_into_game(true);
        Self { bridge }
    }
}

impl FrameHook for BridgeHook {
    fn pre_frame(&mut self, emu: &mut Emulator, _input: &mut goldlink_runtime::Input) {
        let mailbox = emu.mailbox_offset();
        let moved = {
            let mut b = lock(&self.bridge);
            b.wide_side = emu.wide().min(usize::from(u8::MAX)) as u8;
            b.pre_frame(emu.nds.main_ram_mut(), mailbox);
            b.moved_boxes()
        };
        // The steady overworld (UC11) only in the overworld: battles and menus keep their own
        // cameras.
        use crate::bridge::Scene;
        let scene = emu
            .mailbox()
            .map_or(Scene::Unknown, crate::bridge::scene_of);
        emu.set_in_field(matches!(scene, Scene::Overworld | Scene::StartMenu));

        // The wider picture: the game's HP boxes out toward its edges, from this frame on.
        emu.nds.set_sprite_move(&moved);
    }

    fn post_frame(&mut self, emu: &mut Emulator) {
        let mailbox = emu.mailbox_offset();
        if !lock(&self.bridge).has_game_font() {
            let font = crate::menu_text::GameFont::read(emu.rom());
            lock(&self.bridge).set_game_font(font);
        }
        if !lock(&self.bridge).has_game_text() {
            let text = crate::battle_ui::GameText::read(emu.rom());
            lock(&self.bridge).set_game_text(text);
        }
        let actions = lock(&self.bridge).post_frame(&mut NdsMachine(&mut emu.nds), mailbox);
        if actions.cart_replaced {
            emu.write_cart_save();
        }
        if actions.boot_into_game
            && let Err(e) = emu.boot_into_game()
        {
            emu.push_event(Event::SaveFailed(format!("the game couldn't restart: {e}")));
        }
        if let Some(p) = actions.backup_taken {
            emu.push_event(Event::BackupTaken(p));
        }
        if let Some(e) = actions.backup_failed {
            emu.push_event(Event::BackupFailed(e));
        }
        if actions.game_hung {
            let dir = lock(&self.bridge).dir().join("crashes");
            write_hang_report(emu, &dir);
        }
    }
}

/// A game that stopped responding (the bridge's hang watch): `crashes/hang-<time>.json` (the
/// build, the player's place, both CPUs' registers, the mailbox) and beside it the whole machine
/// as a savestate (`.mlds`, loadable by the dev tour's LoadState with the same ROM), so where the
/// ARM9 is stuck can be read afterwards (dev/docs: the crash triage by the ARM9's PC).
fn write_hang_report(emu: &mut goldlink_runtime::Emulator, dir: &std::path::Path) {
    use goldlink_abi::{CoopCommBlock, CoopPlayerState};
    use std::mem::offset_of;
    let stamp = chrono::Local::now().format("%Y%m%d-%H%M%S").to_string();
    let regs = emu.nds.registers(9);
    log::error!(
        "the game stopped responding: ARM9 PC {:08X} LR {:08X}; report crashes/hang-{stamp}.json",
        regs[15],
        regs[14]
    );
    let hex = |r: [u32; 17]| r.iter().map(|v| format!("{v:08X}")).collect::<Vec<_>>();
    let player = emu.mailbox().map(|mb| {
        let p = offset_of!(CoopCommBlock, local_player);
        let u16_at = |o: usize| u16::from_le_bytes([mb[p + o], mb[p + o + 1]]);
        serde_json::json!({
            "map": u16_at(offset_of!(CoopPlayerState, map_id)),
            "x": u16_at(offset_of!(CoopPlayerState, x)) as i16,
            "z": u16_at(offset_of!(CoopPlayerState, z)) as i16,
            "activity": mb[p + offset_of!(CoopPlayerState, activity)],
        })
    });
    let mailbox: Option<String> = emu
        .mailbox()
        .map(|mb| mb.iter().map(|b| format!("{b:02X}")).collect());
    let report = serde_json::json!({
        "kind": "the game stopped responding (its main loop stood still; the emulator ran on)",
        "build": option_env!("GOLDLINK_BUILD").unwrap_or("dev"),
        "os": std::env::consts::OS,
        "time": chrono::Utc::now().to_rfc3339(),
        "frames": emu.frame_count(),
        "player": player,
        // r0..r15 then CPSR.
        "arm9": hex(emu.nds.registers(9)),
        "arm7": hex(emu.nds.registers(7)),
        "mailbox": mailbox,
    });
    let _ = std::fs::create_dir_all(dir);
    if let Ok(text) = serde_json::to_string_pretty(&report) {
        let _ = goldlink_runtime::write_atomic(
            &dir.join(format!("hang-{stamp}.json")),
            text.as_bytes(),
        );
    }
    if let Some(state) = emu.nds.savestate() {
        let path = dir.join(format!("hang-{stamp}.mlds"));
        // Some 20 MB: off the emulation thread.
        std::thread::spawn(move || {
            let _ = goldlink_runtime::write_atomic(&path, &state);
        });
    }
    emu.push_event(Event::GameHung);
}
