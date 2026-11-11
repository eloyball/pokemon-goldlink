//! The game, prepared in memory (unified-client RFC §3): check the player's ROM, apply the
//! release's BPS patch, check the result. Nothing is written to disk; the bytes go to the core.

pub mod bps;

use serde::Deserialize;
use sha1::{Digest, Sha1};

/// The fields of `release.json` (written by `scripts/package_client.sh`) the ROM step needs.
#[derive(Debug, Clone, Deserialize)]
pub struct Release {
    pub version: String,
    /// The HeartGold (USA) ROM the patch applies to, and the patched game.
    pub rom_sha1: String,
    pub patched_sha1: String,
    /// The patch's file name, next to `release.json`.
    pub patch: String,
    /// `owner/name` on GitHub, for the update check ("" = none).
    #[serde(default)]
    pub repository: String,
}

impl Release {
    pub fn parse(json: &str) -> Result<Self, String> {
        serde_json::from_str(json).map_err(|e| format!("release.json: {e}"))
    }
}

/// What a ROM's header says it is (the first 0x200 bytes).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NdsHeader {
    pub title: String,
    /// `IPKE` = HeartGold (USA), `IPGE` = SoulSilver (USA); the last letter is the region.
    pub game_code: [u8; 4],
    pub revision: u8,
}

/// The DS header's own checksum (CRC-16/MODBUS over bytes 0..0x15E, stored at 0x15E): a file
/// whose header doesn't match it isn't a DS ROM.
pub fn header_crc(rom: &[u8]) -> u16 {
    let mut crc: u16 = 0xFFFF;
    for &b in &rom[..0x15E] {
        crc ^= u16::from(b);
        for _ in 0..8 {
            crc = if crc & 1 != 0 {
                (crc >> 1) ^ 0xA001
            } else {
                crc >> 1
            };
        }
    }
    crc
}

impl NdsHeader {
    pub fn parse(rom: &[u8]) -> Option<Self> {
        if rom.len() < 0x200 || u16::from_le_bytes([rom[0x15E], rom[0x15F]]) != header_crc(rom) {
            return None;
        }
        let title = rom[..12]
            .iter()
            .take_while(|&&b| b != 0)
            .map(|&b| b as char)
            .collect();
        Some(Self {
            title,
            game_code: rom[0x0C..0x10].try_into().ok()?,
            revision: rom[0x1E],
        })
    }

    /// The game and region, in words, for a refusal ("SoulSilver (Europe)").
    pub fn describe(&self) -> String {
        let game = match &self.game_code[..3] {
            b"IPK" => "HeartGold",
            b"IPG" => "SoulSilver",
            _ => {
                return format!(
                    "\"{}\" ({})",
                    self.title,
                    String::from_utf8_lossy(&self.game_code)
                );
            }
        };
        let region = match self.game_code[3] {
            b'E' => "USA",
            b'P' => "Europe",
            b'J' => "Japan",
            b'K' => "Korea",
            b'D' => "Germany",
            b'F' => "France",
            b'I' => "Italy",
            b'S' => "Spain",
            other => return format!("{game} ({})", other as char),
        };
        format!("{game} ({region})")
    }
}

pub fn sha1_hex(bytes: &[u8]) -> String {
    Sha1::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// Why a ROM can't be used. Each one says what to do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RomError {
    NotAnNdsRom,
    AlreadyPatched,
    /// Another game or region.
    WrongGame(String),
    /// HeartGold (USA), but not the expected dump (another revision, or modified).
    UnknownDump {
        sha1: String,
        want: String,
    },
    Patch(String),
    PatchMismatch,
}

impl std::fmt::Display for RomError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotAnNdsRom => write!(
                f,
                "That file isn't a Nintendo DS ROM. Pick your Pokémon HeartGold (USA) ROM."
            ),
            Self::AlreadyPatched => write!(
                f,
                "That's an already patched GoldLink ROM. Pick your original Pokémon HeartGold (USA) ROM."
            ),
            Self::WrongGame(what) => write!(
                f,
                "That's Pokémon {what}; GoldLink needs Pokémon HeartGold (USA)."
            ),
            Self::UnknownDump { sha1, want } => write!(
                f,
                "That's HeartGold (USA), but not the copy GoldLink needs: an unmodified ROM has SHA-1 {want}; this one has {sha1}."
            ),
            Self::Patch(why) => write!(f, "Patching failed: {why}."),
            Self::PatchMismatch => write!(
                f,
                "Patching gave an unexpected result: the release may be damaged. Download it again."
            ),
        }
    }
}

impl std::error::Error for RomError {}

/// Checks the player's ROM against the release: an unmodified HeartGold (USA) of the expected
/// dump. The header explains a wrong pick.
pub fn check_base(base: &[u8], release: &Release) -> Result<(), RomError> {
    let header = NdsHeader::parse(base).ok_or(RomError::NotAnNdsRom)?;
    let sha1 = sha1_hex(base);
    if sha1.eq_ignore_ascii_case(&release.patched_sha1) {
        return Err(RomError::AlreadyPatched);
    }
    if &header.game_code != b"IPKE" {
        return Err(RomError::WrongGame(header.describe()));
    }
    if !sha1.eq_ignore_ascii_case(&release.rom_sha1) {
        return Err(RomError::UnknownDump {
            sha1,
            want: release.rom_sha1.to_ascii_lowercase(),
        });
    }
    Ok(())
}

/// Checks the player's ROM against the release, applies the patch and checks the result.
/// Hashes the bytes it was given (never the file twice), so a file that changes while it's read
/// can't slip through.
pub fn prepare(base: &[u8], patch: &[u8], release: &Release) -> Result<Vec<u8>, RomError> {
    check_base(base, release)?;
    let game = bps::apply(base, patch).map_err(|e| RomError::Patch(e.to_string()))?;
    if !sha1_hex(&game).eq_ignore_ascii_case(&release.patched_sha1) {
        return Err(RomError::PatchMismatch);
    }
    Ok(game)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rom(code: &[u8; 4], fill: u8) -> Vec<u8> {
        let mut r = vec![fill; 0x400];
        r[..12].copy_from_slice(b"POKEMON HG\0\0");
        r[0x0C..0x10].copy_from_slice(code);
        let crc = header_crc(&r);
        r[0x15E..0x160].copy_from_slice(&crc.to_le_bytes());
        r
    }

    fn release_for(base: &[u8], game: &[u8]) -> Release {
        Release {
            version: "0.3".into(),
            rom_sha1: sha1_hex(base),
            patched_sha1: sha1_hex(game),
            patch: "goldlink.bps".into(),
            repository: String::new(),
        }
    }

    #[test]
    fn patches_in_memory_and_checks_both_hashes() {
        let base = rom(b"IPKE", 1);
        let mut game = base.clone();
        game[0x300] = 0xAB;
        game.extend_from_slice(&[7; 64]);
        let patch = bps::create(&base, &game);
        let release = release_for(&base, &game);
        assert_eq!(prepare(&base, &patch, &release), Ok(game.clone()));
        assert_eq!(
            prepare(&game, &patch, &release),
            Err(RomError::AlreadyPatched)
        );
        let mut other = release.clone();
        other.patched_sha1 = "00".repeat(20);
        assert_eq!(prepare(&base, &patch, &other), Err(RomError::PatchMismatch));
    }

    #[test]
    fn refusals_say_what_was_picked() {
        let base = rom(b"IPKE", 1);
        let release = release_for(&base, &rom(b"IPKE", 2));
        assert_eq!(prepare(&[0; 16], &[], &release), Err(RomError::NotAnNdsRom));
        let ss = rom(b"IPGP", 1);
        assert_eq!(
            prepare(&ss, &[], &release),
            Err(RomError::WrongGame("SoulSilver (Europe)".into()))
        );
        assert!(
            RomError::WrongGame("SoulSilver (Europe)".into())
                .to_string()
                .contains("SoulSilver (Europe)")
        );
        let modified = rom(b"IPKE", 3);
        assert!(matches!(
            prepare(&modified, &[], &release),
            Err(RomError::UnknownDump { .. })
        ));
    }

    #[test]
    fn release_json_parses() {
        let r = Release::parse(r#"{"version":"0.3","wire_version":8,"rom_sha1":"ab","patched_sha1":"cd","patch":"goldlink.bps","bizhawk_version":"2.11.1"}"#).unwrap();
        assert_eq!(
            (r.version.as_str(), r.patch.as_str()),
            ("0.3", "goldlink.bps")
        );
    }
}
