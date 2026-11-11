//! Import a save… (UC8 §2.3): another GoldLink's saves into this one. The folder may be a
//! BizHawk install (the dev setup or a v0.3 release's), a v0.3 launcher's data folder (its
//! `bizhawk/`), another unified client's data folder, or a `coop_saves` folder itself.
//!
//! Brings over the world saves (`CODE-UID.sav` and their `.bak`, never over a file this install
//! has), merges the world list, and the player secret (the relay knows a player by it in a world
//! whose roster is locked): taken at once when this install has no world saves of its own,
//! otherwise left for the player to decide ([`adopt_secret`]).

use std::path::{Path, PathBuf};

use goldlink_bridge::config::{FILE as CONFIG_FILE, NetConfig};
use goldlink_bridge::worlds::{FILE as WORLDS_FILE, WorldList, is_world_code};

const SAVES: &str = "coop_saves";

#[derive(Debug, PartialEq, Eq)]
pub enum Secret {
    /// The folder has none (or the same one).
    Same,
    /// This install had no world saves: it's now this player's secret.
    Taken,
    /// This install has worlds of its own under another secret: the player decides.
    Differs(u64),
}

#[derive(Debug, PartialEq, Eq)]
pub struct Report {
    /// World codes whose saves came over.
    pub worlds: Vec<String>,
    /// World-list entries added.
    pub listed: usize,
    pub secret: Secret,
}

impl Report {
    pub fn summary(&self) -> String {
        let saves = match self.worlds.len() {
            0 => "No new world saves (this computer has them already)".to_owned(),
            1 => format!("Brought over the save for world {}", self.worlds[0]),
            _ => format!(
                "Brought over the saves for worlds {}",
                self.worlds.join(", ")
            ),
        };
        match self.secret {
            Secret::Taken => format!("{saves}, and your player identity."),
            _ => format!("{saves}."),
        }
    }
}

/// `CODE-UID.sav` (a world's save) → its code.
fn world_of(name: &str) -> Option<&str> {
    let stem = name.strip_suffix(".sav")?;
    let (code, uid) = stem.split_once('-')?;
    (is_world_code(code) && uid.len() == 8 && uid.bytes().all(|b| b.is_ascii_hexdigit()))
        .then_some(code)
}

/// The folder that holds `coop_saves/` (and the config beside it), from what the player picked.
pub fn find_root(picked: &Path) -> Option<PathBuf> {
    if picked.file_name().is_some_and(|n| n == SAVES) && picked.is_dir() {
        return picked.parent().map(Path::to_path_buf);
    }
    [
        picked.to_path_buf(),
        picked.join("bizhawk"),
        picked.join("BizHawk"),
    ]
    .into_iter()
    .find(|p| p.join(SAVES).is_dir())
}

fn world_saves(dir: &Path) -> Vec<String> {
    std::fs::read_dir(dir)
        .map(|d| {
            d.flatten()
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .filter(|n| world_of(n).is_some())
                .collect()
        })
        .unwrap_or_default()
}

fn secret_in(root: &Path) -> Option<u64> {
    let text = std::fs::read_to_string(root.join(CONFIG_FILE)).ok()?;
    let v: serde_json::Value = serde_json::from_str(&text).ok()?;
    u64::from_str_radix(v["player_secret"].as_str()?, 16)
        .ok()
        .filter(|&s| s != 0)
}

/// Imports from `picked` into the data folder `data`. Nothing of this install is overwritten.
pub fn import(data: &Path, picked: &Path) -> Result<Report, String> {
    let root = find_root(picked).ok_or(
        "There are no GoldLink saves in that folder (it has no coop_saves folder). Pick the \
         folder of your old GoldLink or BizHawk.",
    )?;
    let same = |a: &Path, b: &Path| {
        a.canonicalize()
            .ok()
            .zip(b.canonicalize().ok())
            .is_some_and(|(a, b)| a == b)
    };
    if same(&root, data) {
        return Err("That's this GoldLink's own folder.".into());
    }
    let to = data.join(SAVES);
    let had_worlds = !world_saves(&to).is_empty();
    std::fs::create_dir_all(&to).map_err(|e| format!("can't create {}: {e}", to.display()))?;

    let mut worlds = Vec::new();
    let from = root.join(SAVES);
    let entries =
        std::fs::read_dir(&from).map_err(|e| format!("can't read {}: {e}", from.display()))?;
    for e in entries.flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        let base = name.strip_suffix(".bak").unwrap_or(&name);
        let Some(code) = world_of(base) else { continue };
        let target = to.join(&name);
        if !e.path().is_file() || target.exists() {
            continue;
        }
        std::fs::copy(e.path(), &target).map_err(|err| format!("can't copy {name}: {err}"))?;
        if !name.ends_with(".bak") && !worlds.iter().any(|w| w == code) {
            worlds.push(code.to_owned());
        }
    }
    worlds.sort();

    let listed = WorldList::new(data, &to).merge_from(&root.join(WORLDS_FILE));

    let mine = NetConfig::load_or_create(data);
    let secret = match secret_in(&root) {
        Some(s) if s != mine.player_secret => {
            if had_worlds {
                Secret::Differs(s)
            } else {
                adopt_secret(data, s)?;
                Secret::Taken
            }
        }
        _ => Secret::Same,
    };
    Ok(Report {
        worlds,
        listed,
        secret,
    })
}

/// Makes `secret` this install's player secret (the imported worlds know the player by it).
pub fn adopt_secret(data: &Path, secret: u64) -> Result<(), String> {
    let mut c = NetConfig::load_or_create(data);
    c.player_secret = secret;
    c.save(data)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("goldlink-import-{name}-{}", std::process::id()));
        std::fs::remove_dir_all(&d).ok();
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    /// The dev BizHawk folder this was written for: its saves, list and secret come over; a
    /// second import changes nothing.
    #[test]
    fn a_bizhawk_folder_into_a_fresh_install() {
        let dir = temp("fresh");
        let (old, data) = (dir.join("BizHawk"), dir.join("data"));
        std::fs::create_dir_all(old.join(SAVES)).unwrap();
        std::fs::write(
            old.join(CONFIG_FILE),
            "{\"server\": \"h:7777\", \"player_secret\": \"0123456789ABCDEF\"}",
        )
        .unwrap();
        for f in [
            "WORLD1-0A1B2C3D.sav",
            "WORLD1-0A1B2C3D.sav.bak",
            "WORLD2-11223344.sav",
        ] {
            std::fs::write(old.join(SAVES).join(f), f).unwrap();
        }
        for f in [
            "sram_owner.txt",
            "unassigned-20260101-000000.sav",
            "TEST01-00000001.sav.before-test-mon",
        ] {
            std::fs::write(old.join(SAVES).join(f), "x").unwrap();
        }
        // This install made its own secret and claimed WORLD1 with a blank cart (as before UC8).
        std::fs::create_dir_all(data.join(SAVES)).unwrap();
        std::fs::write(
            data.join(SAVES).join("sram_owner.txt"),
            "WORLD1-0A1B2C3D\nabc\n",
        )
        .unwrap();
        let before = NetConfig::load_or_create(&data).player_secret;
        assert_ne!(before, 0x0123_4567_89AB_CDEF);

        let r = import(&data, &old).unwrap();
        assert_eq!(r.worlds, ["WORLD1", "WORLD2"]);
        assert_eq!(r.secret, Secret::Taken);
        assert!(r.summary().contains("identity"), "{}", r.summary());
        assert_eq!(
            NetConfig::load_or_create(&data).player_secret,
            0x0123_4567_89AB_CDEF
        );
        assert_eq!(
            std::fs::read_to_string(data.join(SAVES).join("WORLD1-0A1B2C3D.sav.bak")).unwrap(),
            "WORLD1-0A1B2C3D.sav.bak"
        );
        for f in [
            "unassigned-20260101-000000.sav",
            "TEST01-00000001.sav.before-test-mon",
        ] {
            assert!(!data.join(SAVES).join(f).exists(), "{f} stays behind");
        }
        assert_eq!(
            std::fs::read_to_string(data.join(SAVES).join("sram_owner.txt")).unwrap(),
            "WORLD1-0A1B2C3D\nabc\n",
            "this install's owner file isn't replaced"
        );
        assert!(
            old.join(SAVES).join("WORLD1-0A1B2C3D.sav").is_file(),
            "the old folder stays"
        );

        let again = import(&data, &old.join(SAVES)).unwrap();
        assert!(again.worlds.is_empty());
        assert_eq!(again.secret, Secret::Same);
        std::fs::remove_dir_all(&dir).ok();
    }

    /// An install with worlds of its own keeps its secret until the player says otherwise, and
    /// never loses a save it has.
    #[test]
    fn an_install_with_its_own_worlds() {
        let dir = temp("own");
        let (old, data) = (dir.join("old"), dir.join("data"));
        std::fs::create_dir_all(old.join("bizhawk").join(SAVES)).unwrap();
        std::fs::write(
            old.join("bizhawk").join(CONFIG_FILE),
            "{\"player_secret\": \"00000000000000AB\"}",
        )
        .unwrap();
        std::fs::write(
            old.join("bizhawk").join(SAVES).join("NEWONE-00000001.sav"),
            "old",
        )
        .unwrap();
        std::fs::write(
            old.join("bizhawk").join(SAVES).join("MINE01-00000002.sav"),
            "old",
        )
        .unwrap();
        std::fs::create_dir_all(data.join(SAVES)).unwrap();
        std::fs::write(data.join(SAVES).join("MINE01-00000002.sav"), "mine").unwrap();
        let mine = NetConfig::load_or_create(&data).player_secret;

        // A v0.3 launcher's data folder: its saves are in bizhawk/.
        let r = import(&data, &old).unwrap();
        assert_eq!(r.worlds, ["NEWONE"]);
        assert_eq!(r.secret, Secret::Differs(0xAB));
        assert_eq!(NetConfig::load_or_create(&data).player_secret, mine);
        assert_eq!(
            std::fs::read_to_string(data.join(SAVES).join("MINE01-00000002.sav")).unwrap(),
            "mine"
        );
        adopt_secret(&data, 0xAB).unwrap();
        assert_eq!(NetConfig::load_or_create(&data).player_secret, 0xAB);

        assert!(import(&data, &dir.join("nothing")).is_err());
        assert!(import(&data, &data).is_err(), "not into itself");
        std::fs::remove_dir_all(&dir).ok();
    }
}
