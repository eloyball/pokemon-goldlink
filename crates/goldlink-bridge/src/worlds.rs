//! The worlds this install played, most recent first (the C# bridge's Worlds.cs, Milestone 6
//! §4.2): `coop_worlds.json` beside `coop_config.json`, at most 50, seeded once from
//! `coop_saves/CODE-UID.sav`. Forgetting a world drops the entry, never its save. Same file
//! format, so a BizHawk install's list copies in.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

pub const FILE: &str = "coop_worlds.json";
const MAX_ENTRIES: usize = 50;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorldEntry {
    /// 6 letters or digits, upper case.
    pub code: String,
    /// host:port it was played on ("" = unknown: a save from before the list).
    #[serde(default)]
    pub server: String,
    /// This install created it (else joined).
    #[serde(default)]
    pub created: bool,
    /// Unix seconds (0 = unknown).
    #[serde(default)]
    pub last_played: i64,
    /// The relay's QUIC certificate fingerprint (SHA-256, hex; "" = none yet), pinned on the
    /// first connection (protocol 9, trust on first use): from then on it is required.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub pin: String,
    /// The relay's QUIC port, learned with the pin.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub quic_port: u16,
}

fn is_zero(v: &u16) -> bool {
    *v == 0
}

impl WorldEntry {
    /// What the list shows.
    pub fn label(&self) -> String {
        let how = if self.created { "created" } else { "joined" };
        let server = if self.server.is_empty() {
            "server unknown"
        } else {
            &self.server
        };
        format!("{}  ·  {how}  ·  {server}", self.code)
    }
}

#[derive(Serialize, Deserialize, Default)]
struct File {
    #[serde(default)]
    worlds: Vec<WorldEntry>,
}

pub fn is_world_code(code: &str) -> bool {
    code.len() == 6
        && code
            .bytes()
            .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit())
}

pub struct WorldList {
    path: PathBuf,
    entries: Vec<WorldEntry>,
}

impl WorldList {
    pub fn new(config_dir: &Path, saves_dir: &Path) -> Self {
        let mut list = Self {
            path: config_dir.join(FILE),
            entries: Vec::new(),
        };
        if !list.load() {
            list.seed_from_saves(saves_dir);
            list.save();
        }
        list
    }

    pub fn entries(&self) -> &[WorldEntry] {
        &self.entries
    }

    pub fn find(&self, code: &str) -> Option<&WorldEntry> {
        self.entries.iter().find(|e| e.code == code)
    }

    /// A world was joined or created on this server: to the top of the list.
    pub fn record(&mut self, code: &str, server: &str, created: bool) {
        let now = chrono::Utc::now().timestamp();
        match self.entries.iter_mut().find(|e| e.code == code) {
            Some(e) => {
                e.server = server.to_owned();
                e.created |= created;
                e.last_played = now;
            }
            None => self.entries.push(WorldEntry {
                code: code.to_owned(),
                server: server.to_owned(),
                created,
                last_played: now,
                pin: String::new(),
                quic_port: 0,
            }),
        }
        self.sort();
        self.entries.truncate(MAX_ENTRIES);
        self.save();
    }

    /// Pins the relay's QUIC certificate (hex) and port for a world on `server` (a world that
    /// moved to another server starts over: no pin carries across servers).
    pub fn set_pin(&mut self, code: &str, server: &str, pin: &str, quic_port: u16) {
        if let Some(e) = self.entries.iter_mut().find(|e| e.code == code) {
            if e.server != server {
                e.server = server.to_owned();
            }
            e.pin = pin.to_owned();
            e.quic_port = quic_port;
            self.save();
        }
    }

    /// The pin and QUIC port recorded for a world on `server`, if any.
    pub fn pin_for(&self, code: &str, server: &str) -> Option<(String, u16)> {
        self.find(code)
            .filter(|e| e.server == server && !e.pin.is_empty() && e.quic_port != 0)
            .map(|e| (e.pin.clone(), e.quic_port))
    }

    pub fn forget(&mut self, code: &str) {
        let before = self.entries.len();
        self.entries.retain(|e| e.code != code);
        if self.entries.len() != before {
            self.save();
        }
    }

    /// Adds another install's worlds (UC8 §2.3: Import a save…): the ones this list doesn't
    /// have, with their server and pin. Returns how many were added.
    pub fn merge_from(&mut self, file: &Path) -> usize {
        let Ok(text) = std::fs::read_to_string(file) else {
            return 0;
        };
        let other: File = serde_json::from_str(&text).unwrap_or_default();
        let mut added = 0;
        for e in other.worlds {
            if is_world_code(&e.code) && self.find(&e.code).is_none() {
                self.entries.push(e);
                added += 1;
            }
        }
        if added > 0 {
            self.sort();
            self.entries.truncate(MAX_ENTRIES);
            self.save();
        }
        added
    }

    fn sort(&mut self) {
        self.entries
            .sort_by_key(|e| std::cmp::Reverse(e.last_played));
    }

    fn seed_from_saves(&mut self, saves_dir: &Path) {
        let Ok(dir) = std::fs::read_dir(saves_dir) else {
            return;
        };
        for e in dir.flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            // CODE-UID.sav: 6 + 1 + 8 + 4.
            let Some(stem) = name.strip_suffix(".sav") else {
                continue;
            };
            let Some((code, uid)) = stem.split_once('-') else {
                continue;
            };
            let uid_ok = uid.len() == 8
                && uid
                    .bytes()
                    .all(|b| b.is_ascii_digit() || (b'A'..=b'F').contains(&b));
            if !is_world_code(code) || !uid_ok || self.find(code).is_some() {
                continue;
            }
            let modified = e
                .metadata()
                .and_then(|m| m.modified())
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map_or(0, |d| d.as_secs() as i64);
            self.entries.push(WorldEntry {
                code: code.to_owned(),
                server: String::new(),
                created: false,
                last_played: modified,
                pin: String::new(),
                quic_port: 0,
            });
        }
        self.sort();
    }

    fn load(&mut self) -> bool {
        let Ok(text) = std::fs::read_to_string(&self.path) else {
            return false;
        };
        let file: File = serde_json::from_str(&text).unwrap_or_default();
        for e in file.worlds {
            if is_world_code(&e.code) && self.find(&e.code).is_none() {
                self.entries.push(e);
            }
        }
        self.sort();
        true
    }

    /// The C# bridge's layout: one entry per line.
    fn save(&self) {
        let mut s = String::from("{\n  \"worlds\": [\n");
        for (i, e) in self.entries.iter().enumerate() {
            let server = serde_json::to_string(&e.server).unwrap_or_else(|_| "\"\"".into());
            // The pin must survive a restart: without it every start trusts the relay's
            // certificate anew and a changed identity is never noticed (UC8).
            let pin = if e.pin.is_empty() || e.quic_port == 0 {
                String::new()
            } else {
                let p = serde_json::to_string(&e.pin).unwrap_or_else(|_| "\"\"".into());
                format!(", \"pin\": {p}, \"quic_port\": {}", e.quic_port)
            };
            s.push_str(&format!(
                "    {{ \"code\": \"{}\", \"server\": {server}, \"created\": {}, \"last_played\": {}{pin} }}",
                e.code, e.created, e.last_played
            ));
            s.push_str(if i + 1 < self.entries.len() {
                ",\n"
            } else {
                "\n"
            });
        }
        s.push_str("  ]\n}\n");
        goldlink_runtime::write_atomic(&self.path, s.as_bytes()).ok();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seed_record_forget_and_reload() {
        let dir = std::env::temp_dir().join(format!("goldlink-worlds-{}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();
        let saves = dir.join("coop_saves");
        std::fs::create_dir_all(&saves).unwrap();
        std::fs::write(saves.join("TEST02-C7BE7B93.sav"), [0]).unwrap();
        let old = std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_700_000_000);
        std::fs::File::options()
            .write(true)
            .open(saves.join("TEST02-C7BE7B93.sav"))
            .unwrap()
            .set_modified(old)
            .unwrap();
        std::fs::write(saves.join("unassigned-20261025-233004.sav"), [0]).unwrap();
        std::fs::write(saves.join("bad-code.sav"), [0]).unwrap();
        let mut w = WorldList::new(&dir, &saves);
        assert_eq!(w.entries().len(), 1);
        assert_eq!(w.entries()[0].code, "TEST02");
        w.record("NEW001", "relay.example:7777", true);
        assert_eq!(w.entries()[0].code, "NEW001");
        assert_eq!(
            w.entries()[0].label(),
            "NEW001  ·  created  ·  relay.example:7777"
        );
        w.forget("TEST02");
        let again = WorldList::new(&dir, &saves);
        assert_eq!(again.entries(), w.entries());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn the_pin_survives_a_restart_and_worlds_merge() {
        let dir = std::env::temp_dir().join(format!("goldlink-worlds-pin-{}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();
        std::fs::create_dir_all(&dir).unwrap();
        let mut w = WorldList::new(&dir, &dir.join("none"));
        w.record("PINNED", "relay.example:7777", true);
        w.set_pin("PINNED", "relay.example:7777", &"ab".repeat(32), 7777);
        let again = WorldList::new(&dir, &dir.join("none"));
        assert_eq!(
            again.pin_for("PINNED", "relay.example:7777"),
            Some(("ab".repeat(32), 7777)),
            "a restart keeps the pin"
        );

        // Another install's list: new worlds come in with their pin; known ones stay ours.
        let other = dir.join("other");
        std::fs::create_dir_all(&other).unwrap();
        std::fs::write(
            other.join(FILE),
            format!(
                "{{\"worlds\": [{{\"code\": \"WORLD1\", \"server\": \"h:7777\", \"created\": false, \"last_played\": 5, \"pin\": \"{}\", \"quic_port\": 7777}}, {{\"code\": \"PINNED\", \"server\": \"x:1\", \"created\": false, \"last_played\": 9}}]}}",
                "cd".repeat(32)
            ),
        )
        .unwrap();
        let mut w = WorldList::new(&dir, &dir.join("none"));
        assert_eq!(w.merge_from(&other.join(FILE)), 1);
        assert_eq!(w.find("PINNED").unwrap().server, "relay.example:7777");
        assert_eq!(
            WorldList::new(&dir, &dir.join("none")).pin_for("WORLD1", "h:7777"),
            Some(("cd".repeat(32), 7777))
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn reads_the_csharp_file() {
        let dir = std::env::temp_dir().join(format!("goldlink-worlds-cs-{}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join(FILE),
            "{\n  \"worlds\": [\n    { \"code\": \"HELLOK\", \"server\": \"1.2.3.4:7777\", \"created\": true, \"last_played\": 1762480000 },\n    { \"code\": \"OTHER2\", \"server\": \"\", \"created\": false, \"last_played\": 0 }\n  ]\n}\n",
        )
        .unwrap();
        let w = WorldList::new(&dir, &dir.join("none"));
        assert_eq!(w.entries().len(), 2);
        assert!(w.find("HELLOK").unwrap().created);
        std::fs::remove_dir_all(&dir).ok();
    }
}
