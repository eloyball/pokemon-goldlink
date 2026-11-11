//! One save file per world (the C# bridge's SaveSlots.cs, docs/MILESTONE2_PLAN.md 7.2), same
//! files: `coop_saves/CODE-UID.sav` (+ `.bak`) and `sram_owner.txt`, which records which world
//! the image in the cart belongs to and the hash of the last image written or copied, so progress
//! made while the bridge wasn't watching is never lost.

use std::path::{Path, PathBuf};

use sha1::{Digest, Sha1};

pub use goldlink_runtime::saves::SAVE_SIZE;

const OWNER_FILE: &str = "sram_owner.txt";

pub struct SaveSlots {
    dir: PathBuf,
    owner: String,
    hash: String,
    pub last_error: String,
}

pub fn room_key(code: &str, uid: u32) -> String {
    goldlink_runtime::saves::world_key(code, uid)
}

pub fn hash(image: &[u8]) -> String {
    Sha1::digest(image)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// Erased flash: no save, New Game only.
pub fn blank() -> Vec<u8> {
    vec![0xFF; SAVE_SIZE]
}

fn is_blank(image: &[u8]) -> bool {
    match image.first() {
        Some(&first) if first == 0xFF || first == 0x00 => image.iter().all(|&b| b == first),
        _ => false,
    }
}

impl SaveSlots {
    /// `dir` is the `coop_saves` folder.
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        let dir = dir.into();
        let (mut owner, mut hash) = (String::new(), String::new());
        if let Ok(text) = std::fs::read_to_string(dir.join(OWNER_FILE)) {
            let lines: Vec<&str> = text.lines().collect();
            if lines.len() >= 2 {
                owner = lines[0].trim().to_owned();
                hash = lines[1].trim().to_owned();
            }
        }
        Self {
            dir,
            owner,
            hash,
            last_error: String::new(),
        }
    }

    /// The world the cart's image belongs to ("" = unknown).
    pub fn owner(&self) -> &str {
        &self.owner
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    pub fn room_file(&self, key: &str) -> PathBuf {
        self.dir.join(format!("{key}.sav"))
    }

    pub fn has_room_save(&self, key: &str) -> bool {
        self.room_file(key).is_file()
    }

    /// This computer has this world's save: its file, or the cart holding the world's image.
    pub fn has_save_for(&self, key: &str, cart: &[u8]) -> bool {
        self.has_room_save(key) || (self.owner == key && !is_blank(cart))
    }

    /// Picks the image for a world the player just joined (at the title screen, before a save is
    /// in play). Keeps the cart's image first: for its owner world when it changed since the
    /// bridge last saw it, or as `unassigned-*.sav` when its owner is unknown. Returns the image
    /// to put in the cart, or None when the cart already holds the world's save.
    pub fn prepare_join(&mut self, key: &str, cart: &[u8]) -> Option<Vec<u8>> {
        let current = hash(cart);
        // A blank cart holds no progress: never kept over the owner's file (an imported save
        // must survive a join that finds the emulator's empty cart, UC8 §2.3).
        if !self.owner.is_empty() && current != self.hash && !is_blank(cart) {
            let owner_file = self.room_file(&self.owner.clone());
            self.store(&owner_file, cart);
        } else if self.owner.is_empty() && !is_blank(cart) {
            let stamp = chrono::Local::now().format("%Y%m%d-%H%M%S");
            self.store(&self.dir.join(format!("unassigned-{stamp}.sav")), cart);
        }

        let mut target = blank();
        let file = self.room_file(key);
        if file.is_file() {
            match std::fs::read(&file) {
                Ok(saved) if saved.len() == SAVE_SIZE => target = saved,
                Ok(_) => {
                    self.last_error = format!(
                        "{} has the wrong size; starting a new save.",
                        file.file_name().unwrap_or_default().to_string_lossy()
                    )
                }
                Err(e) => self.last_error = e.to_string(),
            }
        }

        let wanted = hash(&target);
        self.owner = key.to_owned();
        self.hash = wanted.clone();
        self.save_owner();
        (wanted != current).then_some(target)
    }

    /// The game finished writing a save: keep it for the world that owns the cart.
    pub fn capture(&mut self, cart: &[u8]) {
        if self.owner.is_empty() {
            return;
        }
        let file = self.room_file(&self.owner.clone());
        self.store(&file, cart);
        self.hash = hash(cart);
        self.save_owner();
    }

    fn store(&mut self, path: &Path, image: &[u8]) {
        match goldlink_runtime::saves::store(path, image) {
            Ok(()) => self.last_error.clear(),
            Err(e) => self.last_error = e,
        }
    }

    fn save_owner(&mut self) {
        let text = format!("{}\n{}\n", self.owner, self.hash);
        if let Err(e) = goldlink_runtime::write_atomic(&self.dir.join(OWNER_FILE), text.as_bytes())
        {
            self.last_error = e;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("goldlink-slots-{name}-{}", std::process::id()));
        std::fs::remove_dir_all(&d).ok();
        d
    }

    #[test]
    fn join_a_new_world_with_a_blank_cart() {
        let dir = temp("new");
        let mut s = SaveSlots::new(&dir);
        // Blank cart, new world: the target is blank too, so nothing to swap.
        assert_eq!(s.prepare_join("AAAAAA-00000001", &blank()), None);
        assert_eq!(s.owner(), "AAAAAA-00000001");
        // The game saves: captured for the world.
        let save = vec![3u8; SAVE_SIZE];
        s.capture(&save);
        assert_eq!(std::fs::read(s.room_file("AAAAAA-00000001")).unwrap(), save);
        // The owner survives a restart.
        let s2 = SaveSlots::new(&dir);
        assert_eq!(s2.owner(), "AAAAAA-00000001");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn switching_worlds_keeps_both_saves() {
        let dir = temp("switch");
        let mut s = SaveSlots::new(&dir);
        let a = vec![1u8; SAVE_SIZE];
        s.prepare_join("AAAAAA-00000001", &blank());
        s.capture(&a);
        // Play on without the bridge seeing it (the cart changed after the last capture).
        let a2 = vec![2u8; SAVE_SIZE];
        // Join B: A's newer image is kept for A, B starts blank.
        let img = s.prepare_join("BBBBBB-00000002", &a2).unwrap();
        assert_eq!(img, blank());
        assert_eq!(std::fs::read(s.room_file("AAAAAA-00000001")).unwrap(), a2);
        // Back to A: its save comes back.
        let back = s.prepare_join("AAAAAA-00000001", &blank()).unwrap();
        assert_eq!(back, a2);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn has_save_for_the_file_or_the_worlds_cart() {
        let dir = temp("has");
        let mut s = SaveSlots::new(&dir);
        assert!(!s.has_save_for("WORLD1-0A1B2C3D", &blank()));
        // A join that put a blank cart in for the world (as before UC8): still no save.
        assert_eq!(s.prepare_join("WORLD1-0A1B2C3D", &blank()), None);
        assert!(!s.has_save_for("WORLD1-0A1B2C3D", &blank()));
        // The cart holds the world's game (never written to its file yet): that's a save.
        assert!(s.has_save_for("WORLD1-0A1B2C3D", &vec![7u8; SAVE_SIZE]));
        assert!(!s.has_save_for("OTHER1-00000001", &vec![7u8; SAVE_SIZE]));
        s.capture(&vec![7u8; SAVE_SIZE]);
        assert!(s.has_save_for("WORLD1-0A1B2C3D", &blank()));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn an_unknown_cart_is_kept_as_unassigned() {
        let dir = temp("unassigned");
        let mut s = SaveSlots::new(&dir);
        let img = vec![9u8; SAVE_SIZE];
        assert_eq!(s.prepare_join("CCCCCC-00000003", &img), Some(blank()));
        let kept: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .flatten()
            .filter(|e| e.file_name().to_string_lossy().starts_with("unassigned-"))
            .collect();
        assert_eq!(kept.len(), 1);
        assert_eq!(std::fs::read(kept[0].path()).unwrap(), img);
        std::fs::remove_dir_all(&dir).ok();
    }
}
