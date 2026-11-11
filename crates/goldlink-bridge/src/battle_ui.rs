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
/// `BattleInput.spriteBallGauge[6]`: the player's party balls on the touch screen (measured;
/// the opponent's six follow). Each `Sprite`'s animation (+0xF0) is the ball's state.
const INPUT_BALL_GAUGE: u32 = 0x5D8;
const SPRITE_ANIMATION: u32 = 0xF0;
const INPUT_CURSOR: u32 = 0x6DC;
const INPUT_CHOSEN: u32 = 0x6E0;
/// pret `sOverlayRegions` (poke_overlay.c): three regions of eight loaded overlays, each
/// {id, active}. Found on the running ROM; the overlays loaded in a battle are listed there.
const LOADED_OVERLAYS: u32 = 0x021D_0DB0;
const LOADED_OVERLAY_COUNT: u32 = 24;
/// The battle's bag and party screens (overlay 8 takes overlay 10's place, the battle menu's).
const OVERLAY_BATTLE_BAG_PARTY: u32 = 8;
/// `BattleSystem`: the context (`BattleStruct`, +0x30), each battler's `OpponentData` (+0x34,
/// four pointers) and how many battlers there are (+0x44).
const SYSTEM_CONTEXT: u32 = 0x30;
const SYSTEM_OPPONENTS: u32 = 0x34;
const SYSTEM_MAX_BATTLERS: u32 = 0x44;
/// `OpponentData.hpBar` (pret `BattleHpBar`): the box sprite (+4), battler (+0x24), box kind
/// (+0x25), hp, max, gained, the animated value (subpixels, +0x28..), exp, max exp, gained exp,
/// its animated value (+0x38..), level (+0x48).
const OPPONENT_HP_BAR: u32 = 0x28;
/// `Sprite`: its position (fx32 x, y) and whether it's drawn (+0x34).
const SPRITE_DRAW: u32 = 0x34;
/// hg-engine `BattleStruct.battlemon` (+0x2D40, 0xC0 each): species, level (+0x34), nickname
/// (+0x36, 11 characters), status (+0x6C), gender (+0x7E, low 4 bits).
const CONTEXT_BATTLEMON: u32 = 0x2D40;
const BATTLEMON_SIZE: u32 = 0xC0;
/// The HP bar's "not animating yet" value.
const CALC_IDLE: i32 = i32::MIN;
/// `String`: size, magic, then the characters.
const STRING_MAGIC: u32 = 0xB6F8_D2EC;
const MESSAGE_MAX: usize = 256;

use goldlink_rom::nitro::{Charmap, MoveInfo};

/// The game's words for the HUD, from the player's ROM: its characters, the moves' names and
/// types.
pub struct GameText {
    charmap: Charmap,
    moves: MoveInfo,
    /// The battle's text bank (197): the touch screen's two-option labels.
    battle: Vec<Vec<u16>>,
}

/// Bank 197's two-option labels (pret battle_input.c): yes / no, forget a move / keep old moves,
/// give up on {move} / don't give up on {move}, use next Pokémon / flee, switch / keep battling.
const LABELS: [(i8, usize, usize); 5] = [
    (menu::YES_NO, 940, 941),
    (menu::KEEP_FORGET_MOVE, 1181, 1182),
    (menu::GIVE_UP_ON_MOVE, 1186, 1187),
    (menu::SWITCH_OR_FLEE, 1217, 1218),
    (menu::SWITCH_OR_KEEP, 1215, 1216),
];

impl GameText {
    pub fn read(rom: &[u8]) -> Self {
        Self {
            charmap: Charmap::new(),
            moves: MoveInfo::read(rom),
            battle: goldlink_rom::nitro::message_bank(rom, 197),
        }
    }

    fn label(&self, id: usize, fills: &[&str]) -> String {
        self.battle
            .get(id)
            .map(|c| self.charmap.decode_with(c, fills))
            .unwrap_or_default()
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
    }

    /// The menu with its words: the message as the player reads it, each move's name and type.
    pub fn hud(&self, ui: &BattleUi) -> BattleHud {
        let text = self.charmap.decode(&ui.message);
        let move_name = self.moves.name(ui.option_move).to_owned();
        let options = LABELS
            .iter()
            .find(|(m, _, _)| *m == ui.menu)
            .map(|&(_, a, b)| (self.label(a, &[&move_name]), self.label(b, &[&move_name])));
        let boxes = ui
            .boxes
            .iter()
            .map(|b| HudBox {
                name: self.charmap.decode(&b.name),
                raw: b.clone(),
            })
            .collect();
        // The target buttons are screen positions (player left, opponent right, player right,
        // opponent left): button i is battler type 2 + i, the HP box kind of the battler there.
        // Battler ids aren't positions in a link battle (they count from the host's side).
        let targets = std::array::from_fn(|i| {
            let battler = ui
                .boxes
                .iter()
                .find(|b| usize::from(b.kind) == 2 + i)
                .map_or(i, |b| usize::from(b.battler & 3));
            let t = ui.targets[battler];
            t.shown.then(|| HudTarget {
                name: self.charmap.decode(&ui.names[battler]),
                gender: t.gender,
                hp: t.hp,
                max_hp: t.max_hp,
            })
        });
        BattleHud {
            options,
            boxes,
            targets,
            run_is_back: ui.run_is_back,
            party: ui.party,
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
                        power: self.moves.power(m.id).unwrap_or(0),
                        accuracy: self.moves.accuracy(m.id).unwrap_or(0),
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
    /// 0: none (a status move).
    pub power: u8,
    /// 0: never misses.
    pub accuracy: u8,
}

/// An HP box with its Pokémon's name decoded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HudBox {
    pub name: String,
    pub raw: HpBox,
}

/// A battler the target menu offers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HudTarget {
    pub name: String,
    pub gender: u8,
    pub hp: i16,
    pub max_hp: u16,
}

/// What the one-screen battle HUD draws.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BattleHud {
    /// The command menu shows BACK where RUN is (a double battle's second Pokémon).
    pub run_is_back: bool,
    /// The player's party balls, while the touch screen shows them.
    pub party: Option<[Ball; 6]>,
    /// A two-option menu's labels (top, bottom), in the ROM's words.
    pub options: Option<(String, String)>,
    pub boxes: Vec<HudBox>,
    /// The target menu, by button (player left, opponent right, player right, opponent left):
    /// the battler there, when it can be picked.
    pub targets: [Option<HudTarget>; 4],
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
    /// FIGHT, BAG, RUN (or BACK) and POKéMON on pret's main menu hitboxes: the first turn, the
    /// later ones, and their variants (pret `sBattleMenuTemplates`).
    pub const COMMANDS: [i8; 6] = [1, 2, 3, 4, 7, 8];
    pub const FIGHT: i8 = 11;
    pub const TARGET: i8 = 12;
    pub const YES_NO: i8 = 13;
    pub const KEEP_FORGET_MOVE: i8 = 14;
    pub const GIVE_UP_ON_MOVE: i8 = 15;
    pub const SWITCH_OR_FLEE: i8 = 16;
    pub const SWITCH_OR_KEEP: i8 = 17;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BattleMove {
    pub id: u16,
    pub pp: u8,
    pub max: u8,
}

/// A battler's HP box as the game has it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct HpBox {
    pub battler: u8,
    /// pret's HP bar type: 0 the player's (singles), 1 the opponent's (singles); doubles' are
    /// 2.. (even: the player's side).
    pub kind: u8,
    /// The game's box sprite: its centre (screen pixels) and whether it's drawn.
    pub x: i32,
    pub y: i32,
    pub drawn: bool,
    pub hp: i32,
    pub max_hp: i32,
    /// While the bar drains or fills: the value it shows, in 1/256 HP; otherwise None.
    pub hp_anim: Option<i32>,
    pub exp: i32,
    pub max_exp: i32,
    pub exp_anim: Option<i32>,
    pub level: u8,
    /// The Pokémon (from the battle's own data): its nickname in the game's codes, gender
    /// (0 male, 1 female, 2 none) and status bits.
    pub name: Vec<u16>,
    pub gender: u8,
    pub status: u32,
}

impl HpBox {
    /// Whether the box belongs to the player's side.
    pub fn players(&self) -> bool {
        self.kind % 2 == 0
    }

    /// The HP the bar shows now (the game's own drain, rounded up as it draws it).
    pub fn shown_hp(&self) -> i32 {
        match self.hp_anim {
            Some(v) => (v + 255) >> 8,
            None => self.hp,
        }
    }

    /// The rectangle the game's box covers (screen pixels, end exclusive), measured on the
    /// running game: the player's singles box carries its HP numbers and EXP bar.
    pub fn rect(&self) -> [i32; 4] {
        let (up, down) = if self.kind == 0 { (24, 25) } else { (17, 17) };
        [self.x - 64, self.y - up, self.x + 64, self.y + down]
    }
}

/// A party ball on the battle's touch screen (pret: its sprite's animation 0..3).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Ball {
    #[default]
    Empty,
    Healthy,
    Status,
    Fainted,
}

/// The target menu's battlers (double battles): which can be chosen.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Target {
    pub shown: bool,
    pub gender: u8,
    pub hp: i16,
    pub max_hp: u16,
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
    /// The battlers' HP boxes.
    pub boxes: Vec<HpBox>,
    /// The two-option menus' move (forget a move, give up on a move).
    pub option_move: u16,
    /// The command menu shows BACK where RUN is (a double battle's second Pokémon).
    pub run_is_back: bool,
    /// The player's party balls as the touch screen shows them (None when it doesn't).
    pub party: Option<[Ball; 6]>,
    /// The target menu's battlers, by battler id (player left, opponent right, player right,
    /// opponent left), and every battler's nickname.
    pub targets: [Target; 4],
    pub names: [Vec<u16>; 4],
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
    let (boxes, names) = read_boxes(&r, system);
    let Some(input) = r.ptr(system + SYSTEM_INPUT) else {
        return Some(BattleUi {
            menu: -1,
            message,
            bag_or_party,
            boxes,
            names,
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
    let mut targets = [Target::default(); 4];
    if menu == menu::TARGET {
        // BattleInputTargetMenu: TargetPokemon[4] {selectedMon, gender:2 hide:1, status, _, hp,
        // hpMax}; `hide` set means the battler is there to pick.
        for (i, t) in targets.iter_mut().enumerate() {
            let at = input + INPUT_MENU + i as u32 * 8;
            let flags = r.u8(at + 1)?;
            *t = Target {
                shown: flags & 0x4 != 0,
                gender: flags & 0x3,
                hp: r.u16(at + 4)? as i16,
                max_hp: r.u16(at + 6)?,
            };
        }
    }
    let option_move = r.u16(input + INPUT_MENU).unwrap_or(0);
    let party = read_party(&r, input);
    // BattleInputMainMenu.cancelRunDisplay (+8).
    let run_is_back = menu::COMMANDS.contains(&menu) && r.u8(input + INPUT_MENU + 8) == Some(1);
    Some(BattleUi {
        menu,
        waiting: waiting && !bag_or_party,
        cursor,
        moves,
        message,
        bag_or_party,
        boxes,
        option_move,
        run_is_back,
        party,
        targets,
        names,
    })
}

/// Every battler's HP box, and every battler's nickname.
fn read_boxes(r: &Ram<'_>, system: u32) -> (Vec<HpBox>, [Vec<u16>; 4]) {
    let mut boxes = Vec::new();
    let mut names: [Vec<u16>; 4] = Default::default();
    let context = r.ptr(system + SYSTEM_CONTEXT);
    let battlers = r.u32(system + SYSTEM_MAX_BATTLERS).unwrap_or(0).min(4);
    let mon = |battler: u32| context.map(|c| c + CONTEXT_BATTLEMON + battler * BATTLEMON_SIZE);
    for battler in 0..4u32 {
        if let Some(m) = mon(battler) {
            names[battler as usize] = (0..11)
                .map_while(|i| r.u16(m + 0x36 + i * 2).filter(|&c| c != 0xFFFF))
                .collect();
        }
    }
    for i in 0..battlers {
        let Some(od) = r.ptr(system + SYSTEM_OPPONENTS + i * 4) else {
            continue;
        };
        let bar = od + OPPONENT_HP_BAR;
        let s32 = |off: u32| r.u32(bar + off).map(|v| v as i32);
        let Some(battler) = r.u8(bar + 0x24) else {
            continue;
        };
        let sprite = r.ptr(bar + 4).and_then(|box_obj| r.ptr(box_obj));
        let (x, y, drawn) = sprite.map_or((0, 0, false), |sp| {
            (
                s32_at(r, sp) >> 12,
                s32_at(r, sp + 4) >> 12,
                r.u8(sp + SPRITE_DRAW) == Some(1),
            )
        });
        let anim = |v: Option<i32>, gained: Option<i32>| {
            v.filter(|&c| c != CALC_IDLE && c != 0 && gained.unwrap_or(0) != 0)
        };
        let m = mon(u32::from(battler) & 3);
        boxes.push(HpBox {
            battler,
            kind: r.u8(bar + 0x25).unwrap_or(0),
            x,
            y,
            drawn,
            hp: s32(0x28).unwrap_or(0),
            max_hp: s32(0x2C).unwrap_or(0),
            hp_anim: anim(s32(0x34), s32(0x30)),
            exp: s32(0x38).unwrap_or(0),
            max_exp: s32(0x3C).unwrap_or(0),
            exp_anim: anim(s32(0x44), s32(0x40)),
            level: r.u8(bar + 0x48).unwrap_or(0),
            name: names[(battler & 3) as usize].clone(),
            gender: m.and_then(|m| r.u8(m + 0x7E)).map_or(2, |g| g & 0xF),
            status: m.and_then(|m| r.u32(m + 0x6C)).unwrap_or(0),
        });
    }
    (boxes, names)
}

/// The player's party balls, when the touch screen draws them.
fn read_party(r: &Ram<'_>, input: u32) -> Option<[Ball; 6]> {
    let mut balls = [Ball::Empty; 6];
    let mut drawn = false;
    for (i, b) in balls.iter_mut().enumerate() {
        let sprite = r
            .ptr(input + INPUT_BALL_GAUGE + i as u32 * 4)
            .and_then(|m| r.ptr(m))?;
        drawn |= r.u8(sprite + SPRITE_DRAW) == Some(1);
        *b = match r.u16(sprite + SPRITE_ANIMATION)? {
            1 => Ball::Healthy,
            2 => Ball::Status,
            3 => Ball::Fainted,
            _ => Ball::Empty,
        };
    }
    drawn.then_some(balls)
}

fn s32_at(r: &Ram<'_>, addr: u32) -> i32 {
    r.u32(addr).unwrap_or(0) as i32
}

/// The game's HP boxes with how far to move them: the opponents' `shift` to the left, the
/// player's to the right. melonDS moves the sprite pieces whose centre lies in a box (the
/// pieces are 64 x 64 and reach above the box, so their centres sit on its middle row).
pub fn box_moves(ui: &BattleUi, shift: i32) -> Vec<[i32; 5]> {
    ui.boxes
        .iter()
        .map(|b| {
            let [x0, y0, x1, y1] = b.rect();
            let dx = if b.players() { shift } else { -shift };
            [x0, y0, x1, y1, dx]
        })
        .collect()
}
