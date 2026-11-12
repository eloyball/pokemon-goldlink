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
    Close,
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

fn steps() -> &'static [Step] {
    match std::env::var("GOLDLINK_DEV_TOUR_ROUTE").as_deref() {
        Ok("faint") => FAINT_STEPS,
        Ok("intro") => intro_steps(),
        Ok("learn") => LEARN_STEPS,
        Ok("mega") => MEGA_STEPS,
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
    last_menu: Option<(i8, bool)>,
    /// A debug command waiting for the ROM's answer: its sequence number, command, and frames.
    pending: Option<(u8, u8, u32)>,
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

    pub fn hook(&self) -> TourHook {
        TourHook(self.clone())
    }

    /// The window's part: screenshots when the game side asks, and closing at the end.
    pub fn ui(&self, ctx: &egui::Context) {
        let mut s = self.state();
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
        if let Some(name) = s.shot.clone() {
            if !s.requested {
                ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::default()));
                s.requested = true;
            }
            let image = ctx.input(|i| {
                i.raw.events.iter().find_map(|e| match e {
                    egui::Event::Screenshot { image, .. } => Some(image.clone()),
                    _ => None,
                })
            });
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
        let mut s = self.0.state();
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
            Step::Close => false,
        };
        if next {
            s.step += 1;
            s.frames = 0;
        }
    }
}
