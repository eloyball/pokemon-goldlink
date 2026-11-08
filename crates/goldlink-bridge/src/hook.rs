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
    pub bridge: SharedBridge,
}

impl FrameHook for BridgeHook {
    fn pre_frame(&mut self, emu: &mut Emulator, _input: &mut goldlink_runtime::Input) {
        let mailbox = emu.mailbox_offset();
        lock(&self.bridge).pre_frame(emu.nds.main_ram_mut(), mailbox);
    }

    fn post_frame(&mut self, emu: &mut Emulator) {
        let mailbox = emu.mailbox_offset();
        let actions = lock(&self.bridge).post_frame(&mut NdsMachine(&mut emu.nds), mailbox);
        if actions.cart_replaced {
            emu.write_cart_save();
        }
        if let Some(p) = actions.backup_taken {
            emu.push_event(Event::BackupTaken(p));
        }
        if let Some(e) = actions.backup_failed {
            emu.push_event(Event::BackupFailed(e));
        }
    }
}
