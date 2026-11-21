//! Development only (`--features dev-hooks`, docs/DEBUG_TOOLS.md; never in a release:
//! scripts/package_client.sh refuses a program with these names in it). Environment variables
//! that drive the release program for tests and screenshots on a headless display, where
//! nothing can click:
//!
//! - `GOLDLINK_DEBUG_LOSE_GPU=<seconds>[,<times>]` loses the window's device that long after
//!   each window opens (`times` windows, 1 by default): first an error nothing catches (logged,
//!   not a panic), then the loss as wgpu reports it (gpu_watch.rs, KNOWN_RISKS #138).
//! - `GOLDLINK_DEV_INPUT="<seconds>:click <x> <y>;<seconds>:key <Name>;..."` clicks (in points)
//!   or presses keys (egui's names: Escape, Enter, F11) that many seconds after the start.
//! - `GOLDLINK_DEV_HUD_TEXT=<text>` replaces the battle HUD's message in two-option prompts (a
//!   long one, as a trainer's "about to send in" prompt has, from a wild battle's).
//! - `GOLDLINK_DEV_BACKUP=<seconds>` asks the bridge for an emergency backup that long after the
//!   start (taken at the game's next safe moment).

use std::sync::Mutex;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};

use eframe::egui;
use eframe::egui_wgpu;

static OPENED: Mutex<Option<Instant>> = Mutex::new(None);
static LOST_SO_FAR: AtomicU32 = AtomicU32::new(0);

pub(crate) fn window_watched() {
    *OPENED.lock().unwrap() = Some(Instant::now());
}

pub fn lose_gpu_tick(rs: &egui_wgpu::RenderState) {
    let Some(spec) = std::env::var("GOLDLINK_DEBUG_LOSE_GPU").ok() else {
        return;
    };
    let mut parts = spec.split(',');
    let secs: f32 = parts.next().and_then(|s| s.parse().ok()).unwrap_or(10.0);
    let times: u32 = parts.next().and_then(|s| s.parse().ok()).unwrap_or(1);
    if LOST_SO_FAR.load(Ordering::Relaxed) >= times {
        return;
    }
    let due = OPENED
        .lock()
        .unwrap()
        .is_some_and(|t| t.elapsed() >= Duration::from_secs_f32(secs));
    if !due {
        return;
    }
    *OPENED.lock().unwrap() = None;
    LOST_SO_FAR.fetch_add(1, Ordering::Relaxed);
    log::warn!("debug: losing the window's GPU device");
    // A validation error with no scope: the handler's, not a panic.
    let _ = rs.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("debug: invalid"),
        size: 4,
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::STORAGE,
        mapped_at_creation: false,
    });
    crate::gpu_watch::LOST.store(true, Ordering::Relaxed);
}

enum Step {
    Click(egui::Pos2),
    Key(egui::Key),
}

struct Input {
    start: Instant,
    steps: Vec<(f32, Step, u8)>,
}

static INPUT: Mutex<Option<Input>> = Mutex::new(None);

fn parse(spec: &str) -> Vec<(f32, Step, u8)> {
    spec.split(';')
        .filter_map(|item| {
            let (t, what) = item.trim().split_once(':')?;
            let t: f32 = t.trim().parse().ok()?;
            let mut w = what.split_whitespace();
            let step = match w.next()? {
                "click" => {
                    Step::Click(egui::pos2(w.next()?.parse().ok()?, w.next()?.parse().ok()?))
                }
                "key" => Step::Key(egui::Key::from_name(w.next()?)?),
                _ => return None,
            };
            Some((t, step, 0))
        })
        .collect()
}

/// Adds the scripted input that's due to this frame's (a press one frame, its release the next).
pub fn input(raw: &mut egui::RawInput) {
    let mut g = INPUT.lock().unwrap();
    let inp = g.get_or_insert_with(|| Input {
        start: Instant::now(),
        steps: std::env::var("GOLDLINK_DEV_INPUT")
            .map(|s| parse(&s))
            .unwrap_or_default(),
    });
    let now = inp.start.elapsed().as_secs_f32();
    for (t, step, state) in inp.steps.iter_mut() {
        if *t > now || *state == 2 {
            continue;
        }
        let pressed = *state == 0;
        match step {
            Step::Click(pos) => {
                raw.events.push(egui::Event::PointerMoved(*pos));
                raw.events.push(egui::Event::PointerButton {
                    pos: *pos,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: Default::default(),
                });
            }
            Step::Key(key) => raw.events.push(egui::Event::Key {
                key: *key,
                physical_key: None,
                pressed,
                repeat: false,
                modifiers: Default::default(),
            }),
        }
        *state += 1;
        log::info!("dev input at {now:.1} s");
        break; // one step a frame
    }
}

/// Keeps frames coming while scripted input is still to come (the launcher only draws on events).
pub fn pending(ctx: &egui::Context) {
    if INPUT
        .lock()
        .unwrap()
        .as_ref()
        .is_some_and(|i| i.steps.iter().any(|s| s.2 < 2))
    {
        ctx.request_repaint_after(Duration::from_millis(50));
    }
}

/// `GOLDLINK_DEV_BACKUP`: one backup asked for, once due.
pub fn backup_tick(bridge: Option<&goldlink_bridge::SharedBridge>) {
    static START: Mutex<Option<Instant>> = Mutex::new(None);
    static DONE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    let Some(b) = bridge else { return };
    let Some(secs) = std::env::var("GOLDLINK_DEV_BACKUP")
        .ok()
        .and_then(|s| s.parse::<f32>().ok())
    else {
        return;
    };
    let start = *START.lock().unwrap().get_or_insert_with(Instant::now);
    if start.elapsed().as_secs_f32() >= secs && !DONE.swap(true, Ordering::Relaxed) {
        log::info!("dev: a backup asked for");
        goldlink_bridge::hook::lock(b).backup_soon();
    }
}

/// `GOLDLINK_DEV_HUD_TEXT`: the HUD with that message in its two-option prompts.
/// `GOLDLINK_DEV_HUD_EFFECTS`: the four moves' matchup marks, one letter each (S super, V not
/// very, X none, - plain), to see them all whatever the foe.
pub fn hud_text(
    hud: &goldlink_bridge::battle_ui::BattleHud,
) -> goldlink_bridge::battle_ui::BattleHud {
    use goldlink_bridge::types::Effect;
    let mut h = hud.clone();
    if let (Ok(t), Some(_)) = (std::env::var("GOLDLINK_DEV_HUD_TEXT"), &h.options) {
        h.message = t;
    }
    if let Ok(e) = std::env::var("GOLDLINK_DEV_HUD_EFFECTS") {
        for (m, c) in h.moves.iter_mut().zip(e.chars()) {
            if let Some(m) = m {
                m.effect = match c {
                    'S' => Some(Effect::Super),
                    'V' => Some(Effect::NotVery),
                    'X' => Some(Effect::None),
                    _ => None,
                };
            }
        }
    }
    h
}
