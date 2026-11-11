//! What the battle's touch-screen menu shows, read from the game (UC9 §3.2), for the one-screen
//! battle HUD: which menu is up and waiting for a choice, the key cursor, the active Pokémon's
//! moves and PP, and the battle's current message.
//!
//! The path is the vanilla game's (pret pokeheartgold), checked on the running co-op ROM:
//! the main overlay manager runs the field; the field's process manager has the battle as its
//! child application (overlay 12); that manager's data is the `BattleSystem`, whose
//! `battleInput` (+0x19C) is the touch screen's menu and whose `msgBuffer` (+0x18) is the last
//! message printed. Every pointer is checked to stay in main RAM, so a wrong guess reads
//! nothing rather than garbage.

const MAIN_RAM: u32 = 0x0200_0000;
/// pret `sMainWork`: [1] the running main overlay manager (the field's, in a game).
const MAIN_WORK: u32 = 0x0211_1868;
/// `OverlayManager`: its template's overlay id, its data.
const MANAGER_OVERLAY: u32 = 0x0C;
const MANAGER_DATA: u32 = 0x1C;
/// The battle application's overlay.
const OVERLAY_BATTLE: u32 = 12;
/// `FieldSystem.processManager`; `FieldProcessManager.child`.
const FIELD_PROCESS: u32 = 0x00;
const PROCESS_CHILD: u32 = 0x04;
/// `BattleSystem.msgBuffer`, `.battleInput`.
const SYSTEM_MESSAGE: u32 = 0x18;
const SYSTEM_INPUT: u32 = 0x19C;
/// `BattleInput`: the menu's data (a union; the fight menu's moves), its text objects (set
/// while a menu is drawn), the menu's id, the key cursor, and the "chosen with a key" flag.
/// Offsets measured on the running ROM (pret's own count drifts in this struct).
const INPUT_MENU: u32 = 0x1C;
const INPUT_TEXT_OBJECTS: u32 = 0x4D0;
const TEXT_OBJECT_COUNT: u32 = 13;
const TEXT_OBJECT_SIZE: u32 = 0x14;
const INPUT_MENU_ID: u32 = 0x68B;
const INPUT_CURSOR: u32 = 0x6DC;
const INPUT_CHOSEN: u32 = 0x6E0;
/// pret `sOverlayRegions` (poke_overlay.c): three regions of eight loaded overlays, each
/// {id, active}. Found on the running ROM; the overlays loaded in a battle are listed there.
const LOADED_OVERLAYS: u32 = 0x021D_0DB0;
const LOADED_OVERLAY_COUNT: u32 = 24;
/// The battle's bag and party screens (overlay 8 takes overlay 10's place, the battle menu's).
const OVERLAY_BATTLE_BAG_PARTY: u32 = 8;
/// `String`: size, magic, then the characters.
const STRING_MAGIC: u32 = 0xB6F8_D2EC;
const MESSAGE_MAX: usize = 256;

use goldlink_rom::nitro::{Charmap, MoveInfo};

/// The game's words for the HUD, from the player's ROM: its characters, the moves' names and
/// types.
pub struct GameText {
    charmap: Charmap,
    moves: MoveInfo,
}

impl GameText {
    pub fn read(rom: &[u8]) -> Self {
        Self {
            charmap: Charmap::new(),
            moves: MoveInfo::read(rom),
        }
    }

    /// The menu with its words: the message as the player reads it, each move's name and type.
    pub fn hud(&self, ui: &BattleUi) -> BattleHud {
        let text = self.charmap.decode(&ui.message);
        BattleHud {
            menu: ui.menu,
            waiting: ui.waiting,
            cursor: ui.cursor,
            bag_or_party: ui.bag_or_party,
            message: text.split_whitespace().collect::<Vec<_>>().join(" "),
            moves: ui.moves.map(|m| {
                m.map(|m| {
                    let ty = self.moves.type_of(m.id).unwrap_or(0);
                    HudMove {
                        name: self.moves.name(m.id).to_owned(),
                        type_id: ty,
                        type_name: self
                            .moves
                            .type_names
                            .get(ty as usize)
                            .cloned()
                            .unwrap_or_default(),
                        pp: m.pp,
                        max: m.max,
                    }
                })
            }),
        }
    }
}

/// A move as the HUD shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HudMove {
    pub name: String,
    /// The game's type number (hg-engine's order: Normal, Fighting, … Fairy at 9).
    pub type_id: u8,
    pub type_name: String,
    pub pp: u8,
    pub max: u8,
}

/// What the one-screen battle HUD draws.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BattleHud {
    pub menu: i8,
    pub waiting: bool,
    pub cursor: Option<(i8, i8)>,
    /// The battle's bag or party is open on the touch screen.
    pub bag_or_party: bool,
    /// The battle's message on one line ("What will Steelix do?").
    pub message: String,
    pub moves: [Option<HudMove>; 4],
}

/// pret `BattleMenuID`.
pub mod menu {
    pub const MAIN_INITIAL: i8 = 1;
    pub const MAIN: i8 = 2;
    pub const FIGHT: i8 = 11;
    pub const TARGET: i8 = 12;
    pub const YES_NO: i8 = 13;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BattleMove {
    pub id: u16,
    pub pp: u8,
    pub max: u8,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BattleUi {
    /// The touch screen's menu (pret `BattleMenuID`); -1 none.
    pub menu: i8,
    /// The menu is drawn and waiting for a choice.
    pub waiting: bool,
    /// The key cursor (row, column), when the player has used the keys in this menu.
    pub cursor: Option<(i8, i8)>,
    /// The fight menu's moves (valid in it).
    pub moves: [Option<BattleMove>; 4],
    /// The battle's last message, in the game's character codes.
    pub message: Vec<u16>,
    /// The battle's bag or party is open on the touch screen.
    pub bag_or_party: bool,
}

struct Ram<'a>(&'a [u8]);

impl Ram<'_> {
    fn at(&self, addr: u32, len: u32) -> Option<&[u8]> {
        let o = addr.checked_sub(MAIN_RAM)? as usize;
        self.0.get(o..o + len as usize)
    }
    fn u8(&self, addr: u32) -> Option<u8> {
        self.at(addr, 1).map(|b| b[0])
    }
    fn u16(&self, addr: u32) -> Option<u16> {
        self.at(addr, 2).map(|b| u16::from_le_bytes([b[0], b[1]]))
    }
    fn u32(&self, addr: u32) -> Option<u32> {
        self.at(addr, 4)
            .map(|b| u32::from_le_bytes(b.try_into().expect("4 bytes")))
    }
    /// A pointer read at `addr`, if it points into main RAM.
    fn ptr(&self, addr: u32) -> Option<u32> {
        self.u32(addr)
            .filter(|&p| p >= MAIN_RAM && ((p - MAIN_RAM) as usize) < self.0.len())
    }
}

/// The battle's menu state, when a battle runs.
pub fn read(main_ram: &[u8]) -> Option<BattleUi> {
    let r = Ram(main_ram);
    let field = r.ptr(MAIN_WORK + 4)?;
    let field_system = r.ptr(field + MANAGER_DATA)?;
    let process = r.ptr(field_system + FIELD_PROCESS)?;
    let child = r.ptr(process + PROCESS_CHILD)?;
    if r.u32(child + MANAGER_OVERLAY)? != OVERLAY_BATTLE {
        return None;
    }
    let system = r.ptr(child + MANAGER_DATA)?;
    let bag_or_party = (0..LOADED_OVERLAY_COUNT).any(|i| {
        let e = LOADED_OVERLAYS + i * 8;
        r.u32(e) == Some(OVERLAY_BATTLE_BAG_PARTY) && r.u32(e + 4) == Some(1)
    });
    let message = r
        .ptr(system + SYSTEM_MESSAGE)
        .filter(|&s| r.u32(s + 4) == Some(STRING_MAGIC))
        .and_then(|s| {
            let len = (r.u16(s + 2)? as usize).min(MESSAGE_MAX);
            (0..len as u32).map(|i| r.u16(s + 8 + i * 2)).collect()
        })
        .unwrap_or_default();
    let Some(input) = r.ptr(system + SYSTEM_INPUT) else {
        return Some(BattleUi {
            menu: -1,
            message,
            bag_or_party,
            ..BattleUi::default()
        });
    };
    let menu = r.u8(input + INPUT_MENU_ID)? as i8;
    let drawn = (0..TEXT_OBJECT_COUNT)
        .any(|i| r.u32(input + INPUT_TEXT_OBJECTS + i * TEXT_OBJECT_SIZE) != Some(0));
    let waiting = menu >= 0 && drawn && r.u8(input + INPUT_CHOSEN) == Some(0);
    let cursor = (r.u8(input + INPUT_CURSOR)? == 1).then(|| {
        (
            r.u8(input + INPUT_CURSOR + 1).unwrap_or(0) as i8,
            r.u8(input + INPUT_CURSOR + 2).unwrap_or(0) as i8,
        )
    });
    let mut moves = [None; 4];
    if menu == menu::FIGHT {
        // BattleInputFightMenu: moveNo[4], pp[4], ppMax[4].
        for (i, m) in moves.iter_mut().enumerate() {
            let i = i as u32;
            let id = r.u16(input + INPUT_MENU + i * 2)?;
            if id != 0 {
                *m = Some(BattleMove {
                    id,
                    pp: r.u8(input + INPUT_MENU + 8 + i)?,
                    max: r.u8(input + INPUT_MENU + 12 + i)?,
                });
            }
        }
    }
    Some(BattleUi {
        menu,
        waiting: waiting && !bag_or_party,
        cursor,
        moves,
        message,
        bag_or_party,
    })
}
