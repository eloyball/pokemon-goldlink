//! `GOLDLINK_DEV_TOUR=<dir>` with `goldlink-dev --window`: walks the game through the screens the
//! window's layouts care about (UC9 §3.1) and saves the window at each, as `<dir>/<n>-<name>.png`,
//! for the user's approval of a layout change. Dev only (D2): it presses keys and sends debug
//! commands (a wild battle needs a dev ROM, `COOP_DEBUG_TOOLS 1`). Then it closes the window.
//!
//! The steps: the overworld; the start menu (X); its first entry (an application: the Pokédex or
//! the party); back out; a wild battle at its command menu; FIGHT's moves.

use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};

use eframe::egui;
use goldlink::goldlink_bridge::layout::*;
use goldlink::goldlink_bridge::{Scene, scene_of};
use goldlink_abi::CoopDebugCommand as C;
use goldlink_core::Keys;
use goldlink_runtime::{Emulator, FrameHook, Input};

#[derive(Debug, Clone, Copy)]
enum Step {
    /// Until the game shows this scene (or gives up after the frames).
    Until(Scene, u32),
    Wait(u32),
    Press(Keys),
    /// The touch screen as the DS draws it, saved as `<n>-<name>-raw.png`.
    Raw(&'static str),
    /// The lead: this species holding this item at this level, alone in the party (debug
    /// command 14 preset 9; the buffer's address from hg-engine/build/rom_gen.ld, the constants
    /// from its headers, as link_e2e_probe.lua does; PROBE_ROOT is the repository).
    Lead(&'static str, &'static str, u8),
    /// Presses A every second until the battle's touch screen waits in this menu (or gives up).
    MashUntilMenu(i8, u32),
    /// A touch on the touch screen (held a few frames).
    Touch(u16, u16),
    Debug(u8, u8),
    Shot(&'static str),
    /// Main RAM as it is, saved as `<n>-<name>.ram` (4 MiB from 0x02000000): what a probe would
    /// read, for scripts to look at the game's state after the run.
    Dump(&'static str),
    /// Writes into main RAM what GOLDLINK_DEV_TOUR_POKES lists (lines "<address> <bytes> <value>",
    /// hex; none without it), once the file is there: a change to the game's state tried before
    /// it goes into the ROM, computed by a script from the Dump just before.
    Poke,
    /// Locks running on or off, as the start menu pop-up's button does.
    Running(bool),
    /// Holds the keys for the frames; with a name, records every frame (UC11's lighting) as
    /// `<dir>/rec-<name>/<frame>.bin`: the wide top screen and the 3D scene's buffers (see
    /// `record_frame`).
    Walk(Keys, u32, &'static str),
    /// Presses A every 40 frames (as link_e2e_probe.lua's battles) for the frames.
    Mash(u32),
    /// Windows in step (GOLDLINK_DEV_TOUR_ROLE A, B..., GOLDLINK_DEV_TOUR_ROLES all of them,
    /// GOLDLINK_DEV_TOUR_MARKS a shared folder): Mark leaves `mark_<role>_<name>` there; Peer
    /// waits for the others' (or gives up after the frames).
    Mark(&'static str),
    Peer(&'static str, u32),
    /// Presses A every 30 frames while a field script runs (a message, a prompt), until the
    /// field has been free for a second (or the frames run out).
    Settle(u32),
    /// Presses A every 40 frames, only while a field script runs (a request's prompt: Yes is
    /// the cursor's first place; nothing in the field is talked to), until the game leaves the
    /// overworld (a battle) or the frames run out.
    MashTask(u32),
    /// Presses A every second until the naming screen is open (found in RAM by its keyboard's
    /// first row, as `NamingScreenAppData` holds it), then logs where its data and the pointers
    /// to it are, and the VBlank callback (the addresses goldlink_bridge::naming needs). With
    /// true, also taps the intro's touch-only buttons (NO INFO NEEDED, the controls' Touch).
    MashUntilNaming(u32, bool),
    /// Types this on the window's keyboard ('\n' Enter, '\x08' Backspace, '\t' Tab), as a player
    /// would.
    Type(&'static str),
    /// Logs the naming screen's entry (its character codes) and what naming::read sees.
    NamingLog,
    /// UC16's emulator pass: every case of GOLDLINK_DEV_TOUR_WIDE_CASES (crate::wide).
    Wide,
    /// The debug warp to this map, tile and facing (preset COOP_WARP_FROM_BUFFER: the place from
    /// gCoopDebugMonBuffer).
    WarpTo(u16, u16, u16, u16),
    /// For the frames: every 40 while a field script runs, the touch screen saved
    /// (`<n>-prompt-<frame>.png`) with what the ROM's menu mirror holds and what the window's
    /// button finder (`hud::touch_choices`) sees, then this key (the one-screen menu box).
    Capture(u32, Keys),
    /// A every 90 frames while the game is out of the overworld (an app: the starter's), until
    /// it's back (or the frames run out).
    MashApp(u32),
    Close,
}

/// GOLDLINK_DEV_TOUR_ROUTE=starter-prompt: the one-screen menu box at the starter: the save put back before
/// Elm's starter (debug command 20 / 21, as dev/bizhawk/m5_probe.lua's starter mode), the warp to
/// Elm's lab, the starter machine, then the "Give a nickname?" prompt captured (B: the naming
/// screen opens either way in GoldLink).
const PROMPT_STEPS: &[Step] = &[
    Step::Until(Scene::Overworld, 60 * 120),
    Step::Wait(120),
    Step::Debug(C::COOP_DEBUG_CHALLENGE as u8, 21),
    Step::Wait(30),
    Step::Debug(C::COOP_DEBUG_WARP as u8, 12),
    Step::Wait(240),
    Step::Until(Scene::Overworld, 900),
    Step::Wait(60),
    Step::Press(Keys::B),
    Step::Wait(30),
    Step::Press(Keys::B),
    Step::Wait(30),
    Step::Press(Keys::UP),
    Step::Wait(30),
    Step::Press(Keys::A),
    Step::Wait(40),
    Step::MashTask(60 * 30),
    Step::MashApp(60 * 60),
    Step::Capture(60 * 40, Keys::A),
    Step::Close,
];

/// GOLDLINK_DEV_TOUR_ROUTE=area-border (UC16 option C): the borders between areas with the most
/// textures copied in: the Ruins of Alph and Route 32, Route 38 and Ecruteak, from each side.
const AREA_BORDER_STEPS: &[Step] = &[
    Step::Until(Scene::Overworld, 60 * 120),
    Step::Wait(120),
    // The Ruins of Alph (area 20) at x 440, Route 32's block (area 3) in the right strip.
    Step::WarpTo(113, 440, 300, 3),
    Step::Wait(420),
    Step::Shot("ruins-route32"),
    // Route 32 at x 456, the Ruins' block in the left strip.
    Step::WarpTo(36, 456, 300, 2),
    Step::Wait(420),
    Step::Shot("route32-ruins"),
    // Route 38 (area 7) at x 343, Ecruteak's block (area 6) in the right strip.
    Step::WarpTo(42, 343, 175, 3),
    Step::Wait(420),
    Step::Shot("route38-ecruteak"),
    // Ecruteak at x 360, Route 38's block in the left strip.
    Step::WarpTo(78, 360, 175, 2),
    Step::Wait(420),
    Step::Shot("ecruteak-route38"),
    Step::Close,
];

/// GOLDLINK_DEV_TOUR_ROUTE=heap: the field heap's free bytes (debug command 21, into
/// gCoopDebugMonBuffer) where the save stands, then in Goldenrod, Ecruteak and Violet City (the
/// warp presets), each with a RAM dump (UC16 option C's budget).
const HEAP_STEPS: &[Step] = &[
    Step::Until(Scene::Overworld, 60 * 120),
    Step::Wait(240),
    Step::Debug(C::COOP_DEBUG_HEAP_FREE as u8, 0),
    Step::Wait(30),
    Step::Dump("heap-here"),
    Step::Debug(C::COOP_DEBUG_WARP as u8, 3),
    Step::Wait(400),
    Step::Debug(C::COOP_DEBUG_HEAP_FREE as u8, 0),
    Step::Wait(30),
    Step::Dump("heap-goldenrod"),
    Step::Debug(C::COOP_DEBUG_WARP as u8, 4),
    Step::Wait(400),
    Step::Debug(C::COOP_DEBUG_HEAP_FREE as u8, 0),
    Step::Wait(30),
    Step::Dump("heap-ecruteak"),
    Step::Debug(C::COOP_DEBUG_WARP as u8, 2),
    Step::Wait(400),
    Step::Debug(C::COOP_DEBUG_HEAP_FREE as u8, 0),
    Step::Wait(30),
    Step::Dump("heap-violet"),
    Step::Close,
];

/// GOLDLINK_DEV_TOUR_ROUTE=walk-right: a picture where the save stands, then four seconds walking
/// right (with GOLDLINK_DEV_TOUR_FILM=1, every frame: the lighting at a time of day).
const WALK_RIGHT_STEPS: &[Step] = &[
    Step::Until(Scene::Overworld, 60 * 120),
    Step::Wait(180),
    Step::Shot("standing"),
    Step::Walk(Keys::RIGHT, 240, ""),
    Step::Wait(30),
    Step::Shot("walked"),
    Step::Close,
];

/// GOLDLINK_DEV_TOUR_ROUTE=wide (UC16): the widened screen's cases, one after another.
const WIDE_STEPS: &[Step] = &[
    Step::Until(Scene::Overworld, 60 * 120),
    Step::Wait(120),
    Step::Wide,
    Step::Close,
];

/// GOLDLINK_DEV_TOUR_ROUTE=naming (a new world): Oak's speech up to the name keyboard; the
/// keyboard's addresses; then a name typed on the window's keyboard across the three pages,
/// a delete, and Enter.
const NAMING_STEPS: &[Step] = &[
    Step::Wait(240),
    // NO INFO NEEDED, where it is on the touch screen.
    Step::Touch(128, 152),
    Step::Wait(90),
    Step::MashUntilNaming(60 * 180, true),
    Step::Wait(90),
    Step::NamingLog,
    Step::Shot("naming-open"),
    Step::Type("Lyra-2!"),
    Step::Wait(240),
    Step::NamingLog,
    Step::Raw("naming-typed"),
    Step::Shot("naming-typed"),
    Step::Type("\x08\x08?"),
    Step::Wait(180),
    Step::NamingLog,
    Step::Shot("naming-edited"),
    Step::Type("\n"),
    Step::Wait(240),
    Step::NamingLog,
    Step::Shot("naming-done"),
    Step::Close,
];

/// GOLDLINK_DEV_TOUR_ROUTE=catch: a wild Pokémon caught with a Master Ball, and its nickname
/// typed on the window's keyboard (the naming screen opened from inside the battle).
const CATCH_STEPS: &[Step] = &[
    Step::Until(Scene::Overworld, 60 * 120),
    Step::Wait(60),
    // Three Master Balls.
    Step::Debug(C::COOP_DEBUG_CHALLENGE as u8, 7),
    Step::Wait(60),
    Step::Debug(C::COOP_DEBUG_WILD_BATTLE as u8, 2),
    Step::Until(Scene::Battle, 600),
    Step::Wait(60 * 15),
    // BAG (m5_probe.lua throw_ball): Down, Left, Left; the Poké Balls pocket; the second ball
    // (the Master Ball); USE.
    Step::Press(Keys::DOWN),
    Step::Wait(30),
    Step::Press(Keys::LEFT),
    Step::Wait(30),
    Step::Press(Keys::LEFT),
    Step::Wait(60),
    Step::Press(Keys::A),
    Step::Wait(150),
    Step::Press(Keys::RIGHT),
    Step::Wait(30),
    Step::Press(Keys::A),
    Step::Wait(150),
    Step::Press(Keys::RIGHT),
    Step::Wait(30),
    Step::Shot("catch-balls"),
    Step::Press(Keys::A),
    Step::Wait(150),
    Step::Press(Keys::A),
    Step::Wait(60),
    // The catch, the Pokédex entry, "Give a nickname?" (A: yes), to the keyboard.
    Step::MashUntilNaming(60 * 60, false),
    Step::Wait(90),
    Step::NamingLog,
    Step::Shot("catch-naming"),
    Step::Type("Ratty"),
    Step::Wait(180),
    Step::NamingLog,
    Step::Shot("catch-typed"),
    Step::Type("\n"),
    Step::Wait(60 * 6),
    Step::Shot("catch-done"),
    Step::Close,
];

/// NamingScreenAppData's keyboard, its first row with the capitals up (CHAR_A..CHAR_J,
/// CHAR_SPACE, CHAR_COMMA, CHAR_PERIOD), and that row's place in the data.
const UPPER_ROW: [u16; 13] = [
    299, 300, 301, 302, 303, 304, 305, 306, 307, 308, 478, 429, 430,
];
const UPPER_ROW_AT: usize = 0x3A + 26;

/// NamingScreenAppData's offset in main RAM, found by its keyboard row (not ROM's copy of it:
/// its max length is a name's).
fn find_naming(ram: &[u8]) -> Option<usize> {
    let row: Vec<u8> = UPPER_ROW.iter().flat_map(|c| c.to_le_bytes()).collect();
    ram.windows(row.len())
        .enumerate()
        .filter(|(i, w)| i % 2 == 0 && *w == row.as_slice())
        .filter_map(|(i, _)| i.checked_sub(UPPER_ROW_AT))
        .find(|&d| {
            let max = u32::from_le_bytes(ram[d + 0x0C..d + 0x10].try_into().unwrap());
            (1..=12).contains(&max)
        })
}

/// GOLDLINK_DEV_TOUR_ROUTE=faint: a level 100 wild Pokémon knocks out the lead, for the
/// two-option menu ("Use next Pokémon?" / "Flee").
const FAINT_STEPS: &[Step] = &[
    Step::Until(Scene::Overworld, 60 * 120),
    Step::Wait(60),
    // A healthy second Pokémon (a shiny Haunter, debug preset 1), so the game asks.
    Step::Debug(C::COOP_DEBUG_GIVE_TEST_MON as u8, 1),
    Step::Wait(300),
    Step::Debug(C::COOP_DEBUG_WILD_BATTLE as u8, 100),
    Step::Until(Scene::Battle, 600),
    Step::Wait(60 * 16),
    // FIGHT and the first move, turn after turn, until the lead faints and the game asks.
    Step::MashUntilMenu(
        goldlink::goldlink_bridge::battle_ui::menu::SWITCH_OR_FLEE,
        60 * 120,
    ),
    Step::Wait(30),
    Step::Shot("faint-question"),
    Step::Press(Keys::DOWN),
    Step::Wait(30),
    Step::Shot("faint-question-cursor"),
    Step::Close,
];

/// GOLDLINK_DEV_TOUR_ROUTE=intro (a new world): Oak's speech and the new game's choices, A every
/// 1.5 s and a shot every 3 s, until the overworld (or 4 minutes).
fn intro_steps() -> &'static [Step] {
    static STEPS: OnceLock<Vec<Step>> = OnceLock::new();
    STEPS.get_or_init(|| {
        // The topics come first: NO INFO NEEDED, tapped where it is on the touch screen.
        let mut v = vec![
            Step::Wait(240),
            Step::Shot("intro-topics"),
            Step::Touch(128, 152),
            Step::Wait(90),
            Step::Shot("intro-after-topics"),
        ];
        // Then, round after round: a tap where NO INFO NEEDED is (it does nothing elsewhere),
        // and A for the rest of Oak's speech.
        // On the name keyboard the tap and A type letters; START jumps to OK and A confirms.
        for _ in 0..70 {
            v.extend([
                Step::Touch(128, 152),
                Step::Wait(60),
                Step::Press(Keys::A),
                Step::Wait(40),
                Step::Press(Keys::START),
                Step::Wait(30),
                Step::Press(Keys::A),
                Step::Wait(60),
                Step::Shot("intro"),
            ]);
        }
        v.push(Step::Close);
        v
    })
}

/// GOLDLINK_DEV_TOUR_ROUTE=learn: a level 27 Geodude one EXP point short of 28 (debug preset 10)
/// beats a wild Pokémon and learns Stealth Rock: "forget a move?", then "give up on it?".
const LEARN_STEPS: &[Step] = &[
    Step::Until(Scene::Overworld, 60 * 120),
    Step::Wait(60),
    Step::Debug(C::COOP_DEBUG_GIVE_TEST_MON as u8, 10),
    Step::Wait(300),
    Step::Debug(C::COOP_DEBUG_WILD_BATTLE as u8, 2),
    Step::Until(Scene::Battle, 600),
    Step::MashUntilMenu(
        goldlink::goldlink_bridge::battle_ui::menu::KEEP_FORGET_MOVE,
        60 * 120,
    ),
    Step::Wait(30),
    Step::Shot("learn-forget"),
    // The lower choice (keep the old moves): the game asks whether to give up on the new one.
    Step::Press(Keys::DOWN),
    Step::Wait(20),
    Step::Press(Keys::A),
    Step::MashUntilMenu(
        goldlink::goldlink_bridge::battle_ui::menu::GIVE_UP_ON_MOVE,
        60 * 30,
    ),
    Step::Wait(30),
    Step::Shot("learn-give-up"),
    Step::Close,
];

/// GOLDLINK_DEV_TOUR_ROUTE=mega: the Key Stone, a Mega Stone holder, a wild battle's fight menu;
/// the touch screen's MEGA button off, then lit (for the HUD's reading of it).
const MEGA_STEPS: &[Step] = &[
    Step::Until(Scene::Overworld, 60 * 120),
    Step::Wait(60),
    // The Key Stone comes with the game cleared (debug command 16 bit 3), with messages: B
    // closes them (the tour's save faces Falkner, and A would talk to him; the field must be free
    // for the next debug commands).
    Step::Debug(C::COOP_DEBUG_GIVE_HMS as u8, 8),
    Step::Wait(120),
    Step::Press(Keys::B),
    Step::Wait(90),
    Step::Press(Keys::B),
    Step::Wait(90),
    Step::Press(Keys::B),
    Step::Wait(90),
    Step::Press(Keys::B),
    Step::Wait(90),
    Step::Press(Keys::B),
    Step::Wait(90),
    Step::Press(Keys::B),
    Step::Wait(240),
    Step::Lead("GENGAR", "GENGARITE", 50),
    Step::Wait(400),
    Step::Lead("GENGAR", "GENGARITE", 50),
    Step::Wait(400),
    Step::Debug(C::COOP_DEBUG_WILD_BATTLE as u8, 5),
    Step::Until(Scene::Battle, 300),
    Step::Debug(C::COOP_DEBUG_WILD_BATTLE as u8, 5),
    Step::Until(Scene::Battle, 600),
    Step::MashUntilMenu(goldlink::goldlink_bridge::battle_ui::menu::FIGHT, 60 * 60),
    Step::Wait(40),
    Step::Raw("mega-off"),
    Step::Shot("mega-off"),
    Step::Touch(212, 172),
    Step::Wait(40),
    Step::Raw("mega-on"),
    Step::Shot("mega-on"),
    // The first move with MEGA lit: Gengar mega-evolves before it attacks.
    Step::Touch(64, 52),
    Step::Wait(60 * 9),
    Step::Shot("mega-evolved"),
    Step::Close,
];

/// hg-engine's `#define NAME value` in one of its headers.
fn engine_constant(root: &std::path::Path, header: &str, name: &str) -> Option<u32> {
    let text = std::fs::read_to_string(root.join("hg-engine").join(header)).ok()?;
    text.lines().find_map(|l| {
        let mut w = l.split_whitespace();
        (w.next() == Some("#define") && w.next() == Some(name))
            .then(|| w.next()?.parse().ok())
            .flatten()
    })
}

/// A symbol's address from hg-engine's build (rom_gen.ld: `name = 0x...;`).
/// Whether a field script runs (link_e2e_probe.lua's field_task: gFieldSysPtr's task).
fn field_task(ram: &[u8]) -> bool {
    static PTR: OnceLock<Option<u32>> = OnceLock::new();
    let Some(ptr) = *PTR.get_or_init(|| {
        let root = PathBuf::from(std::env::var_os("PROBE_ROOT").unwrap_or_else(|| ".".into()));
        engine_symbol(&root, "gFieldSysPtr")
    }) else {
        return false;
    };
    let u32_at = |addr: u32| -> Option<u32> {
        let off = addr.checked_sub(0x0200_0000)? as usize;
        Some(u32::from_le_bytes(ram.get(off..off + 4)?.try_into().ok()?))
    };
    u32_at(ptr)
        .filter(|&f| (0x0200_0000..0x0240_0000).contains(&f))
        .and_then(|f| u32_at(f + 0x10))
        .is_some_and(|task| task != 0)
}

/// The touch screen saved as `<dir>/<name>.png`, and a line: whether the window would show both
/// screens, what the ROM's menu mirror holds and what the window's button finder sees (the
/// one-screen menu box's inputs).
pub(crate) fn capture_prompt(emu: &Emulator, dir: &std::path::Path, name: &str) -> String {
    let bottom: Vec<u32> = emu.nds.framebuffer(1).to_vec();
    let found = goldlink::hud::touch_choices(&bottom);
    let at = emu.mailbox_offset();
    let ram = emu.nds.main_ram();
    let lp = ram[at + LOCAL_PLAYER..at + LOCAL_PLAYER + PLAYER_STATE_SIZE].to_vec();
    let words = goldlink::goldlink_bridge::menu_text::read_menu(ram, &lp);
    let table = goldlink_rom::nitro::Charmap::new();
    let img = egui::ColorImage {
        size: [256, 192],
        pixels: bottom
            .iter()
            .map(|&p| egui::Color32::from_rgb((p >> 16) as u8, (p >> 8) as u8, p as u8))
            .collect(),
    };
    let _ = write_png(&dir.join(format!("{name}.png")), &img);
    let top: Vec<u32> = emu.nds.framebuffer(0).to_vec();
    let rows = &top[256 * goldlink::hud::SCENE_ROWS as usize..256 * 192];
    let scene = scene_of(&ram[at..at + MAILBOX_SIZE]);
    format!(
        "{name}: scene {scene:?}, text box {}; both screens (Pokégear {}, call {}); mirror {:?}; touch buttons {} {:?}",
        goldlink::hud::game_textbox_shown(rows),
        goldlink::hud::pokegear_shown(&bottom),
        goldlink::hud::phone_call_shown(&bottom),
        words.iter().map(|w| table.decode(w)).collect::<Vec<_>>(),
        found.len(),
        found
            .iter()
            .map(|c| (c.x0, c.y0, c.x1, c.y1, c.selected))
            .collect::<Vec<_>>()
    )
}

fn engine_symbol(root: &std::path::Path, name: &str) -> Option<u32> {
    let text = std::fs::read_to_string(root.join("hg-engine/build/rom_gen.ld")).ok()?;
    let prefix = format!("{name} = 0x");
    text.lines()
        .find_map(|l| {
            l.strip_prefix(&prefix)?
                .strip_suffix(';')
                .map(str::to_owned)
        })
        .and_then(|h| u32::from_str_radix(&h, 16).ok())
}

/// GOLDLINK_DEV_TOUR_ROUTE=pokecenter (UC11's lighting): the debug warp to Cherrygrove (in
/// front of its Pokémon Center), frames recorded walking the town, then inside the centre.
const POKECENTER_STEPS: &[Step] = &[
    Step::Until(Scene::Overworld, 60 * 120),
    Step::Wait(90),
    Step::Debug(C::COOP_DEBUG_WARP as u8, 1),
    Step::Wait(400),
    Step::Shot("cherrygrove"),
    Step::Walk(Keys::DOWN, 40, "town-down"),
    Step::Walk(Keys::UP, 40, "town-up"),
    Step::Walk(Keys::UP, 40, "enter"),
    Step::Wait(150),
    Step::Shot("pokecenter"),
    Step::Walk(Keys::UP, 48, "center-up"),
    Step::Wait(20),
    Step::Shot("pokecenter-counter"),
    Step::Walk(Keys::LEFT, 48, "center-left"),
    Step::Walk(Keys::RIGHT, 96, "center-right"),
    Step::Wait(20),
    Step::Shot("pokecenter-side"),
    Step::Walk(Keys::LEFT, 48, ""),
    Step::Walk(Keys::DOWN, 120, "center-down"),
    Step::Wait(20),
    Step::Shot("center-door"),
    Step::Close,
];

/// GOLDLINK_DEV_TOUR_ROUTE=walk (UC11's smooth camera): frames recorded walking down, up, left
/// and right from where the save stands.
const WALK_STEPS: &[Step] = &[
    Step::Until(Scene::Overworld, 60 * 120),
    Step::Wait(120),
    Step::Shot("start"),
    Step::Walk(Keys::DOWN, 96, "down"),
    Step::Walk(Keys::UP, 96, "up"),
    Step::Walk(Keys::LEFT, 96, "left"),
    Step::Walk(Keys::RIGHT, 96, "right"),
    Step::Wait(20),
    Step::Shot("end"),
    Step::Close,
];

/// GOLDLINK_DEV_TOUR_ROUTE=spot: the save's spot as it loads, its picture and main RAM (what's
/// drawn past the DS's view: which map blocks and materials, dev/docs/unified-client/UC11).
const SPOT_STEPS: &[Step] = &[
    Step::Until(Scene::Overworld, 60 * 120),
    Step::Wait(240),
    Step::Dump("spot"),
    Step::Poke,
    Step::Wait(4),
    Step::Shot("spot"),
    Step::Close,
];

/// GOLDLINK_DEV_TOUR_ROUTE=gate-out: from a save at a gatehouse's east door, into it and back
/// out (the map transitions both ways), every frame recorded.
const GATE_OUT_STEPS: &[Step] = &[
    Step::Until(Scene::Overworld, 60 * 120),
    Step::Wait(120),
    Step::Walk(Keys::LEFT, 40, "enter"),
    Step::Until(Scene::Overworld, 600),
    Step::Wait(60),
    Step::Walk(Keys::RIGHT, 150, "leave"),
    Step::Until(Scene::Overworld, 600),
    Step::Wait(60),
    Step::Shot("outside"),
    Step::Close,
];

/// GOLDLINK_DEV_TOUR_ROUTE=gate (UC11): from a save standing at a gatehouse's east door, into it,
/// then left and right inside, every frame recorded.
const GATE_STEPS: &[Step] = &[
    Step::Until(Scene::Overworld, 60 * 120),
    Step::Wait(120),
    Step::Shot("outside"),
    Step::Walk(Keys::LEFT, 40, "enter"),
    Step::Until(Scene::Overworld, 600),
    Step::Wait(90),
    Step::Shot("inside"),
    Step::Walk(Keys::LEFT, 64, "left"),
    Step::Walk(Keys::RIGHT, 48, "right"),
    Step::Wait(20),
    Step::Shot("end"),
    Step::Close,
];

/// GOLDLINK_DEV_TOUR_ROUTE=cave (UC11): from a save standing a tile below a cave's mouth, in
/// (one tile up) and out again, every frame recorded.
const CAVE_STEPS: &[Step] = &[
    Step::Until(Scene::Overworld, 60 * 120),
    Step::Wait(120),
    Step::Shot("outside"),
    Step::Walk(Keys::UP, 24, "in"),
    Step::Wait(240),
    Step::Until(Scene::Overworld, 1200),
    Step::Wait(240),
    Step::Shot("inside"),
    Step::Walk(Keys::DOWN, 24, "out"),
    Step::Wait(360),
    Step::Shot("back"),
    Step::Close,
];

/// GOLDLINK_DEV_TOUR_ROUTE=sideways (UC11): left, then right, every frame recorded.
const SIDEWAYS_STEPS: &[Step] = &[
    Step::Until(Scene::Overworld, 60 * 120),
    Step::Wait(120),
    Step::Shot("start"),
    Step::Walk(Keys::LEFT, 96, "left"),
    Step::Walk(Keys::RIGHT, 96, "right"),
    Step::Wait(20),
    Step::Shot("end"),
    Step::Close,
];

/// GOLDLINK_DEV_TOUR_ROUTE=far-left (UC11's neighbouring-area haze): from the save, left for 25
/// seconds (through gatehouses into the next areas); with GOLDLINK_DEV_TOUR_FILM, every frame.
const FAR_LEFT_STEPS: &[Step] = &[
    Step::Until(Scene::Overworld, 60 * 120),
    Step::Wait(120),
    Step::Shot("start"),
    Step::Walk(Keys::LEFT, 1500, ""),
    Step::Shot("end"),
    Step::Close,
];

/// GOLDLINK_DEV_TOUR_ROUTE=up5 (UC11's steady overworld): up for five seconds.
const UP5_STEPS: &[Step] = &[
    Step::Until(Scene::Overworld, 60 * 120),
    Step::Wait(60),
    Step::Walk(Keys::UP, 300, "up"),
    Step::Close,
];

/// GOLDLINK_DEV_TOUR_ROUTE=left-rec (UC11's steady overworld): left for 12 seconds, every frame
/// recorded (through a gatehouse from the user's save).
const LEFT_REC_STEPS: &[Step] = &[
    Step::Until(Scene::Overworld, 60 * 120),
    Step::Wait(60),
    Step::Walk(Keys::LEFT, 720, "left"),
    Step::Close,
];

/// GOLDLINK_DEV_TOUR_ROUTE=down-rec (UC11's steady overworld): down for 12 seconds, every frame
/// recorded (out of a gatehouse from the user's save).
const DOWN_REC_STEPS: &[Step] = &[
    Step::Until(Scene::Overworld, 60 * 120),
    Step::Wait(60),
    Step::Walk(Keys::DOWN, 720, "down"),
    Step::Close,
];

/// GOLDLINK_DEV_TOUR_ROUTE=down12: down for 12 seconds, nothing recorded (timing).
const DOWN12_STEPS: &[Step] = &[
    Step::Until(Scene::Overworld, 60 * 120),
    Step::Wait(60),
    Step::Walk(Keys::DOWN, 720, ""),
    Step::Close,
];

/// GOLDLINK_DEV_TOUR_ROUTE=mom (UC11's steady overworld): from the user's SAVE02 save, right to
/// the mother and A: a text box over the steady scene, every frame recorded.
const MOM_STEPS: &[Step] = &[
    Step::Until(Scene::Overworld, 60 * 120),
    Step::Wait(60),
    Step::Walk(Keys::RIGHT, 40, "right"),
    Step::Press(Keys::A),
    Step::Walk(Keys::NONE, 200, "talk"),
    Step::Close,
];

/// GOLDLINK_DEV_TOUR_ROUTE=starter: from the user's SAVE02 save facing Elm's Poké Balls, A and
/// what each screen shows after it, several times (the starter's choice in the one-screen layout).
const STARTER_STEPS: &[Step] = &[
    Step::Until(Scene::Overworld, 60 * 120),
    Step::Wait(60),
    Step::Shot("before"),
    Step::Press(Keys::A),
    Step::Wait(30),
    Step::Shot("a1"),
    Step::Wait(60),
    Step::Shot("a1-late"),
    Step::Raw("a1"),
    Step::Press(Keys::A),
    Step::Wait(60),
    Step::Shot("a2"),
    Step::Raw("a2"),
    Step::Press(Keys::A),
    Step::Wait(60),
    Step::Shot("a3"),
    Step::Raw("a3"),
    Step::Press(Keys::A),
    Step::Wait(90),
    Step::Shot("a4"),
    Step::Raw("a4"),
    Step::Touch(128, 85),
    Step::Wait(90),
    Step::Shot("b1"),
    Step::Wait(90),
    Step::Shot("b2"),
    Step::Press(Keys::A),
    Step::Wait(90),
    Step::Shot("b3"),
    Step::Close,
];

/// GOLDLINK_DEV_TOUR_ROUTE=mailbox: a step right and A (New Bark's mailbox beside Elm's house in
/// the user's SAVE02 save outside): a field message's box over the lit overworld.
const MAILBOX_STEPS: &[Step] = &[
    Step::Until(Scene::Overworld, 60 * 120),
    Step::Wait(60),
    Step::Walk(Keys::RIGHT, 20, ""),
    Step::Wait(20),
    Step::Press(Keys::A),
    Step::Wait(60),
    Step::Shot("m1"),
    Step::Press(Keys::A),
    Step::Wait(60),
    Step::Shot("m2"),
    Step::Close,
];

/// GOLDLINK_DEV_TOUR_ROUTE=talk-right: right toward the NPC beside the user's SAVE03 save's spot
/// in Cherrygrove and A: a person's message box over the lit overworld.
/// GOLDLINK_DEV_TOUR_ROUTE=left1: one tile left, every frame recorded as `rec-left1` and shots
/// before and after (KNOWN shadows: the NPC's emote bubble on the user's SAVE01 save).
const LEFT1_STEPS: &[Step] = &[
    Step::Until(Scene::Overworld, 60 * 120),
    Step::Wait(120),
    Step::Shot("before"),
    Step::Walk(Keys::LEFT, 16, ""),
    Step::Wait(10),
    Step::Shot("after1"),
    Step::Wait(10),
    Step::Shot("b1"),
    Step::Wait(10),
    Step::Shot("b2"),
    Step::Wait(10),
    Step::Shot("b3"),
    Step::Wait(10),
    Step::Shot("b4"),
    Step::Wait(10),
    Step::Shot("b5"),
    Step::Wait(10),
    Step::Shot("b6"),
    Step::Wait(10),
    Step::Shot("b7"),
    Step::Wait(10),
    Step::Shot("b8"),
    Step::Wait(120),
    Step::Shot("after3"),
    Step::Close,
];

/// GOLDLINK_DEV_TOUR_ROUTE=places: Cherrygrove, Violet City, New Bark Town and Route 29 (debug
/// warps), a shot at each (the shadows of fences and posts).
const PLACES_STEPS: &[Step] = &[
    Step::Until(Scene::Overworld, 60 * 120),
    Step::Wait(60),
    Step::Debug(C::COOP_DEBUG_WARP as u8, 1),
    Step::Until(Scene::Overworld, 600),
    Step::Wait(240),
    Step::Shot("cherrygrove"),
    Step::Debug(C::COOP_DEBUG_WARP as u8, 2),
    Step::Until(Scene::Overworld, 600),
    Step::Wait(240),
    Step::Shot("violet"),
    Step::Debug(C::COOP_DEBUG_WARP as u8, 0),
    Step::Until(Scene::Overworld, 600),
    Step::Wait(240),
    Step::Shot("newbark"),
    Step::Debug(C::COOP_DEBUG_WARP as u8, 13),
    Step::Until(Scene::Overworld, 600),
    Step::Wait(240),
    Step::Shot("route29"),
    Step::Close,
];

/// GOLDLINK_DEV_TOUR_ROUTE=through: two steps right (the follower behind), then back left
/// through the follower (with GOLDLINK_DEV_TOUR_FILM, every frame: the player's light).
const THROUGH_STEPS: &[Step] = &[
    Step::Until(Scene::Overworld, 60 * 120),
    Step::Wait(120),
    Step::Walk(Keys::RIGHT, 32, ""),
    Step::Wait(40),
    Step::Shot("right"),
    Step::Walk(Keys::LEFT, 40, ""),
    Step::Wait(30),
    Step::Shot("back"),
    Step::Close,
];

/// GOLDLINK_DEV_TOUR_ROUTE=grass: Route 29 (debug warp), then left through its tall grass (with
/// GOLDLINK_DEV_TOUR_FILM, every frame: the player's light in the grass).
const GRASS_STEPS: &[Step] = &[
    Step::Until(Scene::Overworld, 60 * 120),
    Step::Wait(60),
    Step::Debug(C::COOP_DEBUG_WARP as u8, 13),
    Step::Until(Scene::Overworld, 600),
    Step::Wait(240),
    Step::Shot("start"),
    Step::Walk(Keys::LEFT, 112, ""),
    Step::Wait(20),
    Step::Shot("grass"),
    Step::Close,
];

/// GOLDLINK_DEV_TOUR_ROUTE=prompts: the Cherrygrove nurse's question, then the start menu's Save
/// (the window and the touch screen at each: the one-screen layout's prompts).
const PROMPTS_STEPS: &[Step] = &[
    Step::Until(Scene::Overworld, 60 * 120),
    Step::Wait(90),
    Step::Debug(C::COOP_DEBUG_WARP as u8, 1),
    Step::Wait(400),
    Step::Walk(Keys::UP, 40, ""),
    Step::Wait(150),
    Step::Walk(Keys::UP, 120, ""),
    Step::Wait(20),
    Step::Press(Keys::A),
    Step::Wait(90),
    Step::Shot("nurse1"),
    Step::Raw("nurse1"),
    Step::Press(Keys::A),
    Step::Wait(90),
    Step::Shot("nurse2"),
    Step::Raw("nurse2"),
    Step::Press(Keys::A),
    Step::Wait(90),
    Step::Shot("nurse3"),
    Step::Raw("nurse3"),
    Step::Dump("nurse3"),
    Step::Press(Keys::B),
    Step::Wait(60),
    Step::Press(Keys::B),
    Step::Wait(60),
    Step::Settle(60 * 20),
    // The PC, right of the counter.
    Step::Walk(Keys::RIGHT, 48, ""),
    Step::Walk(Keys::UP, 16, ""),
    Step::Wait(20),
    Step::Press(Keys::A),
    Step::Wait(90),
    Step::Press(Keys::A),
    Step::Wait(90),
    Step::Shot("pc1"),
    Step::Raw("pc1"),
    Step::Dump("pc1"),
    Step::Press(Keys::A),
    Step::Wait(90),
    Step::Shot("pc2"),
    Step::Raw("pc2"),
    Step::Press(Keys::B),
    Step::Wait(60),
    Step::Press(Keys::B),
    Step::Wait(60),
    Step::Press(Keys::B),
    Step::Wait(60),
    Step::Settle(60 * 20),
    Step::Press(Keys::X),
    Step::Wait(60),
    Step::Shot("menu"),
    Step::Raw("menu"),
    Step::Touch(126, 80),
    Step::Wait(90),
    Step::Shot("save1"),
    Step::Raw("save1"),
    Step::Wait(90),
    Step::Shot("save2"),
    Step::Raw("save2"),
    Step::Close,
];

/// GOLDLINK_DEV_TOUR_ROUTE=gear: the Pokégear from the start menu, its phone, a call to the first
/// contact (the one-screen layout shows both screens while it's up).
const GEAR_STEPS: &[Step] = &[
    Step::Until(Scene::Overworld, 60 * 120),
    Step::Wait(90),
    Step::Press(Keys::X),
    Step::Wait(60),
    Step::Touch(40, 160),
    Step::Wait(150),
    Step::Shot("gear"),
    Step::Touch(172, 178),
    Step::Wait(90),
    Step::Shot("phone"),
    Step::Touch(60, 20),
    Step::Wait(60),
    Step::Shot("contact"),
    Step::Raw("contact"),
    Step::Press(Keys::A),
    Step::Wait(90),
    Step::Shot("call1"),
    Step::Raw("call1"),
    Step::Press(Keys::A),
    Step::Wait(120),
    Step::Shot("call2"),
    Step::Raw("call2"),
    Step::Press(Keys::A),
    Step::Wait(120),
    Step::Shot("call3"),
    Step::Close,
];

const TALK_RIGHT_STEPS: &[Step] = &[
    Step::Until(Scene::Overworld, 60 * 120),
    Step::Wait(60),
    Step::Walk(Keys::RIGHT, 56, ""),
    Step::Wait(20),
    Step::Press(Keys::A),
    Step::Wait(60),
    Step::Shot("t1"),
    Step::Press(Keys::A),
    Step::Wait(60),
    Step::Shot("t2"),
    Step::Close,
];

/// GOLDLINK_DEV_TOUR_ROUTE=follower: as talk-right, then a turn up to the person there and A: a
/// person's message over the lit overworld.
const FOLLOWER_STEPS: &[Step] = &[
    Step::Until(Scene::Overworld, 60 * 120),
    Step::Wait(60),
    Step::Walk(Keys::RIGHT, 56, ""),
    Step::Wait(30),
    Step::Walk(Keys::UP, 3, ""),
    Step::Wait(20),
    Step::Press(Keys::A),
    Step::Wait(15),
    Step::Shot("f1"),
    Step::Wait(15),
    Step::Shot("f2"),
    Step::Wait(30),
    Step::Shot("f3"),
    Step::Wait(60),
    Step::Shot("f4"),
    Step::Close,
];

/// GOLDLINK_DEV_TOUR_ROUTE=lyra (UC11's wider picture): from the user's SAVE02 save, down out of
/// the house into the New Bark Town scene, A every 40 frames for its text (GOLDLINK_DEV_TOUR_FILM).
const LYRA_STEPS: &[Step] = &[
    Step::Until(Scene::Overworld, 60 * 120),
    Step::Wait(60),
    Step::Walk(Keys::DOWN, 90, ""),
    Step::Walk(Keys::NONE, 240, ""),
    Step::Mash(60 * 40),
    Step::Close,
];

/// GOLDLINK_DEV_TOUR_ROUTE=motion (UC11's steady overworld): from the save, up and down running,
/// cycling and walking, then the debug warp to Route 40, south to the sea and surfing down,
/// right and up; every frame recorded (each its own rec- folder).
const MOTION_STEPS: &[Step] = &[
    Step::Until(Scene::Overworld, 60 * 120),
    Step::Wait(60),
    Step::Walk(Keys(Keys::UP.0 | Keys::B.0), 90, "run-up"),
    Step::Walk(Keys(Keys::DOWN.0 | Keys::B.0), 90, "run-down"),
    Step::Debug(C::COOP_DEBUG_TOGGLE_BIKE as u8, 0),
    Step::Wait(60),
    Step::Walk(Keys::UP, 90, "bike-up"),
    Step::Walk(Keys::DOWN, 90, "bike-down"),
    Step::Debug(C::COOP_DEBUG_TOGGLE_BIKE as u8, 0),
    Step::Wait(60),
    Step::Walk(Keys::UP, 90, "walk-up"),
    Step::Walk(Keys::DOWN, 90, "walk-down"),
    Step::Debug(C::COOP_DEBUG_WARP as u8, 6),
    Step::Wait(420),
    Step::Walk(Keys::DOWN, 200, ""),
    Step::Debug(C::COOP_DEBUG_SURF as u8, 0),
    Step::Wait(150),
    Step::Walk(Keys::DOWN, 120, "surf-down"),
    Step::Walk(Keys::RIGHT, 90, "surf-right"),
    Step::Walk(Keys::UP, 60, "surf-up"),
    Step::Close,
];

/// GOLDLINK_DEV_TOUR_ROUTE=route40: the debug warp to Route 40 (its beach), a picture.
const ROUTE40_STEPS: &[Step] = &[
    Step::Until(Scene::Overworld, 60 * 120),
    Step::Wait(60),
    Step::Debug(C::COOP_DEBUG_WARP as u8, 6),
    Step::Wait(420),
    Step::Shot("route40"),
    Step::Close,
];

/// GOLDLINK_DEV_TOUR_ROUTE=stand (UC11's steady overworld): the save's spot, standing, every frame
/// recorded for a second.
const STAND_STEPS: &[Step] = &[
    Step::Until(Scene::Overworld, 60 * 120),
    Step::Wait(120),
    Step::Walk(Keys::NONE, 60, "stand"),
    Step::Close,
];

/// GOLDLINK_DEV_TOUR_ROUTE=ghost-a / ghost-b (UC12, dev/run_visual_pair.sh-like pair): both warp to
/// New Bark Town; B steps two tiles down and stands (their follower behind them); A then walks down
/// through B's follower and B and back up past them, every frame recorded.
const GHOST_STEPS: [&[Step]; 2] = [
    &[
        Step::Until(Scene::Overworld, 60 * 120),
        Step::Wait(60),
        Step::Mark("ready"),
        Step::Peer("ready", 9000),
        Step::Debug(C::COOP_DEBUG_WARP as u8, 0),
        Step::Wait(240),
        Step::Mark("warped"),
        Step::Peer("placed", 6000),
        Step::Wait(120),
        Step::Walk(Keys::DOWN, 96, "down"),
        Step::Walk(Keys::NONE, 60, "stand"),
        Step::Walk(Keys::UP, 96, "up"),
        Step::Walk(Keys::NONE, 60, "after"),
        Step::Close,
    ],
    &[
        Step::Until(Scene::Overworld, 60 * 120),
        Step::Wait(60),
        Step::Mark("ready"),
        Step::Peer("ready", 9000),
        Step::Debug(C::COOP_DEBUG_WARP as u8, 0),
        Step::Wait(240),
        Step::Peer("warped", 6000),
        Step::Walk(Keys::DOWN, 34, ""),
        Step::Wait(60),
        Step::Mark("placed"),
        Step::Wait(60 * 20),
        Step::Close,
    ],
];

/// GOLDLINK_DEV_TOUR_ROUTE=shoes-tap (the running shoes): the field touch screen's shoes button
/// tapped, before and after (two screens shown).
const SHOES_TAP_STEPS: &[Step] = &[
    Step::Until(Scene::Overworld, 60 * 120),
    Step::Wait(90),
    Step::Shot("before"),
    Step::Touch(215, 110),
    Step::Wait(60),
    Step::Shot("after"),
    Step::Touch(215, 110),
    Step::Wait(60),
    Step::Shot("again"),
    Step::Close,
];

/// GOLDLINK_DEV_TOUR_ROUTE=run-lock (the running shoes' button): the start menu's pop-up, then
/// running locked on and a walk up, every frame recorded.
const RUN_LOCK_STEPS: &[Step] = &[
    Step::Until(Scene::Overworld, 60 * 120),
    Step::Wait(60),
    Step::Press(Keys::X),
    Step::Until(Scene::StartMenu, 120),
    Step::Wait(40),
    Step::Running(false),
    Step::Wait(10),
    Step::Shot("menu-off"),
    Step::Running(true),
    Step::Wait(10),
    Step::Shot("menu-on"),
    Step::Press(Keys::B),
    Step::Until(Scene::Overworld, 300),
    Step::Wait(30),
    Step::Walk(Keys::UP, 60, "run"),
    Step::Running(false),
    Step::Walk(Keys::DOWN, 60, "walk"),
    Step::Close,
];

/// GOLDLINK_DEV_TOUR_ROUTE=step-down (UC11's steady overworld): one step down from the save's spot,
/// then standing, every frame recorded.
const STEP_DOWN_STEPS: &[Step] = &[
    Step::Until(Scene::Overworld, 60 * 120),
    Step::Wait(120),
    Step::Walk(Keys::DOWN, 10, "step"),
    Step::Walk(Keys::NONE, 60, "after"),
    Step::Close,
];

/// GOLDLINK_DEV_TOUR_ROUTE=down-past (UC11's steady overworld): down for two seconds from the save's
/// spot (past what's in front of the player), every frame recorded.
const DOWN_PAST_STEPS: &[Step] = &[
    Step::Until(Scene::Overworld, 60 * 120),
    Step::Wait(120),
    Step::Walk(Keys::DOWN, 120, "down"),
    Step::Close,
];

/// GOLDLINK_DEV_TOUR_ROUTE=settle-right (UC13): the field's message closed, then right for three
/// seconds and standing (the user's SAVE02 room: past the desk and the television).
const SETTLE_RIGHT_STEPS: &[Step] = &[
    Step::Until(Scene::Overworld, 60 * 120),
    Step::Mash(900),
    Step::Settle(1200),
    Step::Walk(Keys::RIGHT, 180, ""),
    Step::Walk(Keys::NONE, 60, ""),
    Step::Close,
];

/// GOLDLINK_DEV_TOUR_ROUTE=cave-steps (UC13): from a save a tile below a cave's mouth, in, then a
/// step up, a step left and standing (the user: Dark Cave went black after a step).
const CAVE_STEPS_IN: &[Step] = &[
    Step::Until(Scene::Overworld, 60 * 120),
    Step::Wait(60),
    Step::Walk(Keys::UP, 40, ""),
    Step::Wait(240),
    Step::Walk(Keys::UP, 48, ""),
    Step::Walk(Keys::LEFT, 48, ""),
    Step::Walk(Keys::RIGHT, 48, ""),
    Step::Walk(Keys::NONE, 30, "cave"),
    Step::Close,
];

/// GOLDLINK_DEV_TOUR_ROUTE=tab: Tab shows the touch screen, Tab hides it, a shot of each (the
/// window logs whenever a button holds the keyboard: the game's keys are ignored then).
const TAB_STEPS: &[Step] = &[
    Step::Until(Scene::Overworld, 60 * 120),
    Step::Wait(120),
    Step::Type("\t"),
    Step::Wait(40),
    Step::Shot("tab-shown"),
    Step::Type("\t"),
    Step::Wait(40),
    Step::Shot("tab-hidden"),
    Step::Close,
];

/// GOLDLINK_DEV_TOUR_ROUTE=down2 (UC13): two steps down, then standing (the user's Violet City
/// lanterns: the player between them).
const DOWN2_STEPS: &[Step] = &[
    Step::Until(Scene::Overworld, 60 * 120),
    Step::Wait(120),
    Step::Walk(Keys::DOWN, 32, ""),
    Step::Walk(Keys::NONE, 90, ""),
    Step::Close,
];

/// GOLDLINK_DEV_TOUR_ROUTE=down1 / down1-left2 (UC13): a step down (then two left), then
/// standing: the player level with the user's lanterns, on open ground.
const DOWN1_STEPS: &[Step] = &[
    Step::Until(Scene::Overworld, 60 * 120),
    Step::Wait(120),
    Step::Walk(Keys::DOWN, 16, ""),
    Step::Walk(Keys::NONE, 90, ""),
    Step::Close,
];
const DOWN1_LEFT2_STEPS: &[Step] = &[
    Step::Until(Scene::Overworld, 60 * 120),
    Step::Wait(120),
    Step::Walk(Keys::DOWN, 16, ""),
    Step::Walk(Keys::LEFT, 32, ""),
    Step::Walk(Keys::NONE, 90, ""),
    Step::Close,
];

/// GOLDLINK_DEV_TOUR_ROUTE=right5 (UC11's steady overworld): right for five seconds.
const RIGHT5_STEPS: &[Step] = &[
    Step::Until(Scene::Overworld, 60 * 120),
    Step::Wait(60),
    Step::Walk(Keys::RIGHT, 300, "right"),
    Step::Close,
];

/// GOLDLINK_DEV_TOUR_ROUTE=battle-intro (UC11's smooth camera): a wild battle's intro, every
/// frame recorded.
const BATTLE_INTRO_STEPS: &[Step] = &[
    Step::Until(Scene::Overworld, 60 * 120),
    Step::Wait(90),
    Step::Debug(C::COOP_DEBUG_WILD_BATTLE as u8, 2),
    Step::Walk(Keys::NONE, 900, "intro"),
    Step::Close,
];

/// GOLDLINK_DEV_TOUR_ROUTE=link-a / link-b (dev/run_visual_pair.sh link): two windows in one
/// room, a link battle between them (debug command 12, 6 on A and 7 on B, as
/// link_e2e_probe.lua), A every 40 frames through it.
const LINK_STEPS: [&[Step]; 2] = [
    &[
        Step::Until(Scene::Overworld, 60 * 120),
        Step::Wait(60),
        Step::Press(Keys::B),
        Step::Wait(30),
        Step::Mark("ready"),
        Step::Peer("ready", 9000),
        Step::Wait(120),
        Step::Debug(C::COOP_DEBUG_LINK as u8, 6),
        Step::Mash(60 * 150),
        Step::Close,
    ],
    &[
        Step::Until(Scene::Overworld, 60 * 120),
        Step::Wait(60),
        Step::Press(Keys::B),
        Step::Wait(30),
        Step::Mark("ready"),
        Step::Peer("ready", 9000),
        Step::Wait(120),
        Step::Debug(C::COOP_DEBUG_LINK as u8, 7),
        Step::Mash(60 * 150),
        Step::Close,
    ],
];

/// GOLDLINK_DEV_TOUR_ROUTE=boss-a / boss-b (dev/run_visual_pair.sh boss): two teamed windows
/// fight Falkner together (the 2v2 team boss, as link_e2e_probe.lua's boss mode): a lead
/// (debug 14 preset 4), the warp to Violet Gym (debug 5 preset 10), up to him, A.
const BOSS_STEPS: [&[Step]; 2] = [
    &[
        Step::Until(Scene::Overworld, 60 * 120),
        Step::Wait(60),
        Step::Press(Keys::B),
        Step::Wait(30),
        Step::Mark("ready"),
        Step::Peer("ready", 9000),
        Step::Debug(C::COOP_DEBUG_GIVE_TEST_MON as u8, 4),
        Step::Wait(300),
        Step::Debug(C::COOP_DEBUG_WARP as u8, 10),
        Step::Wait(600),
        Step::Walk(Keys::UP, 24, ""),
        Step::Wait(40),
        Step::Mark("at_boss"),
        Step::Peer("at_boss", 3600),
        Step::Press(Keys::A),
        Step::Mash(60 * 240),
        Step::Close,
    ],
    &[
        Step::Until(Scene::Overworld, 60 * 120),
        Step::Wait(60),
        Step::Press(Keys::B),
        Step::Wait(30),
        Step::Mark("ready"),
        Step::Peer("ready", 9000),
        Step::Debug(C::COOP_DEBUG_GIVE_TEST_MON as u8, 4),
        Step::Wait(300),
        Step::Debug(C::COOP_DEBUG_WARP as u8, 10),
        Step::Wait(600),
        Step::Walk(Keys::UP, 24, ""),
        Step::Wait(40),
        Step::Mark("at_boss"),
        Step::Peer("at_boss", 3600),
        // B a little after A, so A waits first.
        Step::Wait(300),
        Step::Press(Keys::A),
        Step::Mash(60 * 240),
        Step::Close,
    ],
];

/// GOLDLINK_DEV_TOUR_ROUTE=team-a ... team-d (dev/run_visual_pair.sh team): four windows, A+B
/// against C+D (the campaign's teams), as link_e2e_probe.lua's team mode: leads (debug 14:
/// preset 5 Golem for A and B, 6 Magikarp for C and D), A asks C to battle (debug 12, 8 + C's
/// remote slot 1), the others say Yes to the team battle's prompt (A every 40 frames).
const TEAM_STEPS: [&[Step]; 3] = [
    &[
        Step::Until(Scene::Overworld, 60 * 120),
        Step::Wait(60),
        Step::Press(Keys::B),
        Step::Wait(30),
        Step::Mark("ready"),
        Step::Peer("ready", 9000),
        Step::Settle(1800),
        Step::Debug(C::COOP_DEBUG_GIVE_TEST_MON as u8, 5),
        Step::Wait(300),
        Step::Mark("lead"),
        Step::Peer("lead", 6000),
        Step::Wait(60),
        Step::Debug(C::COOP_DEBUG_LINK as u8, 8 + 1),
        Step::MashTask(6000),
        Step::Mash(60 * 240),
        Step::Close,
    ],
    &[
        Step::Until(Scene::Overworld, 60 * 120),
        Step::Wait(60),
        Step::Press(Keys::B),
        Step::Wait(30),
        Step::Mark("ready"),
        Step::Peer("ready", 9000),
        Step::Settle(1800),
        Step::Debug(C::COOP_DEBUG_GIVE_TEST_MON as u8, 5),
        Step::Wait(300),
        Step::Mark("lead"),
        Step::Peer("lead", 6000),
        Step::MashTask(6000),
        Step::Mash(60 * 240),
        Step::Close,
    ],
    &[
        Step::Until(Scene::Overworld, 60 * 120),
        Step::Wait(60),
        Step::Press(Keys::B),
        Step::Wait(30),
        Step::Mark("ready"),
        Step::Peer("ready", 9000),
        Step::Settle(1800),
        Step::Debug(C::COOP_DEBUG_GIVE_TEST_MON as u8, 6),
        Step::Wait(300),
        Step::Mark("lead"),
        Step::Peer("lead", 6000),
        Step::MashTask(6000),
        Step::Mash(60 * 240),
        Step::Close,
    ],
];

/// One recorded frame: `proj` (16 x i32, 20.12), `w_depth` (u32), the map (u32), the activity
/// (u32), the wide picture's side (u32); then the wide top screen ((256 + 2 * side) x 192 u32,
/// 0x00RRGGBB), the depth (384 x 192 u32) and the layer mask (384 x 192 u8). Nothing when the
/// top screen doesn't show the 3D scene.
fn record_frame(emu: &Emulator, path: &std::path::Path) -> std::io::Result<()> {
    use goldlink_abi::{CoopCommBlock, CoopPlayerState};
    use std::mem::offset_of;
    let mut b = goldlink_core::LightBuffers::default();
    if !emu.nds.light_buffers(&mut b) {
        // No 3D scene on the top screen: the picture all the same, the buffers empty.
        let n = goldlink_core::LightBuffers::WIDTH * 192;
        b.depth = vec![0; n];
        b.layer = vec![0; n];
        b.proj = [0; 16];
    }
    let player = offset_of!(CoopCommBlock, local_player);
    let (activity, map) = emu.mailbox().map_or((0, 0), |mb| {
        let m = player + offset_of!(CoopPlayerState, map_id);
        (
            mb[player + offset_of!(CoopPlayerState, activity)] as u32,
            u16::from_le_bytes([mb[m], mb[m + 1]]) as u32,
        )
    });
    let side = emu.wide();
    let wide = if side > 0 {
        emu.nds.wide_frame(0, side)
    } else {
        emu.nds.framebuffer(0).to_vec()
    };
    let mut out = Vec::with_capacity(84 + wide.len() * 4 + b.depth.len() * 5);
    for v in b.proj {
        out.extend_from_slice(&v.to_le_bytes());
    }
    for v in [u32::from(b.w_depth), map, activity, side as u32] {
        out.extend_from_slice(&v.to_le_bytes());
    }
    for v in wide.iter().chain(&b.depth) {
        out.extend_from_slice(&v.to_le_bytes());
    }
    out.extend_from_slice(&b.layer);
    // Then each pixel's polygon ID (the ROM tags a neighbouring area's untextured blocks 62).
    out.extend_from_slice(&b.ids);
    // Then the steady overworld's attached picture (Nds::attached): its shift (two f32, NaN with
    // none) and its pixels (as wide as the picture: 1 opaque, 2 translucent, else 0).
    let (shift, mask) = emu
        .nds
        .attached(side)
        .map_or(([f32::NAN; 2], vec![0; wide.len()]), |a| {
            let mask = a
                .att
                .iter()
                .map(|&c| match c >> 24 {
                    0 => 0,
                    255 => 1,
                    _ => 2,
                })
                .collect();
            (a.shift, mask)
        });
    for v in shift {
        out.extend_from_slice(&v.to_le_bytes());
    }
    out.extend_from_slice(&mask);
    std::fs::write(path, out)?;
    // The battle HUD's view of the frame (UC9's band), beside it.
    let ui = goldlink::goldlink_bridge::battle_ui::read(emu.nds.main_ram());
    let line = match ui {
        Some(b) => format!(
            "menu {} waiting {} message {} boxes {} drawn {}\n",
            b.menu,
            b.waiting,
            b.message.len(),
            b.boxes.len(),
            b.boxes.iter().filter(|x| x.drawn).count()
        ),
        None => "no battle ui\n".to_owned(),
    };
    std::fs::write(path.with_extension("txt"), line)
}

fn steps() -> &'static [Step] {
    match std::env::var("GOLDLINK_DEV_TOUR_ROUTE").as_deref() {
        Ok("faint") => FAINT_STEPS,
        Ok("intro") => intro_steps(),
        Ok("learn") => LEARN_STEPS,
        Ok("mega") => MEGA_STEPS,
        Ok("naming") => NAMING_STEPS,
        Ok("catch") => CATCH_STEPS,
        Ok("wide") => WIDE_STEPS,
        Ok("walk-right") => WALK_RIGHT_STEPS,
        Ok("heap") => HEAP_STEPS,
        Ok("area-border") => AREA_BORDER_STEPS,
        Ok("starter-prompt") => PROMPT_STEPS,
        Ok("pokecenter") => POKECENTER_STEPS,
        Ok("walk") => WALK_STEPS,
        Ok("gate") => GATE_STEPS,
        Ok("gate-out") => GATE_OUT_STEPS,
        Ok("spot") => SPOT_STEPS,
        Ok("cave") => CAVE_STEPS,
        Ok("sideways") => SIDEWAYS_STEPS,
        Ok("far-left") => FAR_LEFT_STEPS,
        Ok("up5") => UP5_STEPS,
        Ok("right5") => RIGHT5_STEPS,
        Ok("settle-right") => SETTLE_RIGHT_STEPS,
        Ok("down2") => DOWN2_STEPS,
        Ok("tab") => TAB_STEPS,
        Ok("down1") => DOWN1_STEPS,
        Ok("down1-left2") => DOWN1_LEFT2_STEPS,
        Ok("cave-steps") => CAVE_STEPS_IN,
        Ok("mom") => MOM_STEPS,
        Ok("starter") => STARTER_STEPS,
        Ok("mailbox") => MAILBOX_STEPS,
        Ok("talk-right") => TALK_RIGHT_STEPS,
        Ok("left1") => LEFT1_STEPS,
        Ok("places") => PLACES_STEPS,
        Ok("through") => THROUGH_STEPS,
        Ok("grass") => GRASS_STEPS,
        Ok("prompts") => PROMPTS_STEPS,
        Ok("gear") => GEAR_STEPS,
        Ok("follower") => FOLLOWER_STEPS,
        Ok("stand") => STAND_STEPS,
        Ok("step-down") => STEP_DOWN_STEPS,
        Ok("run-lock") => RUN_LOCK_STEPS,
        Ok("shoes-tap") => SHOES_TAP_STEPS,
        Ok("ghost-a") => GHOST_STEPS[0],
        Ok("ghost-b") => GHOST_STEPS[1],
        Ok("down-past") => DOWN_PAST_STEPS,
        Ok("motion") => MOTION_STEPS,
        Ok("route40") => ROUTE40_STEPS,
        Ok("lyra") => LYRA_STEPS,
        Ok("left-rec") => LEFT_REC_STEPS,
        Ok("down-rec") => DOWN_REC_STEPS,
        Ok("down12") => DOWN12_STEPS,
        Ok("battle-intro") => BATTLE_INTRO_STEPS,
        Ok("link-a") => LINK_STEPS[0],
        Ok("link-b") => LINK_STEPS[1],
        Ok("boss-a") => BOSS_STEPS[0],
        Ok("boss-b") => BOSS_STEPS[1],
        Ok("team-a") => TEAM_STEPS[0],
        Ok("team-b") => TEAM_STEPS[1],
        Ok("team-c") => TEAM_STEPS[2],
        Ok("team-d") => TEAM_STEPS[2],
        _ => STEPS,
    }
}

const STEPS: &[Step] = &[
    Step::Until(Scene::Overworld, 60 * 120),
    Step::Wait(90),
    Step::Shot("overworld"),
    Step::Press(Keys::X),
    Step::Until(Scene::StartMenu, 120),
    Step::Wait(40),
    Step::Shot("start-menu"),
    Step::Press(Keys::A),
    Step::Until(Scene::App, 600),
    Step::Wait(120),
    Step::Shot("application"),
    Step::Press(Keys::B),
    // Back in the start menu, then out of it.
    Step::Until(Scene::StartMenu, 900),
    Step::Wait(60),
    Step::Press(Keys::B),
    Step::Until(Scene::Overworld, 300),
    Step::Wait(30),
    Step::Debug(C::COOP_DEBUG_WILD_BATTLE as u8, 5),
    Step::Until(Scene::Battle, 600),
    Step::Wait(330),
    Step::Shot("battle-text"),
    Step::Wait(60 * 11),
    Step::Shot("battle"),
    // The first A shows the cursor on FIGHT.
    Step::Press(Keys::A),
    Step::Wait(30),
    Step::Shot("battle-cursor"),
    Step::Press(Keys::DOWN),
    Step::Wait(20),
    Step::Shot("battle-cursor-bag"),
    Step::Press(Keys::UP),
    Step::Wait(20),
    // A tap on the game's FIGHT button, as the HUD's click makes.
    Step::Touch(128, 84),
    Step::Wait(90),
    Step::Shot("battle-moves"),
    Step::Press(Keys::RIGHT),
    Step::Wait(30),
    Step::Shot("battle-moves-cursor"),
    // Tackle: the HP box drains with the game's bar, then the EXP bar fills.
    Step::Press(Keys::A),
    Step::Wait(330),
    Step::Shot("battle-attack"),
    Step::Wait(20),
    Step::Shot("battle-drain"),
    Step::Wait(40),
    Step::Shot("battle-drained"),
    Step::Wait(240),
    Step::Shot("battle-exp"),
    Step::Close,
];

#[derive(Default)]
struct State {
    step: usize,
    frames: u32,
    /// A press in progress: frames left holding it.
    holding: u32,
    shot: Option<String>,
    requested: bool,
    taken: usize,
    seq: u8,
    log: Vec<String>,
    sized: bool,
    /// Whether a button held the keyboard last frame (logged as it changes).
    focused: bool,
    last_menu: Option<(i8, bool)>,
    /// A debug command waiting for the ROM's answer: its sequence number, command, and frames.
    pending: Option<(u8, u8, u32)>,
    /// Text for the window's keyboard, taken by `input()` before the next frame.
    typed: Option<&'static str>,
    /// Emulated frames so far, and the last one filmed (GOLDLINK_DEV_TOUR_FILM=1: the window
    /// saved at every emulated frame, as `<dir>/film/<frame>.png`, for the lighting's and the
    /// smooth camera's frame-by-frame checks).
    emu_frames: u64,
    /// Frames the field has been free in a row (Step::Settle).
    idle: u32,
    /// Frames shown so far (the window has them), as of the last pre_frame.
    published: u64,
    filmed: u64,
    film_requested: bool,
    wide: Option<crate::wide::Pass>,
}

fn filming() -> bool {
    std::env::var("GOLDLINK_DEV_TOUR_FILM").is_ok_and(|v| v == "1")
}

/// This window's role and the others' (GOLDLINK_DEV_TOUR_ROLES, default "AB").
fn role() -> (String, Vec<String>) {
    let me = std::env::var("GOLDLINK_DEV_TOUR_ROLE").unwrap_or_else(|_| "A".into());
    let all = std::env::var("GOLDLINK_DEV_TOUR_ROLES").unwrap_or_else(|_| "AB".into());
    let peers = all
        .chars()
        .map(|c| c.to_string())
        .filter(|r| *r != me)
        .collect();
    (me, peers)
}

fn mark_path(role: &str, name: &str) -> PathBuf {
    let dir = std::env::var_os("GOLDLINK_DEV_TOUR_MARKS")
        .map_or_else(|| PathBuf::from("."), PathBuf::from);
    dir.join(format!("mark_{role}_{name}"))
}

#[derive(Clone)]
pub struct Tour {
    state: Arc<Mutex<State>>,
    dir: PathBuf,
}

/// The tour, when GOLDLINK_DEV_TOUR names a folder: one for the hook and the window's part.
pub fn shared() -> Option<Tour> {
    static TOUR: OnceLock<Option<Tour>> = OnceLock::new();
    TOUR.get_or_init(|| {
        let dir = PathBuf::from(std::env::var_os("GOLDLINK_DEV_TOUR")?);
        std::fs::create_dir_all(&dir).ok()?;
        Some(Tour {
            state: Arc::default(),
            dir,
        })
    })
    .clone()
}

fn write_png(path: &std::path::Path, img: &egui::ColorImage) -> Result<(), String> {
    let mut rgb = Vec::with_capacity(img.pixels.len() * 3);
    for p in &img.pixels {
        rgb.extend_from_slice(&[p.r(), p.g(), p.b()]);
    }
    let file = std::fs::File::create(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut enc = png::Encoder::new(
        std::io::BufWriter::new(file),
        img.size[0] as u32,
        img.size[1] as u32,
    );
    enc.set_color(png::ColorType::Rgb);
    enc.set_depth(png::BitDepth::Eight);
    let mut writer = enc.write_header().map_err(|e| e.to_string())?;
    writer.write_image_data(&rgb).map_err(|e| e.to_string())
}

impl Tour {
    fn state(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// The window's input: what Step::Type types, as key events and text.
    pub fn input(&self) -> goldlink::ExtraInput {
        let tour = self.clone();
        Box::new(move |raw| {
            let Some(text) = tour.state().typed.take() else {
                return;
            };
            // A headless window may not have the focus; a player typing does.
            raw.focused = true;
            let key = |key| egui::Event::Key {
                key,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            };
            for c in text.chars() {
                raw.events.push(match c {
                    '\n' => key(egui::Key::Enter),
                    '\t' => key(egui::Key::Tab),
                    '\x08' => key(egui::Key::Backspace),
                    c => egui::Event::Text(c.to_string()),
                });
            }
        })
    }

    pub fn hook(&self) -> TourHook {
        TourHook(self.clone())
    }

    /// The window's part: screenshots when the game side asks, and closing at the end.
    pub fn ui(&self, ctx: &egui::Context) {
        let mut s = self.state();
        // A button holding the keyboard (the game's keys are ignored then): logged as it changes.
        let focused = ctx.memory(|m| m.focused().is_some());
        if focused != s.focused {
            s.focused = focused;
            eprintln!("tour: a button holds the keyboard: {focused}");
        }
        // GOLDLINK_DEV_TOUR_SIZE=WxH: the window's inner size for the shots (once).
        if !s.sized {
            s.sized = true;
            if let Some((w, h)) = std::env::var("GOLDLINK_DEV_TOUR_SIZE")
                .ok()
                .and_then(|v| v.split_once('x').map(|(w, h)| (w.parse(), h.parse())))
                .and_then(|(w, h)| Some((w.ok()?, h.ok()?)))
            {
                ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(egui::vec2(w, h)));
            }
        }
        let image = ctx.input(|i| {
            i.raw.events.iter().find_map(|e| match e {
                egui::Event::Screenshot { image, .. } => Some(image.clone()),
                _ => None,
            })
        });
        // The film: a picture per emulated frame, written on its own thread.
        if s.film_requested {
            if let Some(img) = &image {
                s.film_requested = false;
                let path = self.dir.join("film").join(format!("{:06}.png", s.filmed));
                let img = img.clone();
                std::thread::spawn(move || {
                    let _ = std::fs::create_dir_all(path.parent().expect("film folder"));
                    if let Err(e) = write_png(&path, &img) {
                        eprintln!("tour: film: {e}");
                    }
                });
            }
        } else if let Some(name) = s.shot.clone() {
            if !s.requested {
                ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::default()));
                s.requested = true;
            }
            if let Some(img) = image {
                s.taken += 1;
                let path = self.dir.join(format!("{}-{name}.png", s.taken));
                let line = match write_png(&path, &img) {
                    Ok(()) => format!("shot {}", path.display()),
                    Err(e) => format!("shot {name} failed: {e}"),
                };
                eprintln!("tour: {line}");
                s.log.push(line);
                s.shot = None;
                s.requested = false;
            }
        }
        if filming() && s.shot.is_none() && !s.film_requested && s.published != s.filmed {
            s.filmed = s.published;
            s.film_requested = true;
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::default()));
        }
        if s.shot.is_none()
            && steps()
                .get(s.step)
                .is_some_and(|st| matches!(st, Step::Close))
        {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
        ctx.request_repaint();
    }
}

pub struct TourHook(Tour);

impl FrameHook for TourHook {
    fn pre_frame(&mut self, emu: &mut Emulator, input: &mut Input) {
        // Filming: the emulation waits (a while at most) for the window to film the last frame,
        // so every frame is filmed.
        if filming() {
            let start = std::time::Instant::now();
            loop {
                let mut s = self.0.state();
                s.published = s.emu_frames;
                if s.published == 0
                    || (s.filmed == s.published && !s.film_requested)
                    || start.elapsed() > std::time::Duration::from_millis(300)
                {
                    break;
                }
                drop(s);
                std::thread::sleep(std::time::Duration::from_millis(1));
            }
        }
        let mut s = self.0.state();
        s.emu_frames += 1;
        if s.holding > 0 {
            s.holding -= 1;
            match s.step.checked_sub(1).and_then(|i| steps().get(i)) {
                Some(Step::Press(k)) => input.keys |= *k,
                Some(Step::Touch(x, y)) => input.touch = Some((*x, *y)),
                _ => {}
            }
        }
        if s.shot.is_some() || s.holding > 0 {
            return;
        }
        let Some(step) = steps().get(s.step).copied() else {
            return;
        };
        // A debug command's answer before the next step (the ROM refuses some while busy).
        if let Some((seq, cmd, frames)) = s.pending {
            let at = emu.mailbox_offset();
            let ack = at + DIAG + std::mem::offset_of!(goldlink_abi::CoopDiagnostics, debug_ack);
            let ram = emu.nds.main_ram();
            let line = if ram[ack] == seq {
                Some(format!("debug {cmd}: result {}", ram[at + DEBUG_RESULT]))
            } else if frames > 600 {
                Some(format!("debug {cmd}: no answer"))
            } else {
                None
            };
            match line {
                Some(line) => {
                    eprintln!("tour: {line}");
                    s.log.push(line);
                    s.pending = None;
                }
                None => {
                    s.pending = Some((seq, cmd, frames + 1));
                    return;
                }
            }
        }
        let at = emu.mailbox_offset();
        let scene = if emu.mailbox().is_some() {
            let ram = emu.nds.main_ram();
            scene_of(&ram[at..at + MAILBOX_SIZE])
        } else {
            Scene::Unknown
        };
        s.frames += 1;
        let next = match step {
            Step::Until(want, max) => {
                if scene != want && s.frames >= max {
                    let line = format!(
                        "step {}: gave up waiting for {want:?} (at {scene:?})",
                        s.step
                    );
                    eprintln!("tour: {line}");
                    s.log.push(line);
                }
                scene == want || s.frames >= max
            }
            Step::Wait(n) => s.frames >= n,
            Step::Mash(n) => {
                if s.frames % 40 < 3 {
                    input.keys |= Keys::A;
                }
                s.frames >= n
            }
            Step::Settle(max) => {
                let busy = field_task(emu.nds.main_ram());
                if busy {
                    s.idle = 0;
                    if s.frames % 30 < 3 {
                        input.keys |= Keys::A;
                    }
                } else {
                    s.idle += 1;
                }
                s.idle >= 60 || s.frames >= max
            }
            Step::MashTask(max) => {
                if field_task(emu.nds.main_ram()) && s.frames % 40 < 3 {
                    input.keys |= Keys::A;
                }
                if s.frames >= max {
                    let line = format!("step {}: gave up waiting to leave the field", s.step);
                    eprintln!("tour: {line}");
                    s.log.push(line);
                }
                (scene != Scene::Overworld && scene != Scene::Unknown) || s.frames >= max
            }
            Step::Mark(name) => {
                let path = mark_path(&role().0, name);
                if let Err(e) = std::fs::write(&path, b"1") {
                    eprintln!("tour: mark {}: {e}", path.display());
                }
                eprintln!("tour: mark {name}");
                true
            }
            Step::Peer(name, max) => {
                let there = role().1.iter().all(|r| mark_path(r, name).exists());
                if !there && s.frames >= max {
                    let line = format!("step {}: gave up waiting for the peer's {name}", s.step);
                    eprintln!("tour: {line}");
                    s.log.push(line);
                }
                there || s.frames >= max
            }
            Step::Walk(k, n, name) => {
                input.keys |= k;
                if !name.is_empty() {
                    let dir = self.0.dir.join(format!("rec-{name}"));
                    let _ = std::fs::create_dir_all(&dir);
                    if let Err(e) = record_frame(emu, &dir.join(format!("{:04}.bin", s.frames))) {
                        eprintln!("tour: record {name}: {e}");
                    }
                }
                s.frames >= n
            }
            Step::MashUntilMenu(want, max) => {
                use goldlink::goldlink_bridge::battle_ui::menu;
                let b = goldlink::goldlink_bridge::battle_ui::read(emu.nds.main_ram());
                let there = b.as_ref().is_some_and(|b| b.menu == want && b.waiting);
                let now = b.as_ref().map(|b| (b.menu, b.waiting));
                if now != s.last_menu {
                    eprintln!("tour: menu {now:?} at frame {}", s.frames);
                    s.last_menu = now;
                }
                // A for the menus (FIGHT, then the first move) and the messages, but never for a
                // two-option question: those are what the route waits for.
                let question = b.as_ref().is_some_and(|b| {
                    b.waiting && (menu::YES_NO..=menu::SWITCH_OR_KEEP).contains(&b.menu)
                });
                if !there && !question && (30..36).contains(&(s.frames % 60)) {
                    input.keys |= Keys::A;
                }
                if !there && s.frames >= max {
                    let line = format!("step {}: gave up waiting for menu {want}", s.step);
                    eprintln!("tour: {line}");
                    s.log.push(line);
                }
                there || s.frames >= max
            }
            Step::Press(k) => {
                input.keys |= k;
                s.holding = 6;
                true
            }
            Step::MashUntilNaming(max, taps) => {
                let found = (s.frames % 20 == 0)
                    .then(|| find_naming(emu.nds.main_ram()))
                    .flatten();
                if let Some(d) = found {
                    let ram = emu.nds.main_ram();
                    let addr = d as u32 + 0x0200_0000;
                    let ptrs: Vec<String> = (0..ram.len() / 4)
                        .filter(|i| {
                            u32::from_le_bytes(ram[i * 4..i * 4 + 4].try_into().unwrap()) == addr
                        })
                        .map(|i| format!("{:#010x}", i * 4 + 0x0200_0000))
                        .collect();
                    let vblank = u32::from_le_bytes(ram[0x1D_110C..0x1D_1110].try_into().unwrap());
                    let line = format!(
                        "naming data {addr:#010x}, pointed to from {ptrs:?}; gSystem vblank {vblank:#010x}"
                    );
                    eprintln!("tour: {line}");
                    s.log.push(line);
                } else {
                    // A for Oak's speech; taps for the touch-only buttons (NO INFO NEEDED, the
                    // controls explanation's Touch). Both taps are blank keys on the keyboard.
                    match s.frames % 120 {
                        0..6 if taps => input.touch = Some((128, 152)),
                        30..36 | 90..96 => input.keys |= Keys::A,
                        60..66 if taps => input.touch = Some((215, 172)),
                        _ => {}
                    }
                }
                if found.is_none() && s.frames >= max {
                    let line = format!("step {}: gave up waiting for the naming screen", s.step);
                    eprintln!("tour: {line}");
                    s.log.push(line);
                }
                found.is_some() || s.frames >= max
            }
            Step::Type(text) => {
                s.typed = Some(text);
                true
            }
            Step::NamingLog => {
                let ram = emu.nds.main_ram();
                let entry = find_naming(ram).map(|d| {
                    (0..0x20)
                        .map(|i| u16::from_le_bytes([ram[d + 0xD8 + 2 * i], ram[d + 0xD9 + 2 * i]]))
                        .take_while(|&c| c != 0xFFFF)
                        .collect::<Vec<_>>()
                });
                let line = format!(
                    "naming entry {entry:?}; read {:?}",
                    goldlink::goldlink_bridge::naming::read(ram)
                );
                eprintln!("tour: {line}");
                s.log.push(line);
                true
            }
            Step::Dump(name) => {
                s.taken += 1;
                let path = self.0.dir.join(format!("{}-{name}.ram", s.taken));
                let line = match std::fs::write(&path, emu.nds.main_ram()) {
                    Ok(()) => format!("dump {}", path.display()),
                    Err(e) => format!("dump {name} failed: {e}"),
                };
                eprintln!("tour: {line}");
                s.log.push(line);
                true
            }
            Step::Running(on) => {
                emu.set_running_shoes(on);
                eprintln!("tour: running {on}: now {:?}", emu.running_shoes());
                true
            }
            Step::Poke => 'poke: {
                if let Ok(file) = std::env::var("GOLDLINK_DEV_TOUR_POKES") {
                    // Written by a script from the dump before it (the same run: addresses hold);
                    // waited for up to half a minute.
                    if !std::path::Path::new(&file).exists() && s.frames < 1800 {
                        break 'poke false;
                    }
                    let text = std::fs::read_to_string(&file).unwrap_or_default();
                    let ram = emu.nds.main_ram_mut();
                    let mut n = 0;
                    for line in text.lines() {
                        let f: Vec<u32> = line
                            .split_whitespace()
                            .filter_map(|w| u32::from_str_radix(w, 16).ok())
                            .collect();
                        let [addr, size, value] = f[..] else { continue };
                        let at = addr.wrapping_sub(0x0200_0000) as usize;
                        let bytes = value.to_le_bytes();
                        if size as usize <= 4 && at + size as usize <= ram.len() {
                            ram[at..at + size as usize].copy_from_slice(&bytes[..size as usize]);
                            n += 1;
                        }
                    }
                    eprintln!("tour: poked {n} values from {file}");
                }
                true
            }
            Step::Raw(name) => {
                s.taken += 1;
                let path = self.0.dir.join(format!("{}-{name}-raw.png", s.taken));
                let px = emu.nds.framebuffer(1);
                let img = egui::ColorImage {
                    size: [256, 192],
                    pixels: px
                        .iter()
                        .map(|&p| egui::Color32::from_rgb((p >> 16) as u8, (p >> 8) as u8, p as u8))
                        .collect(),
                };
                let line = match write_png(&path, &img) {
                    Ok(()) => format!("raw {}", path.display()),
                    Err(e) => format!("raw {name} failed: {e}"),
                };
                eprintln!("tour: {line}");
                s.log.push(line);
                true
            }
            Step::Lead(species, item, level) => {
                let root = std::path::PathBuf::from(
                    std::env::var_os("PROBE_ROOT").unwrap_or_else(|| ".".into()),
                );
                let found = (
                    engine_symbol(&root, "gCoopDebugMonBuffer"),
                    engine_constant(
                        &root,
                        "include/constants/species.h",
                        &format!("SPECIES_{species}"),
                    ),
                    engine_constant(&root, "include/constants/item.h", &format!("ITEM_{item}")),
                );
                if let (Some(buf), Some(sp), Some(it)) = found {
                    let at = (buf - 0x0200_0000) as usize;
                    let ram = emu.nds.main_ram_mut();
                    ram[at + 0xEC..at + 0xEE].copy_from_slice(&(sp as u16).to_le_bytes());
                    ram[at + 0xEE..at + 0xF0].copy_from_slice(&(it as u16).to_le_bytes());
                    ram[at + 0xF0] = level;
                    s.seq = s.seq.wrapping_add(1).max(1);
                    let at = emu.mailbox_offset();
                    let ram = emu.nds.main_ram_mut();
                    ram[at + DEBUG_RESULT] = 0;
                    ram[at + DEBUG_CMD] = C::COOP_DEBUG_GIVE_TEST_MON as u8;
                    ram[at + DEBUG_ARG] = 9;
                    ram[at + DEBUG_SEQ] = s.seq;
                    s.pending = Some((s.seq, C::COOP_DEBUG_GIVE_TEST_MON as u8, 0));
                } else {
                    let line = format!("lead {species}/{item}: not found ({found:?})");
                    eprintln!("tour: {line}");
                    s.log.push(line);
                }
                true
            }
            Step::Touch(x, y) => {
                input.touch = Some((x, y));
                s.holding = 6;
                true
            }
            Step::Debug(cmd, arg) => {
                s.seq = s.seq.wrapping_add(1).max(1);
                let ram = emu.nds.main_ram_mut();
                ram[at + DEBUG_RESULT] = 0;
                ram[at + DEBUG_CMD] = cmd;
                ram[at + DEBUG_ARG] = arg;
                ram[at + DEBUG_SEQ] = s.seq;
                s.pending = Some((s.seq, cmd, 0));
                true
            }
            Step::Shot(name) => {
                s.shot = Some(name.to_owned());
                // What the battle HUD reads at this shot, for the log.
                if let Some(b) = goldlink::goldlink_bridge::battle_ui::read(emu.nds.main_ram()) {
                    let line = format!(
                        "{name}: menu {} waiting {} cursor {:?} bag/party {}",
                        b.menu, b.waiting, b.cursor, b.bag_or_party
                    );
                    eprintln!("tour: {line}");
                    s.log.push(line);
                }
                true
            }
            Step::WarpTo(map, x, z, facing) => {
                let root =
                    PathBuf::from(std::env::var_os("PROBE_ROOT").unwrap_or_else(|| ".".into()));
                if let Some(buf) = engine_symbol(&root, "gCoopDebugMonBuffer") {
                    let at = (buf - 0x0200_0000) as usize;
                    let ram = emu.nds.main_ram_mut();
                    for (i, v) in [map, x, z, facing].iter().enumerate() {
                        ram[at + 2 * i..at + 2 * i + 2].copy_from_slice(&v.to_le_bytes());
                    }
                }
                s.seq = s.seq.wrapping_add(1).max(1);
                let m = emu.mailbox_offset();
                let ram = emu.nds.main_ram_mut();
                ram[m + DEBUG_RESULT] = 0;
                ram[m + DEBUG_CMD] = C::COOP_DEBUG_WARP as u8;
                ram[m + DEBUG_ARG] = 0xFF;
                ram[m + DEBUG_SEQ] = s.seq;
                s.pending = Some((s.seq, C::COOP_DEBUG_WARP as u8, 0));
                true
            }
            Step::MashApp(max) => {
                let away = scene != Scene::Overworld && scene != Scene::Unknown;
                if away && s.frames % 90 == 45 {
                    input.keys |= Keys::A;
                }
                (!away && s.frames > 120) || s.frames >= max
            }
            Step::Capture(max, key) => {
                if field_task(emu.nds.main_ram()) && s.frames % 40 == 20 {
                    s.taken += 1;
                    let line = capture_prompt(
                        emu,
                        &self.0.dir,
                        &format!("{}-prompt-{}", s.taken, s.frames),
                    );
                    eprintln!("tour: {line}");
                    // The window too, while a menu is open (what the player sees).
                    if !line.contains("mirror []") {
                        s.shot = Some(format!("window-{}", s.frames));
                    }
                    s.log.push(line);
                }
                if field_task(emu.nds.main_ram()) && s.frames % 40 == 30 {
                    input.keys |= key;
                }
                s.frames >= max
            }
            Step::Wide => {
                let root =
                    PathBuf::from(std::env::var_os("PROBE_ROOT").unwrap_or_else(|| ".".into()));
                let found = (
                    engine_symbol(&root, "gCoopDebugMonBuffer"),
                    engine_symbol(&root, "gFieldSysPtr"),
                );
                if let (Some(buffer), Some(fsys)) = found {
                    let busy = field_task(emu.nds.main_ram());
                    let dir = self.0.dir.clone();
                    s.wide
                        .get_or_insert_with(crate::wide::Pass::load)
                        .frame(emu, input, &dir, buffer, fsys, busy)
                } else {
                    eprintln!("tour: wide: symbols not found ({found:?})");
                    true
                }
            }
            Step::Close => false,
        };
        if next {
            s.step += 1;
            s.frames = 0;
        }
    }
}
