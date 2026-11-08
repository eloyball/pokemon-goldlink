//! One running game: the DS, its ROM, the world's save file and backups, frame by frame. No
//! threads, no window: the window app drives it through [`crate::runner`], and tests and the
//! dev binary call [`Emulator::frame`] directly.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Instant;

use goldlink_abi::{COOP_MAGIC, COOP_MAILBOX_VERSION, CoopCommBlock};
use goldlink_core::{FirmwareUser, Keys, Nds};

use crate::backups::{self, Backups};
use crate::saves::{SaveStore, SaveWatcher};

/// The mailbox's size (`gCoopCommBlock`).
pub const MAILBOX_SIZE: usize = std::mem::size_of::<CoopCommBlock>();

/// What a game needs to start.
pub struct GameSetup {
    /// The patched ROM, in memory.
    pub rom: Arc<Vec<u8>>,
    /// The data folder: `coop_saves/`, `coop_backups/` and the settings live here.
    pub data_dir: PathBuf,
    /// The world whose save the game runs on (`CODE-UID`).
    pub world: String,
    /// Main-RAM offset of the mailbox (coop_offsets.json `main_ram_offset`; `release.json` later).
    pub mailbox_offset: usize,
    /// The DS clock at boot; None = the host's time (BizHawk's `UseRealTime`).
    pub rtc: Option<chrono::NaiveDateTime>,
    /// Take emergency backups (off in tests).
    pub backups: bool,
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
    world: String,
    mailbox_offset: usize,
    rtc: Option<chrono::NaiveDateTime>,
    saves: SaveStore,
    backups: Option<Backups>,
    next_backup: Option<Instant>,
    watcher: SaveWatcher,
    flash_writes: Arc<AtomicU32>,
    frame: u64,
    events: Vec<Event>,
}

impl Emulator {
    /// Creates the DS and boots the game on the world's save.
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
            world: setup.world,
            mailbox_offset: setup.mailbox_offset,
            rtc: setup.rtc,
            saves: SaveStore::new(setup.data_dir.join("coop_saves")),
            backups: setup
                .backups
                .then(|| Backups::new(setup.data_dir.join("coop_backups"))),
            next_backup: None,
            watcher: SaveWatcher::default(),
            flash_writes,
            frame: 0,
            events: Vec::new(),
        };
        emu.boot()?;
        Ok(emu)
    }

    /// Power cycle: the cart gets the world's save file as it is on disk.
    pub fn boot(&mut self) -> Result<(), String> {
        let save = self.saves.load(&self.world)?;
        self.nds
            .load_rom(&self.rom, save.as_deref())
            .map_err(|e| e.to_string())?;
        match self.rtc {
            Some(at) => self.nds.set_rtc(at),
            None => self.nds.set_rtc_now(),
        }
        self.watcher
            .reset(self.flash_writes.load(Ordering::Relaxed));
        self.next_backup = None;
        Ok(())
    }

    pub fn world(&self) -> &str {
        &self.world
    }

    pub fn save_path(&self) -> PathBuf {
        self.saves.path(&self.world)
    }

    pub fn frame_count(&self) -> u64 {
        self.frame
    }

    /// Runs one frame with this input, then writes the save if the game just saved and takes a
    /// backup if one is due.
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
        self.maybe_backup();
    }

    fn write_save(&mut self) {
        let event = match self.saves.store(&self.world, self.nds.save()) {
            Ok(()) => Event::Saved,
            Err(e) => Event::SaveFailed(e),
        };
        self.events.push(event);
    }

    /// Writes save memory the game changed and that isn't on disk yet (before quitting).
    pub fn flush(&mut self) {
        if self.watcher.dirty() {
            self.watcher
                .reset(self.flash_writes.load(Ordering::Relaxed));
            self.write_save();
        }
    }

    fn maybe_backup(&mut self) {
        if self.backups.is_none() {
            return;
        }
        let now = Instant::now();
        let due = *self.next_backup.get_or_insert(now + backups::INTERVAL);
        if now < due || !self.mailbox().is_some_and(backups::safe_moment) {
            return;
        }
        self.next_backup = Some(now + backups::INTERVAL);
        let event = match self.nds.savestate() {
            Some(state) => match self.backups.as_ref().unwrap().write(&self.world, &state) {
                Ok(path) => Event::BackupTaken(path),
                Err(e) => Event::BackupFailed(e),
            },
            None => Event::BackupFailed("the emulator couldn't make a savestate".into()),
        };
        self.events.push(event);
    }

    /// The world's backups, newest first.
    pub fn backups(&self) -> Vec<backups::Backup> {
        self.backups
            .as_ref()
            .map(|b| b.list(&self.world))
            .unwrap_or_default()
    }

    /// Restores a backup file (or any savestate of this ROM).
    pub fn load_state_file(&mut self, path: &Path) -> Result<(), String> {
        let state = Backups::read(path)?;
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
