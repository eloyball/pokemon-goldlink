//! One save file per world (the bridge's SaveSlots, docs/MILESTONE2_PLAN.md 7.2), in the same
//! format and names as the BizHawk bridge: `coop_saves/CODE-UID.sav`, 512 KiB, the previous
//! copy kept as `.sav.bak`. A BizHawk install's files can be copied in as they are.
//!
//! The BizHawk bridge had to swap images through the "SRAM" domain and poll a save counter.
//! Here the world's save goes into the cart when the game boots, and [`SaveWatcher`] writes it
//! back when melonDS reports the game's save writes (`Platform::WriteNDSSave`) and the ROM's
//! save counter moves.

use std::path::{Path, PathBuf};

use crate::files::write_atomic;

/// HeartGold's save chip: 4 Mbit flash.
pub const SAVE_SIZE: usize = 524_288;

/// The file name stem of a world's save: `CODE-UID`, the room code and the relay's room id.
pub fn world_key(code: &str, uid: u32) -> String {
    format!("{code}-{uid:08X}")
}

pub struct SaveStore {
    dir: PathBuf,
}

impl SaveStore {
    /// `dir` is the `coop_saves` folder.
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self { dir: dir.into() }
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    pub fn path(&self, key: &str) -> PathBuf {
        self.dir.join(format!("{key}.sav"))
    }

    /// The world's save, or None for a world without one yet (a New Game).
    pub fn load(&self, key: &str) -> Result<Option<Vec<u8>>, String> {
        let path = self.path(key);
        match std::fs::read(&path) {
            Ok(bytes) if bytes.len() == SAVE_SIZE => Ok(Some(bytes)),
            Ok(bytes) => Err(format!(
                "{} has the wrong size ({} bytes, expected {SAVE_SIZE})",
                path.display(),
                bytes.len()
            )),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(format!("{}: {e}", path.display())),
        }
    }

    /// Writes the world's save, keeping the previous one as `.sav.bak`.
    pub fn store(&self, key: &str, image: &[u8]) -> Result<(), String> {
        let path = self.path(key);
        if path.is_file() {
            let mut bak = path.as_os_str().to_owned();
            bak.push(".bak");
            std::fs::copy(&path, PathBuf::from(bak))
                .map_err(|e| format!("can't back up {}: {e}", path.display()))?;
        }
        write_atomic(&path, image)
    }
}

/// When to write the cart's save back to the world's file: after the game saved.
///
/// The co-op ROM bumps the mailbox's `save_counter` when a save finishes; melonDS reports every
/// flash write. A save is written to disk once the counter moved and the flash has been quiet
/// for [`SaveWatcher::SETTLE_FRAMES`], or after [`SaveWatcher::QUIET_FRAMES`] of quiet without
/// the counter (a ROM without the mailbox, or a write the counter doesn't cover).
#[derive(Debug, Default)]
pub struct SaveWatcher {
    seen_writes: u32,
    seen_counter: Option<u32>,
    last_write_frame: Option<u64>,
    counter_moved: bool,
}

impl SaveWatcher {
    pub const SETTLE_FRAMES: u64 = 2;
    pub const QUIET_FRAMES: u64 = 120;

    /// Call once per frame with the number of flash writes so far, the mailbox's save counter
    /// (None without a valid mailbox) and the frame number. True: write the save now.
    pub fn update(&mut self, writes: u32, counter: Option<u32>, frame: u64) -> bool {
        if writes != self.seen_writes {
            self.seen_writes = writes;
            self.last_write_frame = Some(frame);
        }
        if let Some(c) = counter {
            if self.seen_counter.is_some_and(|seen| seen != c) {
                self.counter_moved = true;
            }
            self.seen_counter = Some(c);
        }
        let Some(last) = self.last_write_frame else {
            self.counter_moved = false;
            return false;
        };
        let quiet = frame.saturating_sub(last);
        let due =
            (self.counter_moved && quiet >= Self::SETTLE_FRAMES) || quiet >= Self::QUIET_FRAMES;
        if due {
            self.last_write_frame = None;
            self.counter_moved = false;
        }
        due
    }

    /// Unwritten flash writes (write them before quitting).
    pub fn dirty(&self) -> bool {
        self.last_write_frame.is_some()
    }

    /// The core was rebooted or a state loaded: forget the pending writes and the counter.
    pub fn reset(&mut self, writes: u32) {
        *self = Self {
            seen_writes: writes,
            ..Self::default()
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("goldlink-{name}-{}", std::process::id()));
        std::fs::remove_dir_all(&d).ok();
        d
    }

    #[test]
    fn store_load_and_backup() {
        let dir = temp("saves");
        let s = SaveStore::new(&dir);
        let key = world_key("TEST03", 0xBC71B831);
        assert_eq!(key, "TEST03-BC71B831");
        assert_eq!(s.load(&key), Ok(None));
        let a = vec![1u8; SAVE_SIZE];
        let b = vec![2u8; SAVE_SIZE];
        s.store(&key, &a).unwrap();
        s.store(&key, &b).unwrap();
        assert_eq!(s.load(&key).unwrap().unwrap(), b);
        assert_eq!(
            std::fs::read(dir.join("TEST03-BC71B831.sav.bak")).unwrap(),
            a
        );
        std::fs::write(s.path("SHORT-00000000"), [0u8; 10]).unwrap();
        assert!(s.load("SHORT-00000000").is_err());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn watcher_writes_after_the_counter_and_a_settle() {
        let mut w = SaveWatcher::default();
        assert!(!w.update(0, Some(5), 0));
        // The game writes flash over several frames, then bumps the counter.
        assert!(!w.update(10, Some(5), 1));
        assert!(!w.update(20, Some(5), 2));
        assert!(!w.update(30, Some(6), 3));
        assert!(w.dirty());
        assert!(!w.update(30, Some(6), 4));
        assert!(w.update(30, Some(6), 5));
        assert!(!w.dirty());
        assert!(!w.update(30, Some(6), 6));
    }

    #[test]
    fn watcher_falls_back_to_quiet_flash() {
        let mut w = SaveWatcher::default();
        assert!(!w.update(1, None, 10));
        assert!(!w.update(1, None, 10 + SaveWatcher::QUIET_FRAMES - 1));
        assert!(w.update(1, None, 10 + SaveWatcher::QUIET_FRAMES));
        // A counter that moves without flash writes writes nothing.
        let mut w = SaveWatcher::default();
        w.update(0, Some(1), 0);
        assert!(!w.update(0, Some(2), 1));
        assert!(!w.update(0, Some(2), 200));
    }
}
