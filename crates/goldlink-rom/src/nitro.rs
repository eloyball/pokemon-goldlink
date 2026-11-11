//! Reading the game's own files out of the ROM in memory (UC9 §3.2: the battle HUD's move names
//! and types come from the player's ROM, so they follow hg-engine's changes and the game's
//! language): the NitroFS file system, NARC archives and the text banks of `a/0/2/7`.

use std::collections::HashMap;

fn u16_at(b: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_le_bytes(b.get(at..at + 2)?.try_into().ok()?))
}

fn u32_at(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(at..at + 4)?.try_into().ok()?))
}

/// A file of the ROM's file system by its path (`a/0/2/7`), from the header's file name and
/// allocation tables.
pub fn file<'a>(rom: &'a [u8], path: &str) -> Option<&'a [u8]> {
    let fnt = u32_at(rom, 0x40)? as usize;
    let fat = u32_at(rom, 0x48)? as usize;
    let mut dir = 0usize; // the root's index in the directory table
    let mut parts = path.split('/').peekable();
    while let Some(name) = parts.next() {
        let entry = fnt + dir * 8;
        let mut at = fnt + u32_at(rom, entry)? as usize;
        let mut id = u16_at(rom, entry + 4)? as usize;
        let found = loop {
            let len_byte = *rom.get(at)?;
            if len_byte == 0 {
                return None;
            }
            let len = (len_byte & 0x7F) as usize;
            let is_dir = len_byte & 0x80 != 0;
            let entry_name = rom.get(at + 1..at + 1 + len)?;
            at += 1 + len;
            if is_dir {
                let sub = u16_at(rom, at)? as usize & 0x0FFF;
                at += 2;
                if entry_name == name.as_bytes() {
                    break (true, sub);
                }
            } else {
                if entry_name == name.as_bytes() {
                    break (false, id);
                }
                id += 1;
            }
        };
        match (found, parts.peek()) {
            ((true, sub), Some(_)) => dir = sub,
            ((false, id), None) => {
                let start = u32_at(rom, fat + id * 8)? as usize;
                let end = u32_at(rom, fat + id * 8 + 4)? as usize;
                return rom.get(start..end);
            }
            _ => return None,
        }
    }
    None
}

/// The members of a NARC archive.
pub fn narc(bytes: &[u8]) -> Option<Vec<&[u8]>> {
    if bytes.get(0..4)? != b"NARC" {
        return None;
    }
    let header = u16_at(bytes, 0x0C)? as usize;
    // BTAF: the members' offsets, relative to GMIF's data.
    let btaf = header;
    if bytes.get(btaf..btaf + 4)? != b"BTAF" {
        return None;
    }
    let count = u16_at(bytes, btaf + 8)? as usize;
    let btnf = btaf + u32_at(bytes, btaf + 4)? as usize;
    let gmif = btnf + u32_at(bytes, btnf + 4)? as usize;
    if bytes.get(gmif..gmif + 4)? != b"GMIF" {
        return None;
    }
    let data = gmif + 8;
    (0..count)
        .map(|i| {
            let start = u32_at(bytes, btaf + 12 + i * 8)? as usize;
            let end = u32_at(bytes, btaf + 16 + i * 8)? as usize;
            bytes.get(data + start..data + end)
        })
        .collect()
}

/// A text bank's strings, still encoded (the game's character codes).
pub fn text_bank(bytes: &[u8]) -> Option<Vec<Vec<u16>>> {
    let count = u16_at(bytes, 0)? as usize;
    let key = u16_at(bytes, 2)? as u32;
    (0..count)
        .map(|n| {
            let i = n as u32 + 1;
            let k = 765u32.wrapping_mul(i).wrapping_mul(key) & 0xFFFF;
            let k = k | (k << 16);
            let offset = (u32_at(bytes, 4 + n * 8)? ^ k) as usize;
            let length = (u32_at(bytes, 8 + n * 8)? ^ k) as usize;
            let mut seed = (i.wrapping_mul(596_947)) as u16;
            (0..length)
                .map(|c| {
                    let v = u16_at(bytes, offset + c * 2)? ^ seed;
                    seed = seed.wrapping_add(18_749);
                    Some(v)
                })
                .collect()
        })
        .collect()
}

/// The game's character codes → text (hg-engine's `charmap.txt`). Control codes become nothing
/// (or a line break), so the text is what the player reads.
pub struct Charmap(HashMap<u16, String>);

impl Default for Charmap {
    fn default() -> Self {
        Self::new()
    }
}

impl Charmap {
    pub fn new() -> Self {
        let mut map = HashMap::new();
        for line in include_str!("../../../hg-engine/charmap.txt").lines() {
            let line = line.trim_start();
            if line.starts_with("//") {
                continue;
            }
            let Some((code, text)) = line.split_once('=') else {
                continue;
            };
            let Ok(code) = u16::from_str_radix(code, 16) else {
                continue;
            };
            if text.starts_with('{') || text.starts_with("\\x") {
                continue;
            }
            let text = match text {
                "\\n" | "\\r" | "\\f" => "\n".to_owned(),
                t => t.to_owned(),
            };
            map.entry(code).or_insert(text);
        }
        Self(map)
    }

    /// Text up to the terminator (0xFFFF). The game's commands (0xFFFE and its arguments)
    /// and unknown codes are dropped.
    pub fn decode(&self, codes: &[u16]) -> String {
        let mut out = String::new();
        let mut i = 0;
        while i < codes.len() {
            let c = codes[i];
            i += 1;
            match c {
                0xFFFF => break,
                0xFFFE => {
                    // {command, argument count, arguments…}
                    let args = codes.get(i + 1).copied().unwrap_or(0) as usize;
                    i += 2 + args;
                }
                _ => {
                    if let Some(t) = self.0.get(&c) {
                        out.push_str(t);
                    }
                }
            }
        }
        out
    }
}

/// What the battle HUD shows of a move: its name and type, from the player's ROM.
#[derive(Debug, Clone, Default)]
pub struct MoveInfo {
    pub names: Vec<String>,
    /// The type of each move (the game's type number).
    pub types: Vec<u8>,
    /// The type names, by type number.
    pub type_names: Vec<String>,
}

/// Text bank numbers in `a/0/2/7` (HeartGold USA): move names; type names.
const MOVE_NAMES: usize = 750;
const TYPE_NAMES: usize = 735;

impl MoveInfo {
    /// Reads them from the ROM; empty lists when a file isn't where HeartGold keeps it.
    pub fn read(rom: &[u8]) -> Self {
        let charmap = Charmap::new();
        let bank = |n: usize| -> Vec<String> {
            file(rom, "a/0/2/7")
                .and_then(narc)
                .and_then(|m| m.get(n).copied())
                .and_then(text_bank)
                .map(|b| b.iter().map(|s| charmap.decode(s)).collect())
                .unwrap_or_default()
        };
        // a/0/1/1: one member per move; byte 4 is the type (pret `MoveTbl`).
        let types = file(rom, "a/0/1/1")
            .and_then(narc)
            .map(|m| m.iter().map(|d| d.get(4).copied().unwrap_or(0)).collect())
            .unwrap_or_default();
        Self {
            names: bank(MOVE_NAMES),
            types,
            type_names: bank(TYPE_NAMES),
        }
    }

    pub fn name(&self, id: u16) -> &str {
        self.names.get(id as usize).map_or("", String::as_str)
    }

    pub fn type_of(&self, id: u16) -> Option<u8> {
        self.types.get(id as usize).copied()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Needs the development ROM (GOLDLINK_TEST_ROM, or hg-engine/test.nds).
    #[test]
    #[ignore = "needs the co-op ROM"]
    fn move_names_and_types_from_the_rom() {
        let path = std::env::var_os("GOLDLINK_TEST_ROM").map_or_else(
            || {
                std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                    .join("../../hg-engine/test.nds")
            },
            std::path::PathBuf::from,
        );
        let rom = std::fs::read(path).unwrap();
        let info = MoveInfo::read(&rom);
        for id in [1u16, 33, 88, 225, 482] {
            let t = info.type_of(id).unwrap();
            eprintln!("{id}: {} ({})", info.name(id), info.type_names[t as usize]);
        }
        assert_eq!(info.name(33), "Tackle");
        assert_eq!(
            info.type_names[info.type_of(33).unwrap() as usize],
            "Normal"
        );
    }
}
