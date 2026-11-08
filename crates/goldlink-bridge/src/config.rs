//! `coop_config.json`, the bridge's settings (the C# bridge's Load/SaveNetConfig): the last
//! server, the party size and rules a host picks, the avatar and, above all, `player_secret`,
//! which binds this install to its trainer in locked worlds. Same file, so a BizHawk install's
//! copies in and the player keeps their place (RFC_DELTA §7.1).

use std::path::{Path, PathBuf};

pub const FILE: &str = "coop_config.json";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NetConfig {
    /// host:port ("" = none: there's no built-in server, Milestone 6 §4.3).
    pub server: String,
    pub party_size: u8,
    pub challenge_rules: u16,
    /// The avatar's key ("default", "ethan", "lyra" or a sheet's file name).
    pub avatar: String,
    /// 0 = not made yet.
    pub player_secret: u64,
}

impl Default for NetConfig {
    fn default() -> Self {
        Self {
            server: String::new(),
            party_size: 2,
            challenge_rules: 0,
            avatar: "default".into(),
            player_secret: 0,
        }
    }
}

/// A random non-zero value from the system's generator.
pub fn random_u64() -> u64 {
    let mut b = [0u8; 8];
    getrandom::fill(&mut b).expect("the system's random generator");
    u64::from_le_bytes(b).max(1)
}

impl NetConfig {
    pub fn path(dir: &Path) -> PathBuf {
        dir.join(FILE)
    }

    /// The settings in `dir` (defaults for what's missing or unreadable). A missing secret is
    /// made and written at once.
    pub fn load_or_create(dir: &Path) -> Self {
        let mut c = Self::default();
        if let Ok(text) = std::fs::read_to_string(Self::path(dir))
            && let Ok(v) = serde_json::from_str::<serde_json::Value>(&text)
        {
            if let Some(s) = v["server"].as_str() {
                c.server = s.to_owned();
            }
            if let Some(p) = v["party_size"].as_u64() {
                c.party_size = p.clamp(2, 6) as u8;
            }
            if let Some(r) = v["challenge_rules"].as_u64() {
                c.challenge_rules = goldlink_proto::sanitize_challenge_rules(r as u16);
            }
            if let Some(a) = v["avatar"].as_str()
                && !a.is_empty()
                && a.bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
            {
                c.avatar = a.to_owned();
            }
            if let Some(s) = v["player_secret"].as_str() {
                c.player_secret = u64::from_str_radix(s, 16).unwrap_or(0);
            }
        }
        if c.player_secret == 0 {
            c.player_secret = random_u64();
            c.save(dir).ok();
        }
        c
    }

    /// The C# bridge's layout, field for field.
    pub fn save(&self, dir: &Path) -> Result<(), String> {
        let q = |s: &str| serde_json::to_string(s).unwrap_or_else(|_| "\"\"".into());
        let text = format!(
            "{{\n  \"server\": {},\n  \"party_size\": {},\n  \"challenge_rules\": {},\n  \"avatar\": {},\n  \"player_secret\": \"{:016X}\"\n}}\n",
            q(&self.server),
            self.party_size,
            self.challenge_rules,
            q(&self.avatar),
            self.player_secret
        );
        goldlink_runtime::write_atomic(&Self::path(dir), text.as_bytes())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_csharp_file_and_keeps_the_secret() {
        let dir = std::env::temp_dir().join(format!("goldlink-netcfg-{}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join(FILE),
            "{\n  \"server\": \"127.0.0.1:7777\",\n  \"party_size\": 3,\n  \"challenge_rules\": 17,\n  \"avatar\": \"lyra\",\n  \"player_secret\": \"0123456789ABCDEF\"\n}\n",
        )
        .unwrap();
        let c = NetConfig::load_or_create(&dir);
        assert_eq!(c.server, "127.0.0.1:7777");
        assert_eq!((c.party_size, c.challenge_rules), (3, 17));
        assert_eq!(c.avatar, "lyra");
        assert_eq!(c.player_secret, 0x0123_4567_89AB_CDEF);
        c.save(&dir).unwrap();
        assert_eq!(NetConfig::load_or_create(&dir), c);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_new_install_gets_a_secret() {
        let dir = std::env::temp_dir().join(format!("goldlink-netcfg-new-{}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();
        let c = NetConfig::load_or_create(&dir);
        assert_ne!(c.player_secret, 0);
        assert_eq!(
            NetConfig::load_or_create(&dir).player_secret,
            c.player_secret
        );
        std::fs::remove_dir_all(&dir).ok();
    }
}
