//! The naming screen (a nickname, a box's name, the player's own name), read from the game so
//! the window can type on it: each typed character becomes a tap on that character's key on the
//! touch screen, so the game handles its own input (length limit, sounds, the finished name).
//!
//! Vanilla HeartGold (US), from pret-pokeheartgold `src/naming_screen.c`: the screen is part of
//! the ARM9 binary (no overlay). While it runs, its VBlank callback is the one in `gSystem`, and
//! the static `sAppData` points at its `NamingScreenAppData` (set once its graphics are up, never
//! cleared, so the callback is what says it's open).

/// `gSystem` (hg-engine rom.ld); its first word is the VBlank callback.
const G_SYSTEM: usize = 0x021D_110C;
/// `NamingScreen_VBlankCB` (Thumb), measured on the running game (`dev/run_layout_tour.sh`
/// route `naming`).
const NAMING_VBLANK_CB: u32 = 0x0208_3141;
/// `sAppData`, measured the same way.
const APP_DATA_PTR: usize = 0x021D_43B0;

// NamingScreenAppData (size 0x5D4).
const TYPE: usize = 0x00;
const MAX_LEN: usize = 0x0C;
const IGNORE_INPUT: usize = 0x34; // kbCursor.ignoreInput
const KEYBOARD: usize = 0x3A; // u16 keyboard[6][13]: row 0 the page and control buttons
const TEXT_CURSOR_POS: usize = 0x158;
const PAGE_SWITCH_STATE: usize = 0x45C;
const PAGE_NUM: usize = 0x460;
const APP_DATA_SIZE: usize = 0x5D4;

/// NS_PAGESWITCH_STATE_IDLE: the keys take input.
const PAGE_IDLE: u32 = 4;
/// The first key of each page's first row (CHAR_A, CHAR_a, CHAR_COMMA): which page is shown.
const PAGE_FIRST_KEY: [u16; 3] = [299, 325, 429];

/// The naming screen, as the game shows it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Naming {
    /// 0 capitals, 1 small letters, 2 symbols (3 and 4: the Japanese and number pads, unused).
    pub page: u8,
    /// The keys take a tap now: no page sliding in, no full-name pause, the main loop running.
    pub ready: bool,
    /// Characters entered, and the most there can be.
    pub len: u16,
    pub max_len: u16,
}

fn word(ram: &[u8], addr: usize) -> Option<u32> {
    let o = addr.checked_sub(0x0200_0000)?;
    Some(u32::from_le_bytes(ram.get(o..o + 4)?.try_into().ok()?))
}

fn half(ram: &[u8], addr: usize) -> Option<u16> {
    let o = addr.checked_sub(0x0200_0000)?;
    Some(u16::from_le_bytes(ram.get(o..o + 2)?.try_into().ok()?))
}

/// The naming screen, while it's open.
pub fn read(ram: &[u8]) -> Option<Naming> {
    if NAMING_VBLANK_CB == 0 || word(ram, G_SYSTEM)? != NAMING_VBLANK_CB {
        return None;
    }
    let data = word(ram, APP_DATA_PTR)? as usize;
    if !(0x0200_0000..0x0240_0000 - APP_DATA_SIZE).contains(&data) {
        return None;
    }
    let page = word(ram, data + PAGE_NUM)?;
    let max_len = word(ram, data + MAX_LEN)?;
    if page > 4 || max_len == 0 || max_len > 0x20 || word(ram, data + TYPE)? > 8 {
        return None;
    }
    // The page's own keys are in place (they're loaded as the page is chosen).
    let first = half(ram, data + KEYBOARD + 26)?;
    let settled = PAGE_FIRST_KEY
        .get(page as usize)
        .is_none_or(|&k| k == first);
    Some(Naming {
        page: page as u8,
        ready: settled
            && word(ram, data + PAGE_SWITCH_STATE)? == PAGE_IDLE
            && word(ram, data + IGNORE_INPUT)? == 0,
        len: half(ram, data + TEXT_CURSOR_POS)?,
        max_len: max_len as u16,
    })
}

/// The three pages' keys (rows 1 to 5, 13 columns), as typed characters; ' ' is a blank key.
/// pret `sKeyboardRow_{Upper,Lower,Symbols}_*`. The symbols' last row (weather and faces)
/// has no typed equivalent but the arrows.
const PAGES: [[&str; 5]; 3] = [
    [
        "ABCDEFGHIJ ,.",
        "KLMNOPQRST ’-",
        "UVWXYZ     ♂♀",
        "             ",
        "0123456789   ",
    ],
    [
        "abcdefghij ,.",
        "klmnopqrst ’-",
        "uvwxyz     ♂♀",
        "             ",
        "0123456789   ",
    ],
    [
        ",.:;!?   ♂♀  ",
        "“”‘’()       ",
        "…·~@#%+-*/=  ",
        "◎○□△◇♠♥♦♣★♪  ",
        "         ↑↓  ",
    ],
];

/// What the keyboard can type, as the game writes it ("'" is the game's ’, '"' its ”).
fn normalise(c: char) -> char {
    match c {
        '\'' | '`' | '´' => '’',
        '"' => '”',
        _ => c,
    }
}

/// Where a character is on a page: row (1-5), column. A space is a blank key.
fn on_page(c: char, page: usize) -> Option<(u8, u8)> {
    let c = normalise(c);
    PAGES.get(page)?.iter().enumerate().find_map(|(r, keys)| {
        keys.chars()
            .position(|k| k == c)
            .map(|col| (r as u8 + 1, col as u8))
    })
}

/// A key's middle on the touch screen (`sTouchHitboxDef`: 16 x 19 keys from (28, 88)).
fn key_at(row: u8, col: u8) -> (u16, u16) {
    (
        0x1C + 16 * u16::from(col) + 8,
        0x58 + 0x13 * (u16::from(row) - 1) + 9,
    )
}

/// A page's button in the top row (capitals, small letters, symbols).
fn page_button(page: u8) -> (u16, u16) {
    (0x19 + 0x20 * u16::from(page) + 15, 0x3C + 11)
}

/// BACK (deletes the last character) and OK (done).
pub const BACK: (u16, u16) = (0x9D + 16, 0x3C + 11);
pub const OK: (u16, u16) = (0xC5 + 16, 0x3C + 11);

/// The tap that types `c` from `page`: its key (true), or first the button of the page that has
/// it (false: then `c` again once that page is up). None: the game has no such character.
pub fn tap_for(c: char, page: u8) -> Option<((u16, u16), bool)> {
    if let Some((row, col)) = on_page(c, page as usize) {
        return Some((key_at(row, col), true));
    }
    let p = (0..PAGES.len()).find(|&p| on_page(c, p).is_some())?;
    Some((page_button(p as u8), false))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pages_are_thirteen_keys_wide() {
        for rows in PAGES {
            for row in rows {
                assert_eq!(row.chars().count(), 13, "{row}");
            }
        }
    }

    #[test]
    fn letters_and_page_changes() {
        // 'A' on the capitals: its key, the first in the first row.
        assert_eq!(tap_for('A', 0), Some(((36, 97), true)));
        // 'a' from the capitals: the small letters' button first.
        assert_eq!(tap_for('a', 0), Some((page_button(1), false)));
        assert_eq!(tap_for('a', 1), Some(((36, 97), true)));
        // Digits are on both letter pages; the symbols' page goes back to the capitals.
        assert_eq!(tap_for('7', 1), Some((key_at(5, 7), true)));
        assert_eq!(tap_for('7', 2), Some((page_button(0), false)));
        // An apostrophe is the game's ’, on the letter pages.
        assert_eq!(tap_for('\'', 1), Some((key_at(2, 11), true)));
        assert_eq!(tap_for('?', 0), Some((page_button(2), false)));
        assert_eq!(tap_for('?', 2), Some((key_at(1, 5), true)));
        assert_eq!(tap_for(' ', 1), Some((key_at(1, 10), true)));
        assert_eq!(tap_for(' ', 2), Some((key_at(1, 6), true)));
        assert_eq!(tap_for('é', 0), None);
    }

    #[test]
    fn closed_without_its_addresses() {
        assert_eq!(read(&vec![0u8; 0x40_0000]), None);
    }
}
