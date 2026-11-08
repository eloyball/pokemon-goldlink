//! Emergency backups (the bridge's Backups.cs, docs/GAME_REBALANCE_UPDATE.md §23): a savestate
//! of the whole emulator every few minutes while the player walks the overworld in a world, kept
//! per world in `coop_backups/CODE-UID/`, the newest [`KEEP`] only. They are for crashes and
//! game-breaking bugs the in-game save can't fix; restoring one rolls the whole game back.
//!
//! melonDS savestates, so `.mlds` files (BizHawk's `.State` files don't load here).

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use goldlink_abi::{CoopCommBlock, CoopPlayerState};

use crate::files::write_atomic;

pub const INTERVAL: Duration = Duration::from_secs(5 * 60);
/// One hour at the interval.
pub const KEEP: usize = 12;
const EXTENSION: &str = "mlds";

pub struct Backups {
    root: PathBuf,
}

/// One backup file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Backup {
    pub path: PathBuf,
    pub taken: SystemTime,
}

impl Backups {
    /// `root` is the `coop_backups` folder.
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    pub fn dir(&self, key: &str) -> PathBuf {
        self.root.join(key)
    }

    /// Writes a backup for the world and prunes the old ones.
    pub fn write(&self, key: &str, state: &[u8]) -> Result<PathBuf, String> {
        let stamp = chrono::Local::now().format("%Y%m%d-%H%M%S");
        let path = self.dir(key).join(format!("{stamp}.{EXTENSION}"));
        write_atomic(&path, state)?;
        self.prune(key);
        Ok(path)
    }

    /// The world's backups, newest first.
    pub fn list(&self, key: &str) -> Vec<Backup> {
        let Ok(entries) = std::fs::read_dir(self.dir(key)) else {
            return Vec::new();
        };
        let mut out: Vec<Backup> = entries
            .flatten()
            .filter(|e| e.path().extension().is_some_and(|x| x == EXTENSION))
            .filter_map(|e| {
                let meta = e.metadata().ok()?;
                (meta.len() > 0).then(|| Backup {
                    path: e.path(),
                    taken: meta.modified().unwrap_or(SystemTime::UNIX_EPOCH),
                })
            })
            .collect();
        out.sort_by(|a, b| b.taken.cmp(&a.taken).then(b.path.cmp(&a.path)));
        out
    }

    /// Deletes all but the newest [`KEEP`].
    pub fn prune(&self, key: &str) {
        for old in self.list(key).into_iter().skip(KEEP) {
            std::fs::remove_file(&old.path).ok();
        }
    }

    pub fn read(path: &Path) -> Result<Vec<u8>, String> {
        std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))
    }
}

/// Is now a good moment for a backup? The bridge's rule: in a world, in the overworld with
/// nothing running (no field task or message, no trade or link session), past the bedroom.
/// `mailbox` is the 512-byte `gCoopCommBlock`, already validated.
pub fn safe_moment(mailbox: &[u8]) -> bool {
    use std::mem::offset_of;
    let lp = offset_of!(CoopCommBlock, local_player);
    let at = |off: usize| mailbox.get(off).copied().unwrap_or(0xFF);
    let exchange = offset_of!(CoopCommBlock, exchange_ptr);
    let exchange_ptr = mailbox
        .get(exchange..exchange + 4)
        .map_or(1, |b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]));
    at(offset_of!(CoopCommBlock, is_connected_to_room)) != 0
        && at(lp + offset_of!(CoopPlayerState, activity)) == 1
        && at(lp + offset_of!(CoopPlayerState, is_active)) == 1
        && at(lp + offset_of!(CoopPlayerState, field_busy)) == 0
        && at(lp + offset_of!(CoopPlayerState, save_status)) == 2
        && exchange_ptr == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn write_list_prune() {
        let root = std::env::temp_dir().join(format!("goldlink-backups-{}", std::process::id()));
        std::fs::remove_dir_all(&root).ok();
        let b = Backups::new(&root);
        assert!(b.list("W-00000001").is_empty());
        let dir = b.dir("W-00000001");
        std::fs::create_dir_all(&dir).unwrap();
        // Fake history: KEEP + 3 files with increasing times.
        for i in 0..KEEP + 3 {
            let p = dir.join(format!("20260101-0000{i:02}.mlds"));
            std::fs::write(&p, [i as u8; 4]).unwrap();
            let t = SystemTime::UNIX_EPOCH + Duration::from_secs(1_000_000 + i as u64);
            std::fs::File::options()
                .write(true)
                .open(&p)
                .unwrap()
                .set_modified(t)
                .unwrap();
        }
        std::fs::write(dir.join("empty.mlds"), []).unwrap();
        std::fs::write(dir.join("other.txt"), [1]).unwrap();
        assert_eq!(b.list("W-00000001").len(), KEEP + 3);
        b.prune("W-00000001");
        let left = b.list("W-00000001");
        assert_eq!(left.len(), KEEP);
        assert!(
            left[0]
                .path
                .ends_with(format!("20260101-0000{:02}.mlds", KEEP + 2))
        );
        let written = b.write("W-00000001", &[9; 8]).unwrap();
        assert_eq!(b.list("W-00000001")[0].path, written);
        assert_eq!(b.list("W-00000001").len(), KEEP);
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn safe_moment_reads_the_mailbox() {
        use std::mem::offset_of;
        let mut mb = vec![0u8; 512];
        let lp = offset_of!(CoopCommBlock, local_player);
        mb[offset_of!(CoopCommBlock, is_connected_to_room)] = 1;
        mb[lp + offset_of!(CoopPlayerState, activity)] = 1;
        mb[lp + offset_of!(CoopPlayerState, is_active)] = 1;
        mb[lp + offset_of!(CoopPlayerState, save_status)] = 2;
        assert!(safe_moment(&mb));
        mb[lp + offset_of!(CoopPlayerState, field_busy)] = 1;
        assert!(!safe_moment(&mb));
        mb[lp + offset_of!(CoopPlayerState, field_busy)] = 0;
        mb[offset_of!(CoopCommBlock, exchange_ptr)] = 4;
        assert!(!safe_moment(&mb));
    }
}
