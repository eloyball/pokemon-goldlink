//! The dev panel in `goldlink-dev --window` (F12): the C# bridge's diagnostics card and debug
//! commands (`#if !COOP_SHIP`), never in the release (D2). The ROM handles the commands in its
//! Overlay 131 field entry; a dev ROM is needed (`COOP_DEBUG_TOOLS 1`), a release ROM answers
//! "disabled".

use std::mem::offset_of;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use eframe::egui;
use goldlink::goldlink_bridge::layout::*;
use goldlink::goldlink_bridge::text::player_state_name;
use goldlink_abi::CoopDebugCommand as C;
use goldlink_abi::CoopDiagnostics as D;
use goldlink_runtime::{Emulator, FrameHook};

/// CoopWarpPreset, in order (coop_sync.h; the C# bridge's list).
const WARPS: [&str; 14] = [
    "New Bark Town (Route 29 border, water)",
    "Cherrygrove City (beach)",
    "Violet City (route borders)",
    "Goldenrod City (dense)",
    "Ecruteak City (dense)",
    "Olivine City (harbor)",
    "Route 40 (beach, sea)",
    "Cerulean City (Nugget Bridge north)",
    "Lavender Town (Silence Bridge south)",
    "Celadon City (dense)",
    "Violet Gym (gym gate, Falkner ahead)",
    "Lance's room (team boss by coord event)",
    "Elm's lab (below the starter machine)",
    "Route 29 (below the Potion ball)",
];

#[derive(Default)]
struct State {
    pending: Option<(u8, u8, String)>,
    seq: u8,
    sent: String,
    diag: Vec<u8>,
    result: u8,
    avatar_state: u8,
    calls: u32,
    calls_changed: Option<Instant>,
}

#[derive(Clone, Default)]
pub struct DevPanel(Arc<Mutex<State>>);

/// Writes queued commands to the mailbox and copies what the panel shows, on the emulation
/// thread.
pub struct DevHook(DevPanel);

impl DevPanel {
    pub fn hook(&self) -> DevHook {
        DevHook(self.clone())
    }

    fn state(&self) -> std::sync::MutexGuard<'_, State> {
        self.0.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// The panel's UI; `open` toggles with F12.
    pub fn ui(&self, ctx: &egui::Context, open: &mut bool, warp: &mut usize) {
        if ctx.input(|i| i.key_pressed(egui::Key::F12)) {
            *open = !*open;
        }
        if !*open {
            return;
        }
        let mut s = self.state();
        let d = s.diag.clone();
        egui::Window::new("Dev").default_pos([8.0, 8.0]).open(open).show(ctx, |ui| {
            if d.len() < std::mem::size_of::<D>() {
                ui.label("Waiting for the co-op ROM…");
                return;
            }
            let calls = u32_at(&d, offset_of!(D, field_entry_calls));
            let ticking = s.calls_changed.is_some_and(|t| t.elapsed().as_millis() < 500);
            let status = d[offset_of!(D, field_entry_status)];
            let running = status == 3 && ticking;
            ui.label(match status {
                3 if ticking => format!("Field code: running · {calls} frames"),
                3 => format!("Field code: stalled · {calls} frames"),
                2 => "Field code: error, Overlay 131 has no co-op header (rebuild)".into(),
                1 => "Field code: error, Overlay 131 isn't loaded".into(),
                _ if calls == 0 => "Field code: waiting for the overworld".into(),
                _ => format!("Field code: paused (battle, menu or transition) · {calls} frames"),
            });
            if running {
                ui.label(format!("Player: {}", player_state_name(s.avatar_state)));
            }
            if d[offset_of!(D, render_ready)] == 1 {
                ui.label(format!(
                    "Textures {} of {} free · {} resident; models {} of {} free; sequences {} of {} free; objects {} of 64",
                    d[offset_of!(D, tex_dynamic_free)],
                    d[offset_of!(D, tex_dynamic)],
                    d[offset_of!(D, tex_resident)],
                    d[offset_of!(D, model_dynamic_free)],
                    d[offset_of!(D, model_dynamic)],
                    d[offset_of!(D, seq_dynamic_free)],
                    d[offset_of!(D, seq_dynamic)],
                    d[offset_of!(D, map_objects_active)],
                ));
            }
            let ghost = d[offset_of!(D, test_ghost_state)];
            let at = |o: usize| u16_at(&d, o) as i16;
            if ghost != 0 {
                ui.label(format!(
                    "Test ghost: {} at ({}, {}), height {} (you {})",
                    if ghost == 2 { "moving" } else { "idle" },
                    at(offset_of!(D, test_ghost_x)),
                    at(offset_of!(D, test_ghost_z)),
                    at(offset_of!(D, test_ghost_y)),
                    at(offset_of!(D, player_y)),
                ));
            }
            ui.separator();
            let queue = |s: &mut State, cmd: u32, arg: u8, name: &str| {
                s.pending = Some((cmd as u8, arg, name.to_owned()));
            };
            ui.add_enabled_ui(running, |ui| {
                ui.horizontal_wrapped(|ui| {
                    for (cmd, name) in [
                        (C::COOP_DEBUG_SPAWN_TEST_GHOST, "Spawn ghost"),
                        (C::COOP_DEBUG_STEP_TEST_GHOST, "Step ghost"),
                        (C::COOP_DEBUG_EMOTE_TEST_GHOST, "Emote"),
                        (C::COOP_DEBUG_REMOVE_TEST_GHOST, "Remove ghost"),
                        (C::COOP_DEBUG_TOGGLE_BIKE, "Bike"),
                        (C::COOP_DEBUG_SURF, "Surf"),
                        (C::COOP_DEBUG_FISH, "Fish"),
                    ] {
                        if ui.button(name).clicked() {
                            queue(&mut s, cmd, 0, name);
                        }
                    }
                });
                ui.horizontal(|ui| {
                    egui::ComboBox::from_id_salt("warp")
                        .width(260.0)
                        .selected_text(WARPS[*warp])
                        .show_ui(ui, |ui| {
                            for (i, w) in WARPS.iter().enumerate() {
                                ui.selectable_value(warp, i, *w);
                            }
                        });
                    if ui.button("Warp").clicked() {
                        let name = format!("Warp to {}", WARPS[*warp]);
                        queue(&mut s, C::COOP_DEBUG_WARP, *warp as u8, &name);
                    }
                });
            });
            let ack = d[offset_of!(D, debug_ack)];
            if s.seq != 0 {
                let line = if ack != s.seq || s.pending.is_some() {
                    format!("{}: waiting (event, message or step in progress)", s.sent)
                } else {
                    let r = match s.result {
                        1 => "done",
                        2 => "nothing to act on (spawn the ghost first)",
                        3 => "not now (wrong riding state)",
                        4 => "face surfable water first",
                        5 => "unknown command or preset",
                        6 => "failed (no free object or memory)",
                        7 => "debug tools are compiled out of this ROM",
                        _ => "—",
                    };
                    format!("{}: {r}", s.sent)
                };
                ui.label(line);
            }
        });
    }
}

impl FrameHook for DevHook {
    fn post_frame(&mut self, emu: &mut Emulator) {
        let at = emu.mailbox_offset();
        let mut s = self.0.state();
        if emu.mailbox().is_none() {
            s.diag.clear();
            return;
        }
        if let Some((cmd, arg, name)) = s.pending.take() {
            s.seq = if s.seq == 255 { 1 } else { s.seq + 1 };
            let ram = emu.nds.main_ram_mut();
            ram[at + DEBUG_CMD] = cmd;
            ram[at + DEBUG_ARG] = arg;
            ram[at + DEBUG_SEQ] = s.seq;
            s.sent = name;
        }
        let ram = emu.nds.main_ram();
        let diag = &ram[at + DIAG..at + DIAG + std::mem::size_of::<D>()];
        let calls = u32_at(diag, offset_of!(D, field_entry_calls));
        if s.diag.len() < 4 || calls != u32_at(&s.diag, offset_of!(D, field_entry_calls)) {
            s.calls = calls;
            s.calls_changed = Some(Instant::now());
        }
        s.diag = diag.to_vec();
        s.result = ram[at + DEBUG_RESULT];
        s.avatar_state = ram[at + LOCAL_PLAYER + PS_AVATAR_STATE];
    }
}
