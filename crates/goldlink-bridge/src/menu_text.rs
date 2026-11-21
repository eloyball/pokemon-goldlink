//! The field's open choice menu in the game's own words and font, for the one-screen layout's
//! menu box (the user, 2026-11-18: "extract the text from the second screen and write it
//! normally, using the regular font text on text boxes").
//!
//! - **The words:** Overlay 131 copies the menu a script waits on into its menu mirror every field
//!   frame (hg-engine `src/field/coop_menu_mirror.c`); the field header at 0x023C4000 points to it
//!   (`CoopFieldEntryHeader.menu`). [`read_menu`] reads it from main RAM.
//! - **The font:** the text boxes' font, `a/0/1/6` member 1 (pret `FONT_MESSAGE`,
//!   `src/font_data.c`): a header, 2bpp 8x8 tiles per glyph (glyph = character code - 1), a width
//!   per glyph. [`GameFont::draw`] writes a line as the game's text printer does: colour 1 the
//!   letters, 2 their shadow.

use crate::layout::*;
use goldlink_abi::{
    COOP_MENU_GLYPHS, COOP_MENU_ITEMS, COOP_MENU_MIRROR_MAGIC, CoopFieldEntryHeader, CoopMenuMirror,
};
use std::mem::{offset_of, size_of};

const MENU_SIZE: usize = size_of::<CoopMenuMirror>();

fn u32_le(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(at..at + 4)?.try_into().ok()?))
}

/// The open menu's choices (HeartGold's character codes, each without its 0xFFFF), or none.
/// Only in the field (the header's memory belongs to other overlays elsewhere), every part
/// checked: the header's magic, the pointer inside Overlay 131, the mirror's magic and count.
pub fn read_menu(ram: &[u8], local_player: &[u8]) -> Vec<Vec<u16>> {
    if local_player.get(PS_IS_ACTIVE) != Some(&1)
        || u32_le(ram, FIELD_ENTRY_HEADER) != Some(FIELD_ENTRY_MAGIC)
    {
        return Vec::new();
    }
    let Some(ptr) = u32_le(
        ram,
        FIELD_ENTRY_HEADER + offset_of!(CoopFieldEntryHeader, menu),
    ) else {
        return Vec::new();
    };
    if ptr < OVERLAY131_START || ptr > OVERLAY131_END - MENU_SIZE as u32 || ptr & 3 != 0 {
        return Vec::new();
    }
    let at = (ptr - MAIN_RAM_BASE) as usize;
    let Some(m) = ram.get(at..at + MENU_SIZE) else {
        return Vec::new();
    };
    if u32_le(m, 0) != Some(COOP_MENU_MIRROR_MAGIC) {
        return Vec::new();
    }
    let count = (m[offset_of!(CoopMenuMirror, count)] as usize).min(COOP_MENU_ITEMS as usize);
    let text = offset_of!(CoopMenuMirror, text);
    (0..count)
        .map(|k| {
            (0..COOP_MENU_GLYPHS as usize)
                .map(|i| {
                    let o = text + (k * COOP_MENU_GLYPHS as usize + i) * 2;
                    u16::from_le_bytes([m[o], m[o + 1]])
                })
                .take_while(|&c| c != 0xFFFF)
                .collect()
        })
        .collect()
}

/// HeartGold's space (pret's charmap).
const SPACE: u16 = 0x01DE;

/// The text boxes' font from the player's ROM.
#[derive(Debug, Clone)]
pub struct GameFont {
    /// Each glyph's pixels (colour indices 0..3), `cell` x `height`.
    glyphs: Vec<Vec<u8>>,
    widths: Vec<u8>,
    cell: usize,
    pub height: usize,
}

impl GameFont {
    /// Reads `a/0/1/6` member 1; None when it isn't where HeartGold keeps it.
    pub fn read(rom: &[u8]) -> Option<Self> {
        let narc = goldlink_rom::nitro::narc(goldlink_rom::nitro::file(rom, "a/0/1/6")?)?;
        let f = *narc.get(1)?;
        let header = u32_le(f, 0)? as usize;
        let widths_at = u32_le(f, 4)? as usize;
        let count = u32_le(f, 8)? as usize;
        let height = *f.get(13)? as usize;
        let (tw, th) = (*f.get(14)? as usize, *f.get(15)? as usize);
        if !(1..=2).contains(&tw) || !(1..=2).contains(&th) || count == 0 || count > 2000 {
            return None;
        }
        let size = 16 * tw * th;
        let cell = 8 * tw;
        let mut glyphs = Vec::with_capacity(count);
        for g in 0..count {
            let data = f.get(header + g * size..header + (g + 1) * size)?;
            let mut px = vec![0u8; cell * 8 * th];
            // Tiles left to right, then top to bottom; each row a little-endian u16 whose high
            // byte holds the left four pixels, the leftmost in its top two bits (pret
            // DecompressGlyphTile and GenerateFontHalfRowLookupTable).
            for t in 0..tw * th {
                let (tx, ty) = (t % tw, t / tw);
                for r in 0..8 {
                    let (lo, hi) = (data[t * 16 + 2 * r], data[t * 16 + 2 * r + 1]);
                    for (half, byte) in [hi, lo].into_iter().enumerate() {
                        for p in 0..4 {
                            let v = (byte >> (6 - 2 * p)) & 3;
                            let x = tx * 8 + half * 4 + p;
                            px[(ty * 8 + r) * cell + x] = v;
                        }
                    }
                }
            }
            glyphs.push(px);
        }
        let widths = f.get(widths_at..widths_at + count)?.to_vec();
        Some(Self {
            glyphs,
            widths,
            cell,
            height: height.min(8 * th),
        })
    }

    /// A line of text as colour indices (1 letters, 2 their shadow, 0 clear): its width and
    /// pixels, `height` rows. Control codes (0xFFFE and its arguments) are skipped, a line break
    /// is a space.
    pub fn draw(&self, codes: &[u16]) -> (usize, Vec<u8>) {
        let mut glyphs = Vec::new();
        let mut i = 0;
        while i < codes.len() {
            match codes[i] {
                0xFFFF => break,
                // A line break: a space in the one-line box (the PC's two-line buttons, MOVE /
                // POKéMON and MOVE / ITEMS, read MOVE twice cut at the break).
                0xE000 => {
                    let g = SPACE as usize - 1;
                    if g < self.glyphs.len() {
                        glyphs.push(g);
                    }
                }
                0xFFFE => {
                    // 0xFFFE, the command, its argument count, the arguments.
                    let args = codes.get(i + 2).copied().unwrap_or(0) as usize;
                    i += 3 + args;
                    continue;
                }
                c => {
                    let g = (c as usize).saturating_sub(1);
                    let g = if g < self.glyphs.len() { g } else { 428 - 1 };
                    if g < self.glyphs.len() {
                        glyphs.push(g);
                    }
                }
            }
            i += 1;
        }
        let width: usize = glyphs.iter().map(|&g| self.widths[g] as usize).sum();
        let mut px = vec![0u8; width.max(1) * self.height];
        let mut x0 = 0;
        for g in glyphs {
            let w = (self.widths[g] as usize).min(self.cell);
            for y in 0..self.height {
                for x in 0..w {
                    let v = self.glyphs[g][y * self.cell + x];
                    if v == 1 || v == 2 {
                        px[y * width.max(1) + x0 + x] = v;
                    }
                }
            }
            x0 += self.widths[g] as usize;
        }
        (width.max(1), px)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ram_with(items: &[&[u16]]) -> (Vec<u8>, Vec<u8>) {
        let mut ram = vec![0u8; 4 << 20];
        let h = FIELD_ENTRY_HEADER;
        ram[h..h + 4].copy_from_slice(&FIELD_ENTRY_MAGIC.to_le_bytes());
        let ptr = OVERLAY131_START + 0x1000;
        let o = h + offset_of!(CoopFieldEntryHeader, menu);
        ram[o..o + 4].copy_from_slice(&ptr.to_le_bytes());
        let at = (ptr - MAIN_RAM_BASE) as usize;
        ram[at..at + 4].copy_from_slice(&COOP_MENU_MIRROR_MAGIC.to_le_bytes());
        ram[at + offset_of!(CoopMenuMirror, count)] = items.len() as u8;
        for (k, item) in items.iter().enumerate() {
            for i in 0..COOP_MENU_GLYPHS as usize {
                let c = item.get(i).copied().unwrap_or(0xFFFF);
                let o =
                    at + offset_of!(CoopMenuMirror, text) + (k * COOP_MENU_GLYPHS as usize + i) * 2;
                ram[o..o + 2].copy_from_slice(&c.to_le_bytes());
            }
        }
        let mut lp = vec![0u8; PLAYER_STATE_SIZE];
        lp[PS_IS_ACTIVE] = 1;
        (ram, lp)
    }

    #[test]
    fn reads_the_open_menu() {
        let (ram, lp) = ram_with(&[&[0x133, 0x12F, 0x13D], &[0x128, 0x129]]);
        assert_eq!(
            read_menu(&ram, &lp),
            vec![vec![0x133, 0x12F, 0x13D], vec![0x128, 0x129]]
        );
    }

    #[test]
    fn nothing_outside_the_field_or_with_a_bad_header() {
        let (mut ram, mut lp) = ram_with(&[&[1], &[2]]);
        lp[PS_IS_ACTIVE] = 0;
        assert!(read_menu(&ram, &lp).is_empty());
        lp[PS_IS_ACTIVE] = 1;
        ram[FIELD_ENTRY_HEADER] ^= 1;
        assert!(read_menu(&ram, &lp).is_empty());
    }
}
