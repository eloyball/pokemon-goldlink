//! `goldlink-dev --window`: the game in the release window with a stand-in for the co-op bridge,
//! so it can be played before the Rust bridge exists (UC3). It does what the Lua probes do when
//! they "play the bridge": heartbeat, connected to a world, the world's save loaded, and the
//! roster locked (the adventure starts). No relay, no other players. Dev only (D2).

use std::mem::offset_of;
use std::path::PathBuf;
use std::sync::Arc;

use goldlink_abi::{CoopCommBlock, CoopProgressionGates};
use goldlink_runtime::{Emulator, FrameHook, GameSetup};

pub struct OfflineWorld {
    heartbeat: u32,
}

impl FrameHook for OfflineWorld {
    fn post_frame(&mut self, emu: &mut Emulator) {
        if emu.mailbox().is_none() {
            return;
        }
        self.heartbeat = self.heartbeat.wrapping_add(1);
        let at = emu.mailbox_offset();
        let ram = emu.nds.main_ram_mut();
        let mut put =
            |off: usize, bytes: &[u8]| ram[at + off..at + off + bytes.len()].copy_from_slice(bytes);
        put(
            offset_of!(CoopCommBlock, heartbeat_bridge),
            &self.heartbeat.to_le_bytes(),
        );
        put(offset_of!(CoopCommBlock, is_connected_to_room), &[1]);
        put(offset_of!(CoopCommBlock, save_slot_ready), &[1]);
        put(
            offset_of!(CoopCommBlock, gates)
                + offset_of!(CoopProgressionGates, flag_campaign_started),
            &[1],
        );
    }
}

/// Opens the window on `rom` with the offline world. The cart's save is
/// `<data>/coop_saves/<world>.sav`.
pub fn run(
    rom: PathBuf,
    data: PathBuf,
    world: String,
    rtc: Option<chrono::NaiveDateTime>,
) -> Result<(), String> {
    let bytes = std::fs::read(&rom).map_err(|e| format!("{}: {e}", rom.display()))?;
    let setup = GameSetup {
        rom: Arc::new(bytes),
        cart_save: Some(data.join("coop_saves").join(format!("{world}.sav"))),
        mailbox_offset: crate::MAILBOX,
        rtc,
    };
    goldlink::run(goldlink::Session {
        setup,
        data_dir: data,
        bridge: None,
        hooks: vec![Box::new(OfflineWorld { heartbeat: 0 })],
        title: "GoldLink (dev, offline world)".into(),
    })
}
