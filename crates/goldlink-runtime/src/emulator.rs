//! One running game: the DS, its ROM and the cart's save file, frame by frame. No threads, no
//! window: the window app drives it through [`crate::runner`], and tests and the dev binary call
//! [`Emulator::frame`] directly.
//!
//! The cart's save file is what the cartridge holds across restarts (BizHawk's `.SaveRAM`). The
//! co-op bridge keeps one save per world on top of it (`goldlink-bridge`'s save slots), swapping
//! the world's save into the cart the way the BizHawk bridge does.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

use goldlink_abi::{COOP_MAGIC, COOP_MAILBOX_VERSION, CoopCommBlock};
use goldlink_core::{FirmwareUser, Keys, Nds};

use crate::saves::{self, SaveWatcher};

/// The mailbox's size (`gCoopCommBlock`).
pub const MAILBOX_SIZE: usize = std::mem::size_of::<CoopCommBlock>();

/// Where NitroSDK keeps the reset parameter across a reset (`HW_RESET_PARAMETER_BUF`).
const RESET_PARAMETER: usize = 0x027F_FC20;
const MAIN_RAM_SIZE: usize = 0x40_0000;

/// What a game needs to start.
pub struct GameSetup {
    /// The patched ROM, in memory.
    pub rom: Arc<Vec<u8>>,
    /// The cart's save file: loaded at boot, written after every in-game save (the previous one
    /// kept as `.bak`). None: a blank cart that is never written (tests).
    pub cart_save: Option<PathBuf>,
    /// Main-RAM offset of the mailbox (coop_offsets.json `main_ram_offset`; `release.json` later).
    pub mailbox_offset: usize,
    /// The DS clock at boot; None = the host's time (BizHawk's `UseRealTime`).
    pub rtc: Option<chrono::NaiveDateTime>,
}

/// One frame's input.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Input {
    pub keys: Keys,
    /// Stylus on the bottom screen at (x, y), 0..256 x 0..192.
    pub touch: Option<(u16, u16)>,
}

/// Something worth telling the player.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    Saved,
    SaveFailed(String),
    BackupTaken(PathBuf),
    BackupFailed(String),
}

pub struct Emulator {
    pub nds: Nds,
    rom: Arc<Vec<u8>>,
    cart_save: Option<PathBuf>,
    mailbox_offset: usize,
    rtc: Option<chrono::NaiveDateTime>,
    watcher: SaveWatcher,
    flash_writes: Arc<AtomicU32>,
    frame: u64,
    events: Vec<Event>,
}

impl Emulator {
    /// Creates the DS and boots the game on the cart's save.
    pub fn new(setup: GameSetup) -> Result<Self, String> {
        let flash_writes = Arc::new(AtomicU32::new(0));
        let counter = flash_writes.clone();
        let hook: goldlink_core::SaveHook = Box::new(move |_, _, _| {
            counter.fetch_add(1, Ordering::Relaxed);
        });
        let nds = Nds::new(&FirmwareUser::default(), Some(hook)).map_err(|e| e.to_string())?;
        let mut emu = Self {
            nds,
            rom: setup.rom,
            cart_save: setup.cart_save,
            mailbox_offset: setup.mailbox_offset,
            rtc: setup.rtc,
            watcher: SaveWatcher::default(),
            flash_writes,
            frame: 0,
            events: Vec::new(),
        };
        emu.boot()?;
        Ok(emu)
    }

    /// Power cycle: the cart gets its save file as it is on disk.
    pub fn boot(&mut self) -> Result<(), String> {
        let save = match &self.cart_save {
            Some(p) => saves::load(p)?,
            None => None,
        };
        self.nds
            .load_rom(&self.rom, save.as_deref())
            .map_err(|e| e.to_string())?;
        match self.rtc {
            Some(at) => self.nds.set_rtc(at),
            None => self.nds.set_rtc_now(),
        }
        self.watcher
            .reset(self.flash_writes.load(Ordering::Relaxed));
        Ok(())
    }

    /// Power cycle straight into the game (dev/docs/unified-client/UC9_PROPOSAL.md §2): the cart
    /// keeps what it holds now (the bridge may just have put a world's save in), and the game boots
    /// with reset parameter 1, its own boot into Continue (pret main.c): no intro, title or main
    /// menu. The co-op ROM turns it into New Game on a blank cart (`Coop_RegisterMainOverlay`).
    pub fn boot_into_game(&mut self) -> Result<(), String> {
        let cart = self.nds.save().to_vec();
        self.nds
            .load_rom(&self.rom, (!cart.is_empty()).then_some(cart.as_slice()))
            .map_err(|e| e.to_string())?;
        match self.rtc {
            Some(at) => self.nds.set_rtc(at),
            None => self.nds.set_rtc_now(),
        }
        self.watcher
            .reset(self.flash_writes.load(Ordering::Relaxed));
        // NitroSDK's HW_RESET_PARAMETER_BUF (0x027FFC20), which OS_GetResetParameter reads; main
        // RAM is 4 MB, mirrored up to 0x02800000.
        let at = RESET_PARAMETER & (MAIN_RAM_SIZE - 1);
        self.nds.main_ram_mut()[at..at + 4].copy_from_slice(&1u32.to_le_bytes());
        Ok(())
    }

    /// The cart's save file.
    pub fn cart_save(&self) -> Option<&Path> {
        self.cart_save.as_deref()
    }

    pub fn frame_count(&self) -> u64 {
        self.frame
    }

    /// Runs one frame with this input, then writes the cart's save file if the game just saved.
    pub fn frame(&mut self, input: Input) {
        self.nds.set_input(input.keys, input.touch);
        self.nds.run_frame();
        self.frame += 1;
        self.after_frame();
    }

    fn after_frame(&mut self) {
        let counter = self.mailbox().map(|mb| {
            let at = std::mem::offset_of!(CoopCommBlock, save_counter);
            u32::from_le_bytes(mb[at..at + 4].try_into().unwrap())
        });
        let writes = self.flash_writes.load(Ordering::Relaxed);
        if self.watcher.update(writes, counter, self.frame) {
            self.write_save();
        }
    }

    fn write_save(&mut self) {
        let Some(path) = &self.cart_save else { return };
        let event = match saves::store(path, self.nds.save()) {
            Ok(()) => Event::Saved,
            Err(e) => Event::SaveFailed(e),
        };
        self.events.push(event);
    }

    /// Writes the cart's save file now (after a hook replaced the save memory, or on request).
    pub fn write_cart_save(&mut self) {
        self.watcher
            .reset(self.flash_writes.load(Ordering::Relaxed));
        self.write_save();
    }

    /// Writes save memory the game changed and that isn't on disk yet (before quitting).
    pub fn flush(&mut self) {
        if self.watcher.dirty() {
            self.watcher
                .reset(self.flash_writes.load(Ordering::Relaxed));
            self.write_save();
        }
    }

    /// Restores a backup file (or any savestate of this ROM).
    pub fn load_state_file(&mut self, path: &Path) -> Result<(), String> {
        let state = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
        self.nds.load_savestate(&state).map_err(|e| e.to_string())?;
        self.watcher
            .reset(self.flash_writes.load(Ordering::Relaxed));
        Ok(())
    }

    /// The validated mailbox (magic and version), as its 512 bytes of main RAM.
    pub fn mailbox(&self) -> Option<&[u8]> {
        let mb = self
            .nds
            .main_ram()
            .get(self.mailbox_offset..self.mailbox_offset + MAILBOX_SIZE)?;
        let magic = u32::from_le_bytes(mb[0..4].try_into().unwrap());
        let version = u16::from_le_bytes([mb[4], mb[5]]);
        (magic == COOP_MAGIC && u32::from(version) == COOP_MAILBOX_VERSION).then_some(mb)
    }

    pub fn mailbox_offset(&self) -> usize {
        self.mailbox_offset
    }

    /// Reports something to the window (hooks: the bridge's backups, its save slots).
    pub fn push_event(&mut self, event: Event) {
        self.events.push(event);
    }

    /// Takes what happened since the last call.
    pub fn take_events(&mut self) -> Vec<Event> {
        std::mem::take(&mut self.events)
    }

    /// Both screens, top then bottom: 256 x 384 pixels, 0x00RRGGBB.
    pub fn screens(&self, out: &mut Vec<u32>) {
        out.clear();
        out.extend_from_slice(self.nds.framebuffer(0));
        out.extend_from_slice(self.nds.framebuffer(1));
    }
}

/// FNV-1a over both screens: a framebuffer hash for replay tests.
pub fn screen_hash(emu: &Emulator) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for screen in 0..2 {
        for px in emu.nds.framebuffer(screen) {
            for b in px.to_le_bytes() {
                h ^= u64::from(b);
                h = h.wrapping_mul(0x0100_0000_01b3);
            }
        }
    }
    h
}
