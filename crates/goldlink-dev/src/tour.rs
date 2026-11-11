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

fn steps() -> &'static [Step] {
    match std::env::var("GOLDLINK_DEV_TOUR_ROUTE").as_deref() {
        Ok("faint") => FAINT_STEPS,
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
                // A only in the command and fight menus (FIGHT, then the first move), so no
                // other question is answered for the player.
                let fighting = b.as_ref().is_some_and(|b| {
                    b.waiting && (menu::COMMANDS.contains(&b.menu) || b.menu == menu::FIGHT)
                });
                if !there && fighting && (30..36).contains(&(s.frames % 60)) {
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
            Step::Touch(x, y) => {
                input.touch = Some((x, y));
                s.holding = 6;
                true
            }
            Step::Debug(cmd, arg) => {
                s.seq = s.seq.wrapping_add(1).max(1);
                let ram = emu.nds.main_ram_mut();
                ram[at + DEBUG_CMD] = cmd;
                ram[at + DEBUG_ARG] = arg;
                ram[at + DEBUG_SEQ] = s.seq;
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
