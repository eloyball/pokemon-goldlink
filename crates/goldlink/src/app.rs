//! The game window (RFC §2.2, §4.4, UC2): both screens, the stylus, keyboard and gamepads,
//! sound, and a settings panel on Esc. No emulator chrome: no menus, rewind or savestate keys.

use std::collections::VecDeque;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use eframe::egui;
use goldlink_runtime::audio::AudioOut;
use goldlink_runtime::config::{Config, Filter, Layout};
use goldlink_runtime::runner::lock;
use goldlink_runtime::{Command, Event, Input, Runner};

use crate::hud;
use crate::input::{Controls, touch_at};
use crate::layout;
use crate::theme::{self, Kind};
use crate::world_ui::{self, WorldPanel};
use goldlink_bridge::{Scene, SharedBridge};

/// The release program's bar above the screens (UC8 §2.1): the world, who's online, Leave.
pub const BAR: f32 = 38.0;

/// The halves of the screens texture: the top screen, the touch screen.
const TOP_UV: egui::Rect = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 0.5));
const BOTTOM_UV: egui::Rect = egui::Rect::from_min_max(egui::pos2(0.0, 0.5), egui::pos2(1.0, 1.0));

/// The two screens' parts of the screens texture, uploaded `k` times enlarged (`sharp_prescale`):
/// with smoothed sampling (k > 1) each stops half a texel short of the line between them, so
/// neither screen's edge blends with the other's.
fn screen_uvs(k: usize) -> (egui::Rect, egui::Rect) {
    if k <= 1 {
        return (TOP_UV, BOTTOM_UV);
    }
    let half = 0.5 / (384 * k) as f32;
    (
        egui::Rect::from_min_max(TOP_UV.min, egui::pos2(1.0, 0.5 - half)),
        egui::Rect::from_min_max(egui::pos2(0.0, 0.5 + half), BOTTOM_UV.max),
    )
}
/// How long the HUD holds a touch on the game's button (then lets go).
const TAP: Duration = Duration::from_millis(100);

/// The naming screen's hint: the computer's keyboard types the name.
fn typing_hint(p: &egui::Painter, room: egui::Rect, align: egui::Align2) {
    let font = egui::FontId::proportional((room.height() * 0.45).clamp(11.0, 16.0));
    // One line when it fits, else one line each (no separator left hanging at a line's end).
    let line = "Type on your keyboard   ·   Enter: OK   ·   Backspace: delete";
    let mut galley = p.layout_no_wrap(line.to_owned(), font.clone(), theme::TEXT);
    if galley.size().x > room.width() {
        let lines = "Type on your keyboard\nEnter: OK\nBackspace: delete";
        galley = p.layout(lines.to_owned(), font, theme::TEXT, room.width());
    }
    let at = align.align_size_within_rect(galley.size(), room).min;
    p.rect_filled(
        egui::Rect::from_min_size(at, galley.size()).expand2(egui::vec2(10.0, 4.0)),
        4.0,
        egui::Color32::from_black_alpha(170),
    );
    p.galley(at, galley, theme::TEXT);
}

/// What the release program's game window asks of the shell.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GameAction {
    /// Leave the world: the game saves, then back to the launcher.
    Leave,
}

/// The one-screen layout's state (UC9 §3).
#[derive(Debug, Default)]
struct OneScreen {
    /// Tab: the touch screen shown (or hidden) by hand, until the scene changes.
    by_hand: Option<bool>,
    /// The scene the last frame showed.
    scene: Option<Scene>,
    /// A touch the HUD is making for the player: the DS point and when it began.
    tap: Option<((u16, u16), Instant)>,
}

impl OneScreen {
    fn follow(&mut self, scene: Scene) {
        if scene != Scene::Changing && self.scene != Some(scene) {
            self.scene = Some(scene);
            self.by_hand = None;
        }
    }
}

struct Message {
    text: String,
    error: bool,
    until: Instant,
}

pub struct GameApp {
    runner: Option<Runner>,
    bridge: Option<SharedBridge>,
    world: Option<WorldPanel>,
    config: Config,
    config_dir: PathBuf,
    controls: Controls,
    audio: Option<AudioOut>,
    gilrs: Option<gilrs::Gilrs>,
    texture: Option<egui::TextureHandle>,
    last_seq: u64,
    bottom: egui::Rect,
    one: OneScreen,
    /// Typing a name on the keyboard while the game's naming screen is open.
    typing: crate::typing::Typing,
    /// The game's text box, re-inked for the battle HUD (one-screen layout).
    textbox: Option<egui::TextureHandle>,
    /// The band belongs over the game's text box rows (hud::game_textbox_shown).
    game_textbox: bool,
    /// Frames in a row the game's text box rows have shown something else (the band steps aside
    /// only after a while: an attack's white flash is a frame or two, the VS screen stays).
    textbox_away: u32,
    /// The wider top screen (UC9 §4) and its side, when the emulation thread makes it.
    wide: Option<egui::TextureHandle>,
    wide_side: usize,
    /// Physical pixels per DS pixel the top screen was last drawn at (the sharp filter's
    /// pre-enlargement follows it).
    shown_scale: f32,
    /// The screens texture's enlargement (`sharp_prescale`).
    prescale: usize,
    /// The colour the sides take between two scenes, per screen (top, touch screen) and side:
    /// the screen's edges' when they're one flat colour (black for a fade, white for a cave's
    /// exit flash, a bar across), otherwise black.
    edge_fill: [[Option<Vec<egui::Color32>>; 2]; 2],
    /// The steady overworld's shift for the top picture on screen (UC11): slid back by it.
    steady_shift: Option<[f32; 2]>,
    /// And where the world's origin truly is in it (DS screen columns and rows): see `pixel_lock`.
    steady_origin: [f64; 2],
    /// With it, over the slid picture (split_layer): what the camera follows (the player, their
    /// shadow), slid back by its own shift, and the 2D drawn over the scene (a text box, a
    /// banner), where it is.
    attached: Layer,
    /// What's in front of the player (a windmill's pole, a roof's corner, a person), drawn over
    /// the player's layer again with the world's shift.
    front: Layer,
    fixed: Layer,
    /// hg-engine's MEGA button, as the touch screen shows it (None: not offered; else lit), and
    /// its Mega symbol cut from there.
    mega: Option<bool>,
    mega_icon: Option<egui::TextureHandle>,
    /// The lighting's buffers and worker threads (UC11), from the first lit frame.
    lighting: Option<goldlink_light::Lighting>,

    settings_open: bool,
    messages: VecDeque<Message>,
    /// Drawn after everything else (dev panels).
    pub extra_ui: Option<crate::ExtraUi>,
    /// Merged into each frame's input first (the dev tour's typing).
    pub extra_input: Option<crate::ExtraInput>,
    /// The release program's look (UC8): the top bar, the Esc menu and the Details panel instead
    /// of the in-window World panel (goldlink-dev keeps that).
    release_ui: bool,
    menu_open: bool,
    details_open: bool,
    /// The world being played (the top bar's name).
    world_code: String,
    /// Leaving: what the top bar says until the game has saved.
    pub leaving: Option<String>,
}

impl GameApp {
    pub fn new(
        runner: Runner,
        bridge: Option<SharedBridge>,
        config: Config,
        config_dir: PathBuf,
        notes: Vec<String>,
    ) -> Self {
        let mut messages = VecDeque::new();
        let mut note = |text: String, error: bool| {
            messages.push_back(Message {
                text,
                error,
                until: Instant::now() + Duration::from_secs(10),
            })
        };
        for n in notes {
            note(n, true);
        }
        let audio = match AudioOut::open(runner.shared.clone(), Self::volume_of(&config)) {
            Ok(a) => {
                runner.send(Command::AudioRate(a.rate));
                Some(a)
            }
            Err(e) => {
                note(format!("No sound: {e}"), true);
                None
            }
        };
        let gilrs = gilrs::Gilrs::new().ok();
        let app = Self {
            controls: Controls::new(&config.keys, &config.pad),
            runner: Some(runner),
            world: bridge.as_ref().map(WorldPanel::new),
            bridge,
            config,
            config_dir,
            audio,
            gilrs,
            texture: None,
            last_seq: u64::MAX,
            bottom: egui::Rect::NOTHING,
            one: OneScreen::default(),
            typing: Default::default(),
            textbox: None,
            game_textbox: true,
            textbox_away: 0,
            wide: None,
            wide_side: 0,
            shown_scale: 1.0,
            prescale: 1,
            steady_shift: None,
            steady_origin: [0.0; 2],
            attached: Layer::default(),
            front: Layer::default(),
            fixed: Layer::default(),
            edge_fill: [[None, None], [None, None]],
            mega: None,
            mega_icon: None,
            lighting: None,

            settings_open: false,
            messages,
            extra_ui: None,
            extra_input: None,
            release_ui: false,
            menu_open: false,
            details_open: false,
            world_code: String::new(),
            leaving: None,
        };
        app.apply_wide();
        app
    }

    /// The wider picture on the emulation thread: on in the one-screen layout when chosen.
    fn apply_wide(&self) {
        let v = &self.config.video;
        let side = if v.layout == Layout::OneScreen && v.wide {
            layout::WIDE_SIDE
        } else {
            0
        };
        if let Some(r) = &self.runner {
            r.send(Command::Wide(side));
            // The lighting's buffers come with each frame only while it's on.
            r.send(Command::Lighting(v.lighting));
            r.send(Command::Steady(v.steady));
        }
    }

    /// The player's settings, as changed in the game.
    pub fn config(&self) -> &Config {
        &self.config
    }

    /// The release program's game window (UC8): no World panel; the top bar, Esc's menu and
    /// the Details panel (F3).
    pub fn release(
        runner: Runner,
        bridge: SharedBridge,
        config: Config,
        config_dir: PathBuf,
        notes: Vec<String>,
        world_code: &str,
    ) -> Self {
        let mut app = Self::new(runner, Some(bridge), config, config_dir, notes);
        app.say(
            if app.config.video.layout == Layout::OneScreen {
                "Esc: menu · F3: details · Tab: the touch screen"
            } else {
                "Esc: menu · F3: details"
            },
            false,
        );
        app.world = None;
        app.release_ui = true;
        app.world_code = world_code.to_owned();
        app
    }

    /// The window size that shows the screens at `scale` (with the release bar).
    pub fn window_size(&self, scale: f32) -> egui::Vec2 {
        let bar = if self.release_ui { BAR } else { 0.0 };
        layout::window_size(&self.config.video, scale) + egui::vec2(0.0, bar)
    }

    /// A panel over the game takes the keyboard and the stylus.
    fn blocked(&self) -> bool {
        self.settings_open || self.menu_open
    }

    fn volume_of(c: &Config) -> u8 {
        if c.audio.muted { 0 } else { c.audio.volume }
    }

    fn say(&mut self, text: impl Into<String>, error: bool) {
        self.messages.push_back(Message {
            text: text.into(),
            error,
            until: Instant::now() + Duration::from_secs(if error { 10 } else { 3 }),
        });
        while self.messages.len() > 4 {
            self.messages.pop_front();
        }
    }

    fn save_config(&mut self) {
        if let Err(e) = self.config.save(&self.config_dir) {
            self.say(format!("Settings not saved: {e}"), true);
        }
        if let Some(a) = &self.audio {
            a.set_volume(Self::volume_of(&self.config));
        }
        self.apply_wide();
    }

    /// Reads the keyboard, gamepads and pointer into the next frame's input.
    fn feed_input(&mut self, ctx: &egui::Context) {
        let Some(runner) = &self.runner else { return };
        // Typing in a panel or clicking on one isn't playing.
        let typing = ctx.wants_keyboard_input();
        let over_panel = ctx.is_pointer_over_area() && ctx.wants_pointer_input();
        let (held, down, pos, focused) = ctx.input(|i| {
            (
                i.keys_down.clone(),
                i.pointer.primary_down(),
                i.pointer.interact_pos(),
                i.focused,
            )
        });
        let mut keys = goldlink_core::Keys::NONE;
        if focused && !self.blocked() && !typing {
            if self.typing.naming.is_some() {
                // Naming: the letters type (crate::typing); the arrows still move the cursor.
                let arrows = [
                    egui::Key::ArrowUp,
                    egui::Key::ArrowDown,
                    egui::Key::ArrowLeft,
                    egui::Key::ArrowRight,
                ];
                let held = held.into_iter().filter(|k| arrows.contains(k)).collect();
                keys |= self.controls.keyboard(&held);
            } else {
                keys |= self.controls.keyboard(&held);
            }
        }
        let blocked = self.blocked();
        if let Some(g) = &mut self.gilrs {
            while g.next_event().is_some() {}
            if !blocked {
                keys |= self.controls.gamepads(g);
            }
        }
        let mut touch = match (down, pos) {
            (true, Some(p)) if !self.blocked() && !over_panel => touch_at(self.bottom, p),
            _ => None,
        };
        // A typed character's key on the naming screen.
        if let Some(at) = self.typing.touch() {
            touch = Some(at);
        }
        // The battle HUD's click: the game's button, held a moment, then let go.
        if let Some((at, since)) = self.one.tap {
            if since.elapsed() < TAP {
                touch = Some(at);
            } else {
                self.one.tap = None;
            }
        }
        *lock(&runner.shared.input) = Input { keys, touch };
    }

    fn update_texture(&mut self, ctx: &egui::Context) {
        let Some(runner) = &self.runner else { return };
        let lit = self.config.video.lighting;
        let shown = {
            let frame = lock(&runner.shared.frame);
            if frame.seq == self.last_seq {
                None
            } else {
                self.last_seq = frame.seq;
                // What the HUD reads, from the game's own pixels.
                if self.config.video.layout == Layout::OneScreen {
                    self.mega = hud::mega_state(&frame.pixels[256 * 192..]);
                    if self.mega.is_some() {
                        let img = hud::mega_icon(&frame.pixels[256 * 192..]);
                        match &mut self.mega_icon {
                            Some(t) => t.set(img, egui::TextureOptions::NEAREST),
                            None => {
                                self.mega_icon = Some(ctx.load_texture(
                                    "mega-icon",
                                    img,
                                    egui::TextureOptions::NEAREST,
                                ))
                            }
                        }
                    }
                    let rows = &frame.pixels[256 * hud::SCENE_ROWS as usize..256 * 192];
                    if hud::game_textbox_shown(rows) {
                        self.textbox_away = 0;
                    } else {
                        self.textbox_away = self.textbox_away.saturating_add(1);
                    }
                    self.game_textbox = self.textbox_away < 20;
                    let img = hud::recolour_textbox(rows);
                    match &mut self.textbox {
                        Some(t) => t.set(img, egui::TextureOptions::NEAREST),
                        None => {
                            self.textbox = Some(ctx.load_texture(
                                "textbox",
                                img,
                                egui::TextureOptions::NEAREST,
                            ))
                        }
                    }
                }
                // The wider top screen, when the emulation thread makes it (one-screen layout).
                let wide = (self.config.video.layout == Layout::OneScreen
                    && frame.wide_side > 0
                    && !frame.wide.is_empty())
                .then(|| frame.wide.clone());
                self.wide_side = frame.wide_side;
                self.steady_shift = frame.steady_shift;
                self.steady_origin = frame.steady_origin;
                self.edge_fill = [
                    edge_rows(&frame.pixels[..256 * 192]),
                    edge_rows(&frame.pixels[256 * 192..256 * 384]),
                ];
                let light = if lit { frame.light.clone() } else { None };
                let layers = (frame.fixed.clone(), frame.attached.clone());
                Some((frame.pixels.clone(), wide, frame.wide_side, light, layers))
            }
        };
        let (k, options) = sharp_prescale(self.shown_scale, self.config.video.filter);
        let Some((mut pixels, mut wide, side, light, (fixed, attached))) = shown else {
            if self.texture.is_none() {
                let img = egui::ColorImage::new([256, 384], egui::Color32::BLACK);
                self.texture = Some(ctx.load_texture("screens", img, options));
            }
            return;
        };
        // The top picture before the lighting, with an attached layer: the world under the player
        // is lit as the player's own pixels are (split_layer).
        let unlit: Option<Vec<u32>> = attached
            .as_ref()
            .filter(|_| lit && self.steady_shift.is_some())
            .map(|_| match &wide {
                Some(w) => w.clone(),
                None => pixels[..256 * 192].to_vec(),
            });
        if lit {
            self.light(&mut pixels, wide.as_deref_mut(), side, light.as_ref());
        }
        let image = |px: &[u32], w: usize, h: usize| enlarged(px, w, h, k);
        // The steady overworld's layers (UC11), taken out of the top picture: what the camera
        // follows, slid back by its own shift, and the 2D over the scene, not slid.
        self.attached.shown = false;
        self.front.shown = false;
        self.fixed.shown = false;
        if self.steady_shift.is_some() && (!fixed.is_empty() || attached.is_some()) {
            let full = 256 + 2 * side;
            let (top, w): (&mut [u32], usize) = match wide.as_deref_mut() {
                Some(wide) => (wide, full),
                None => (&mut pixels[..256 * 192], 256),
            };
            let fixed_mask = fit(&fixed, w);
            if let Some(a) = &attached {
                let (mask, mut trans) = (fit(&a.mask, w), fit(&a.trans, w));
                let gain = light_gain(top, unlit.as_deref(), &mask, w);
                // The player's pixels behind the world (3), lit as their own are.
                let own = player_gain(top, unlit.as_deref(), &mask);
                for i in (0..mask.len()).filter(|&i| mask[i] == 3) {
                    trans[i] = scaled(trans[i], own);
                }
                // The world's edges over the player (an outline: the renderer kept them apart),
                // blended over whatever they cover on screen; the picture there is the world under
                // the player.
                let half = a.edge.len() / 2;
                let (edge, edge_under) = (fit(&a.edge[..half], w), fit(&a.edge[half..], w));
                let edge = lit_edges(top, &mask, &edge, &edge_under);
                if mask.iter().any(|&m| m >= 3) {
                    let img = front_layer(top, &mask, &edge, w, k);
                    let world = self.steady_shift;
                    self.front.set(ctx, "front", img, options, w != 256, world);
                    self.front.origin = self.steady_origin;
                }
                let img = split_layer(top, &mask, &fixed_mask, w, k, Some(&trans), Some(gain));
                self.attached
                    .set(ctx, "attached", img, options, w != 256, Some(a.shift));
                self.attached.origin = a.origin;
            }
            if !fixed.is_empty() {
                let none = vec![0; w * 192];
                let other = attached.as_ref().map_or(none, |a| fit(&a.mask, w));
                let img = split_layer(top, &fixed_mask, &other, w, k, None, None);
                self.fixed.set(ctx, "fixed", img, options, w != 256, None);
            }
        }
        if let Some(wide) = &wide {
            let w = 256 + 2 * side;
            let img = image(wide, w, 192);
            match &mut self.wide {
                Some(t) if t.size() == [w * k, 192 * k] => t.set(img, options),
                _ => self.wide = Some(ctx.load_texture("wide", img, options)),
            }
        }
        let img = image(&pixels, 256, 384);
        self.prescale = k;
        match &mut self.texture {
            Some(t) => t.set(img, options),
            None => self.texture = Some(ctx.load_texture("screens", img, options)),
        }
    }

    /// The lighting (UC11): the overworld lit on the top screen (the wider picture when it's
    /// shown, else the screen), everything else (battles, menus, the touch screen) graded.
    fn light(
        &mut self,
        pixels: &mut [u32],
        wide: Option<&mut [u32]>,
        side: usize,
        light: Option<&goldlink_runtime::LightFrame>,
    ) {
        use goldlink_light::{Phase, Place, Scene};
        let lighting = self
            .lighting
            .get_or_insert_with(goldlink_light::Lighting::new);
        let hour = light.map_or_else(|| chrono::Timelike::hour(&chrono::Local::now()), |l| l.hour);
        let phase = Phase::of_hour(hour);
        let world = light.filter(|l| l.overworld());
        let scene = world.map(|l| {
            (
                Scene {
                    depth: &l.buffers.depth,
                    layer: &l.buffers.layer,
                    proj: l.buffers.proj,
                    width: goldlink_core::LightBuffers::WIDTH,
                    fade: &l.buffers.fade,
                },
                Place::of_map(l.map),
            )
        });
        let middle = goldlink_core::Nds::MAX_WIDE_SIDE;
        // The game's fades on the top screen, when the 3D scene is there (a battle's flash).
        let fade: &[goldlink_light::Fade] = light.map_or(&[], |l| &l.buffers.fade);
        let (top, bottom) = pixels.split_at_mut(256 * 192);
        match wide {
            Some(wide) => {
                let w = 256 + 2 * side;
                match &scene {
                    Some((s, place)) => {
                        lighting.overworld(wide, w, middle - side, s, *place, phase)
                    }
                    None => lighting.grade_faded(wide, w, phase, fade),
                }
                // The DS screen alone shows where the wider picture doesn't (menus, the title).
                lighting.grade_faded(top, 256, phase, fade);
            }
            None => match &scene {
                Some((s, place)) => lighting.overworld(top, 256, middle, s, *place, phase),
                None => lighting.grade_faded(top, 256, phase, fade),
            },
        }
        lighting.grade(bottom, 256, 192, phase);
    }

    fn collect_events(&mut self) {
        let Some(runner) = &self.runner else { return };
        let events: Vec<Event> = std::mem::take(&mut *lock(&runner.shared.events));
        let finished = runner.finished();
        for e in events {
            match e {
                Event::Saved | Event::BackupTaken(_) => {}
                Event::SaveFailed(e) => {
                    self.say(format!("Your save couldn't be written: {e}"), true)
                }
                Event::BackupFailed(e) => {
                    self.say(format!("A backup couldn't be written: {e}"), true)
                }
            }
        }
        if finished {
            self.runner = None;
            self.say(
                "The game stopped unexpectedly. Your last in-game save is kept.",
                true,
            );
        }
    }

    fn settings(&mut self, ctx: &egui::Context) {
        let mut open = self.settings_open;
        let mut changed = false;
        let mut quit = false;
        // Never anchored: it can be dragged off the game (UC8).
        egui::Window::new("Settings")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .default_pos(egui::pos2(24.0, BAR + 24.0))
            .show(ctx, |ui| {
                let v = &mut self.config.video;
                ui.horizontal(|ui| {
                    ui.label("Screens:");
                    changed |= ui.radio_value(&mut v.layout, Layout::Vertical, "Top and bottom").changed();
                    changed |= ui.radio_value(&mut v.layout, Layout::Horizontal, "Side by side").changed();
                    changed |= ui.radio_value(&mut v.layout, Layout::OneScreen, "One screen")
                        .on_hover_text("The other screen shows beside the game when it's needed (Tab: show or hide it)")
                        .changed();
                });
                changed |= ui.add(egui::Slider::new(&mut v.gap, 0..=96).text("Gap between the screens")).changed();
                ui.horizontal(|ui| {
                    ui.label("Pixels:");
                    changed |= ui.radio_value(&mut v.filter, Filter::Nearest, "Sharp").changed();
                    changed |= ui.radio_value(&mut v.filter, Filter::Linear, "Smooth").changed();
                });
                changed |= ui.checkbox(&mut v.integer_scale, "Whole-number scaling only").changed();
                changed |= ui
                    .checkbox(&mut v.lighting, "Lighting")
                    .on_hover_text("Light and shade in the overworld, a soft colour grade elsewhere")
                    .changed();
                changed |= ui
                    .checkbox(&mut v.steady, "Steady overworld")
                    .on_hover_text("A flat camera that keeps the town's pixels still as you move; off: the game's own perspective, which shimmers")
                    .changed();
                if self.release_ui && ui.checkbox(&mut v.fullscreen, "Fullscreen (F11)").changed() {
                    changed = true;
                    ui.ctx().send_viewport_cmd(egui::ViewportCommand::Fullscreen(v.fullscreen));
                }
                if v.layout == Layout::OneScreen {
                    changed |= ui.checkbox(&mut v.wide, "Widescreen in the overworld and in battles").changed();
                }
                ui.separator();
                let a = &mut self.config.audio;
                changed |= ui.add(egui::Slider::new(&mut a.volume, 0..=100).text("Volume")).changed();
                changed |= ui.checkbox(&mut a.muted, "Mute").changed();
                ui.separator();
                ui.label("Controls: A = X or K, B = Z or J, X = V, Y = C, L = Q, R = E, Start = Enter, Select = Space, arrows or WASD, the mouse on the bottom screen.");
                ui.small(format!("Change them in {}", self.config_dir.join(goldlink_runtime::config::FILE).display()));
                ui.separator();
                ui.horizontal(|ui| {
                    if ui.button("Back to the game").clicked() {
                        self.settings_open = false;
                    }
                    if let Some(w) = self.world.as_mut()
                        && ui.button("World…").clicked()
                    {
                        w.open = true;
                        self.settings_open = false;
                    }
                    if !self.release_ui && ui.button("Quit").clicked() {
                        quit = true;
                    }
                });
            });
        self.settings_open &= open;
        if changed {
            self.save_config();
        }
        if quit {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
    }

    fn stop(&mut self) {
        // As the C# bridge on close: a one-shot LEAVE frees the slot at once.
        if let Some(b) = &self.bridge {
            let mut b = goldlink_bridge::hook::lock(b);
            if b.link() != goldlink_bridge::LinkState::Offline {
                b.leave();
            }
        }
        if let Some(r) = self.runner.take() {
            r.stop();
        }
    }
}

impl GameApp {
    /// The bar above the screens: the world on the left; who's online and Leave on the right.
    fn top_bar(&mut self, ctx: &egui::Context) -> Option<GameAction> {
        let view = self
            .bridge
            .as_ref()
            .map(|b| goldlink_bridge::hook::lock(b).view());
        let mut action = None;
        egui::TopBottomPanel::top("goldlink-bar")
            .exact_height(BAR)
            .frame(
                egui::Frame::new()
                    .fill(theme::INSET)
                    .inner_margin(egui::Margin::symmetric(12, 0)),
            )
            .show(ctx, |ui| {
                let r = ui.max_rect();
                ui.painter().line_segment(
                    [
                        r.left_bottom() + egui::vec2(-12.0, -1.0),
                        r.right_bottom() + egui::vec2(12.0, -1.0),
                    ],
                    egui::Stroke::new(2.0_f32, theme::EDGE),
                );
                ui.horizontal_centered(|ui| {
                    ui.label(
                        egui::RichText::new("GOLDLINK")
                            .font(egui::FontId::new(15.0, theme::caps_bold()))
                            .color(theme::GOLD),
                    );
                    ui.label(
                        egui::RichText::new(format!("WORLD {}", self.world_code))
                            .font(theme::label(13.0))
                            .color(theme::TEXT_DIM),
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let leave = theme::button(
                            ui,
                            "Leave",
                            Kind::Danger,
                            egui::vec2(78.0, 26.0),
                            self.leaving.is_none(),
                        );
                        if leave
                            .on_hover_text("Saves your game, then back to the launcher")
                            .clicked()
                        {
                            action = Some(GameAction::Leave);
                        }
                        ui.add_space(12.0);
                        if let Some(text) = &self.leaving {
                            ui.label(
                                egui::RichText::new(format!("{text}{}", theme::dots(ui)))
                                    .font(theme::text(16.0))
                                    .color(theme::AMBER),
                            );
                            return;
                        }
                        let (online, expected, connected) = match &view {
                            Some(v) if v.link == goldlink_bridge::LinkState::Connected => {
                                v.mailbox.as_ref().map_or((0, 0, true), |mb| {
                                    let g = goldlink_bridge::layout::GATES;
                                    (
                                        mb[g + goldlink_bridge::layout::GATE_ONLINE_PARTY],
                                        mb[g + goldlink_bridge::layout::GATE_EXPECTED_PARTY],
                                        true,
                                    )
                                })
                            }
                            _ => (0, 0, false),
                        };
                        let text = if !connected {
                            "Offline".to_owned()
                        } else if expected == 0 {
                            "…".to_owned()
                        } else {
                            format!("{online}/{expected}")
                        };
                        ui.label(
                            egui::RichText::new(text)
                                .font(theme::text(20.0))
                                .color(theme::TEXT),
                        )
                        .on_hover_text("Players online in this world");
                        let (dot, _) =
                            ui.allocate_exact_size(egui::vec2(12.0, 12.0), egui::Sense::hover());
                        theme::dot(
                            ui.painter(),
                            dot.center(),
                            if connected {
                                theme::GREEN_DOT
                            } else {
                                theme::TEXT_FAINT
                            },
                        );
                    });
                });
            });
        action
    }

    /// Esc's menu: back to the game, settings, details, the gym gate toggle, Leave.
    fn menu(&mut self, ctx: &egui::Context) -> Option<GameAction> {
        let mut action = None;
        let mut open = true;
        let bridge = self.bridge.clone();
        egui::Window::new("Menu")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
            .show(ctx, |ui| {
                let size = egui::vec2(280.0, 38.0);
                if theme::button(ui, "Back to the game", Kind::Primary, size, true).clicked() {
                    self.menu_open = false;
                }
                if theme::button(ui, "Settings", Kind::Neutral, size, true).clicked() {
                    self.settings_open = true;
                    self.menu_open = false;
                }
                let details = if self.details_open {
                    "Hide details (F3)"
                } else {
                    "Details (F3)"
                };
                if theme::button(ui, details, Kind::Neutral, size, true).clicked() {
                    self.details_open = !self.details_open;
                    self.menu_open = false;
                }
                if let Some(b) = &bridge {
                    let mut ahead = goldlink_bridge::hook::lock(b).allow_ahead;
                    if ui
                        .checkbox(
                            &mut ahead,
                            "Let teammates challenge bosses\nwhile I'm behind",
                        )
                        .changed()
                    {
                        goldlink_bridge::hook::lock(b).allow_ahead = ahead;
                    }
                }
                if theme::button(
                    ui,
                    "Leave world",
                    Kind::Danger,
                    size,
                    self.leaving.is_none(),
                )
                .on_hover_text("Saves your game, then back to the launcher")
                .clicked()
                {
                    action = Some(GameAction::Leave);
                    self.menu_open = false;
                }
            });
        if !open {
            self.menu_open = false;
        }
        action
    }

    /// The technical lines (F3), in a panel that can be dragged anywhere (UC8 §2.1).
    fn details(&mut self, ctx: &egui::Context) {
        let Some(b) = &self.bridge else { return };
        let (view, quic) = {
            let b = goldlink_bridge::hook::lock(b);
            (b.view(), b.quic_stats())
        };
        let mut open = self.details_open;
        egui::Window::new("Details")
            .open(&mut open)
            .collapsible(true)
            .resizable(false)
            .default_pos(egui::pos2(16.0, BAR + 16.0))
            .default_width(340.0)
            .show(ctx, |ui| {
                ui.style_mut().override_font_id = Some(theme::text(15.0));
                world_ui::details(ui, &view);
                ui.separator();
                ui.label(match &quic {
                    Some(_) => "Connection: encrypted (QUIC)",
                    None if view.link == goldlink_bridge::LinkState::Connected => {
                        "Connection: plain UDP"
                    }
                    None => "Connection: offline",
                });
                if !view.message.is_empty() {
                    ui.label(
                        egui::RichText::new(&view.message).color(if view.message_is_error {
                            theme::RED
                        } else {
                            theme::TEXT_DIM
                        }),
                    );
                }
            });
        self.details_open = open;
    }

    /// The one-screen layout (UC9 §3): the top screen alone. In a battle, GoldLink's HUD takes
    /// the game's text box rows; the touch screen shows as a modal over the picture when the game
    /// needs it (the start menu, an application, a battle menu the HUD doesn't draw) or on Tab.
    fn draw_one_screen(
        &mut self,
        ui: &mut egui::Ui,
        area: egui::Rect,
        scene: Scene,
        battle: Option<&goldlink_bridge::battle_ui::BattleHud>,
    ) {
        let frame = layout::one_screen(area, &self.config.video);
        self.shown_scale = frame.height() / 192.0 * ui.ctx().pixels_per_point();
        let Some(t) = self.texture.clone() else {
            return;
        };
        let p = ui.painter().clone();
        // DS pixels → window points; the frame's middle 256 columns are the DS screen.
        let s = frame.height() / 192.0;
        let middle =
            egui::Rect::from_center_size(frame.center(), egui::vec2(256.0 * s, frame.height()));
        // The wider picture shows in the overworld and in battles; elsewhere (menus, the title,
        // cutscenes' screens) the DS screen sits in the middle of the same frame.
        let wide = self
            .wide
            .clone()
            .filter(|_| self.wide_side > 0 && frame.width() > middle.width() + 1.0)
            .filter(|_| matches!(scene, Scene::Overworld | Scene::StartMenu | Scene::Battle));
        // Without the wider picture (menus, the title, between two scenes), the frame's sides
        // continue the shown screens' edges as the wider picture's strips do (a flat screen, a
        // fade, a bar across it), or are black where the edge is a picture: beside the top screen
        // here, beside both when they're stacked (before the game).
        if frame.width() > middle.width() + 1.0 {
            if wide.is_some() {
                p.rect_filled(frame, 0.0, hud::BAND);
            } else {
                p.rect_filled(frame, 0.0, egui::Color32::BLACK);
            }
        }
        let fill_sides = |screen: egui::Rect, rows: &[Option<Vec<egui::Color32>>; 2]| {
            fill_beside(&p, frame, screen, rows);
        };
        let stacked = scene == Scene::Unknown && self.one.by_hand != Some(false);
        if wide.is_none() && !stacked && frame.width() > middle.width() + 1.0 {
            fill_sides(middle, &self.edge_fill[0]);
        }
        let hud_draws = battle.is_some_and(hud::handles);
        let rows = |to: f32| egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, to));
        match battle {
            // The band only with something for it: the HUD's menu or the game's text box (a link
            // battle's VS screen draws the bottom player's party where the band would be).
            Some(b) if hud_draws || self.game_textbox => {
                // The scene above the band.
                let band = hud::band(frame);
                let scene_rows = hud::SCENE_ROWS / 192.0;
                match &wide {
                    Some(w) => {
                        let r = egui::Rect::from_min_max(
                            frame.min,
                            egui::pos2(frame.right(), band.top()),
                        );
                        p.image(w.id(), r, rows(scene_rows), egui::Color32::WHITE);
                    }
                    None => {
                        let r = egui::Rect::from_min_max(
                            middle.min,
                            egui::pos2(middle.right(), band.top()),
                        );
                        p.image(t.id(), r, rows(scene_rows / 2.0), egui::Color32::WHITE);
                    }
                }
                if hud_draws {
                    let mega = self.mega.map(|lit| hud::Mega {
                        lit,
                        icon: self.mega_icon.as_ref().map(egui::TextureHandle::id),
                    });
                    if let Some(at) = hud::draw(ui, frame, b, mega) {
                        self.one.tap = Some((at, Instant::now()));
                    }
                } else if let Some(tb) = &self.textbox {
                    hud::draw_text_band(ui, frame, tb);
                }
            }
            None if stacked => {
                // Before the game: the title and Oak's speech, whose choices (the topics, boy or
                // girl, the name keyboard) are on the touch screen and whose picture runs across
                // both screens. The two stacked as on the DS, as tall as the frame; clicks on
                // the lower one are touches.
                let size = egui::vec2(256.0, 192.0) * (frame.height() / 384.0);
                self.shown_scale = size.y / 192.0 * ui.ctx().pixels_per_point();
                let top = egui::Rect::from_min_size(
                    egui::pos2(frame.center().x - size.x / 2.0, frame.top()),
                    size,
                );
                let bottom = top.translate(egui::vec2(0.0, size.y));
                fill_sides(top, &self.edge_fill[0]);
                fill_sides(bottom, &self.edge_fill[1]);
                let (top_uv, bottom_uv) = screen_uvs(self.prescale);
                p.image(t.id(), top, top_uv, egui::Color32::WHITE);
                p.image(t.id(), bottom, bottom_uv, egui::Color32::WHITE);
                self.bottom = bottom;
                if self.typing.naming.is_some() {
                    // Beside the keyboard, when there's room.
                    let room = egui::Rect::from_min_max(
                        egui::pos2(bottom.right() + 16.0, bottom.top()),
                        egui::pos2(frame.right() - 16.0, bottom.bottom()),
                    );
                    if room.width() >= 140.0 {
                        typing_hint(&p, room, egui::Align2::LEFT_CENTER);
                    }
                }
                return;
            }
            _ => {
                // The steady overworld: slid back by the shift it was drawn with (UC11).
                let shift = self
                    .steady_shift
                    .filter(|_| matches!(scene, Scene::Overworld | Scene::StartMenu));
                let ppp = ui.ctx().pixels_per_point();
                let origin = self.steady_origin;
                match &wide {
                    Some(w) => {
                        let cols = (256 + 2 * self.wide_side) as f32;
                        let side = self.wide_side as f64;
                        let locked = shift.map(|s| pixel_lock(s, origin, frame, cols, side, ppp));
                        let (r, uv) = slide(frame, rows(1.0), locked, cols, false);
                        p.image(w.id(), r, uv, egui::Color32::WHITE);
                        self.attached
                            .draw(&p, frame, true, cols, shift.is_some(), side);
                        self.front
                            .draw(&p, frame, true, cols, shift.is_some(), side);
                        self.fixed
                            .draw(&p, frame, true, cols, shift.is_some(), side);
                    }
                    None => {
                        let locked = shift.map(|s| pixel_lock(s, origin, middle, 256.0, 0.0, ppp));
                        let (r, uv) =
                            slide(middle, screen_uvs(self.prescale).0, locked, 256.0, true);
                        p.image(t.id(), r, uv, egui::Color32::WHITE);
                        self.attached
                            .draw(&p, middle, false, 256.0, shift.is_some(), 0.0);
                        self.front
                            .draw(&p, middle, false, 256.0, shift.is_some(), 0.0);
                        self.fixed
                            .draw(&p, middle, false, 256.0, shift.is_some(), 0.0);
                    }
                };
            }
        }
        let wanted = matches!(scene, Scene::StartMenu | Scene::App)
            || self.typing.naming.is_some()
            || battle.is_some_and(|b| b.bag_or_party || (b.waiting && !hud_draws));
        let modal = self.one.by_hand.unwrap_or(wanted);
        self.bottom = egui::Rect::NOTHING;
        if modal {
            // The touch screen over the dimmed picture, leaving the picture's edges showing.
            p.rect_filled(frame, 0.0, egui::Color32::from_black_alpha(150));
            let r = egui::Rect::from_center_size(frame.center(), middle.size() * 0.86);
            p.rect_filled(r.expand(6.0), 6.0, egui::Color32::from_black_alpha(120));
            p.image(t.id(), r, screen_uvs(self.prescale).1, egui::Color32::WHITE);
            p.rect_stroke(
                r.expand(1.5),
                3.0,
                egui::Stroke::new(3.0_f32, theme::EDGE),
                egui::StrokeKind::Outside,
            );
            self.bottom = r;
            if self.typing.naming.is_some() {
                // Under the keyboard, in the frame's margin.
                let room = egui::Rect::from_min_max(
                    egui::pos2(frame.left(), r.bottom() + 6.0),
                    egui::pos2(frame.right(), frame.bottom()),
                );
                typing_hint(&p, room, egui::Align2::CENTER_CENTER);
            }
        }
    }

    /// One UI frame (the window's, or the launcher shell's once the game runs). The release
    /// look returns what the player asked of the shell.
    pub fn ui(&mut self, ctx: &egui::Context) -> Option<GameAction> {
        let mut action = None;
        if self.release_ui {
            if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
                if self.settings_open {
                    self.settings_open = false;
                } else {
                    self.menu_open = !self.menu_open;
                }
            }
            if ctx.input(|i| i.key_pressed(egui::Key::F3)) {
                self.details_open = !self.details_open;
            }
            if ctx.input(|i| i.key_pressed(egui::Key::F11)) {
                let v = &mut self.config.video;
                v.fullscreen = !v.fullscreen;
                ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(v.fullscreen));
                self.save_config();
            }
        } else if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            match self.world.as_mut() {
                Some(w) if w.open => w.open = false,
                _ => self.settings_open = !self.settings_open,
            }
        }
        let (scene, battle, naming) = match &self.bridge {
            Some(b) => {
                let mut b = goldlink_bridge::hook::lock(b);
                let naming = b.naming();
                let one = self.config.video.layout == Layout::OneScreen;
                // The wider picture moves the game's HP boxes out toward its edges.
                b.box_shift = if one && self.config.video.wide {
                    layout::WIDE_SIDE as i32
                } else {
                    0
                };
                // The naming screen (a nickname after a catch opens inside the battle) is
                // all on the touch screen: no HUD then.
                if one && naming.is_none() {
                    (b.scene(), b.battle_hud(), naming)
                } else if one {
                    (b.scene(), None, naming)
                } else {
                    (Scene::Unknown, None, naming)
                }
            }
            _ => (Scene::Unknown, None, None),
        };
        {
            let listen = !self.blocked() && !ctx.wants_keyboard_input() && ctx.input(|i| i.focused);
            let events = ctx.input(|i| i.events.clone());
            self.typing.update(naming, &events, listen);
        }
        if self.config.video.layout == Layout::OneScreen {
            self.one.follow(scene);
            if !self.blocked()
                && !ctx.wants_keyboard_input()
                && ctx.input(|i| i.key_pressed(egui::Key::Tab))
            {
                let shown = self.bottom != egui::Rect::NOTHING;
                self.one.by_hand = Some(!shown);
            }
        }
        self.feed_input(ctx);
        self.collect_events();
        self.update_texture(ctx);
        if self.release_ui {
            action = self.top_bar(ctx);
        }

        egui::CentralPanel::default()
            .frame(egui::Frame::NONE.fill(egui::Color32::BLACK))
            .show(ctx, |ui| {
                let area = ui.max_rect();
                if self.config.video.layout == Layout::OneScreen {
                    self.draw_one_screen(ui, area, scene, battle.as_ref());
                } else {
                    let (top, bottom) = layout::screens(area, &self.config.video);
                    self.shown_scale = top.height() / 192.0 * ctx.pixels_per_point();
                    self.bottom = bottom;
                    if let Some(t) = &self.texture {
                        let p = ui.painter();
                        let shift = self
                            .steady_shift
                            .filter(|_| matches!(scene, Scene::Overworld | Scene::StartMenu));
                        let (top_uv, bottom_uv) = screen_uvs(self.prescale);
                        let ppp = ctx.pixels_per_point();
                        let origin = self.steady_origin;
                        let locked = shift.map(|s| pixel_lock(s, origin, top, 256.0, 0.0, ppp));
                        let (slid, uv) = slide(top, top_uv, locked, 256.0, true);
                        p.image(t.id(), slid, uv, egui::Color32::WHITE);
                        self.attached
                            .draw(p, top, false, 256.0, shift.is_some(), 0.0);
                        self.front.draw(p, top, false, 256.0, shift.is_some(), 0.0);
                        self.fixed.draw(p, top, false, 256.0, shift.is_some(), 0.0);
                        p.image(t.id(), bottom, bottom_uv, egui::Color32::WHITE);
                    }
                }
                let now = Instant::now();
                self.messages.retain(|m| m.until > now);
                let mut y = area.bottom() - 8.0;
                for m in self.messages.iter().rev() {
                    let color = if m.error {
                        egui::Color32::from_rgb(255, 120, 110)
                    } else {
                        egui::Color32::WHITE
                    };
                    let galley = ui.painter().layout(
                        m.text.clone(),
                        egui::FontId::proportional(15.0),
                        color,
                        area.width() - 24.0,
                    );
                    let pos = egui::pos2(area.left() + 12.0, y - galley.size().y);
                    let bg = egui::Rect::from_min_size(pos, galley.size()).expand(4.0);
                    ui.painter()
                        .rect_filled(bg, 4.0, egui::Color32::from_black_alpha(200));
                    ui.painter().galley(pos, galley, color);
                    y -= 30.0;
                }
            });

        if self.settings_open {
            self.settings(ctx);
        }
        if self.release_ui {
            if self.menu_open
                && let Some(a) = self.menu(ctx)
            {
                action = Some(a);
            }
            if self.details_open {
                self.details(ctx);
            }
        }
        if let (Some(w), Some(b)) = (self.world.as_mut(), self.bridge.as_ref()) {
            w.show(ctx, b);
        }
        if let Some(extra) = self.extra_ui.as_mut() {
            extra(ctx);
        }
        // The emulator runs on its own clock; draw whenever the display can.
        ctx.request_repaint();
        action
    }

    /// Leaves the world and writes the save (the window closes).
    pub fn shutdown(&mut self) {
        self.stop();
    }
}

impl eframe::App for GameApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        let _ = self.ui(ctx);
    }

    fn raw_input_hook(&mut self, _ctx: &egui::Context, raw: &mut egui::RawInput) {
        if let Some(extra) = self.extra_input.as_mut() {
            extra(raw);
        }
    }

    #[cfg(not(windows))]
    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        self.stop();
    }

    #[cfg(windows)]
    fn on_exit(&mut self) {
        self.stop();
    }
}

impl Drop for GameApp {
    fn drop(&mut self) {
        self.stop();
    }
}

/// How the game's pictures are uploaded for the chosen filter at `scale` physical pixels per DS
/// pixel: (whole-number enlargement, sampling). "Sharp" is nearest sampling at any scale: pixel
/// art, no blending (the user: smoothed it looked blurry at some window sizes). At a scale that
/// isn't a whole number some game pixels get a screen pixel more than others; the steady overworld
/// lands the world on whole screen pixels (`pixel_lock`), so those widths stay with the world as
/// it moves instead of crawling (the reason this was sharp bilinear before, 2026-11-13).
fn sharp_prescale(_scale: f32, filter: Filter) -> (usize, egui::TextureOptions) {
    match filter {
        Filter::Linear => (1, egui::TextureOptions::LINEAR),
        Filter::Nearest => (1, egui::TextureOptions::NEAREST),
    }
}

/// A mask (or the like) as wide as a picture `w` across (they come as wide as the wider picture,
/// 192 rows): its middle, or nothing when it's narrower.
fn fit<T: Copy + Default>(m: &[T], w: usize) -> Vec<T> {
    let cols = m.len() / 192;
    if m.is_empty() || cols < w {
        return vec![T::default(); w * 192];
    }
    let from = (cols - w) / 2;
    m.chunks_exact(cols)
        .flat_map(|r| r[from..from + w].iter().copied())
        .collect()
}

/// One of the steady overworld's layers over the slid picture (UC11): its texture, whether this
/// frame has it, whether it's the wider picture's (else the DS screen's), and its own shift.
#[derive(Default)]
struct Layer {
    texture: Option<egui::TextureHandle>,
    shown: bool,
    wide: bool,
    shift: Option<[f32; 2]>,
    /// Where what it's slid for truly is (DS screen columns and rows): see `pixel_lock`.
    origin: [f64; 2],
}

impl Layer {
    fn set(
        &mut self,
        ctx: &egui::Context,
        name: &str,
        img: egui::ColorImage,
        options: egui::TextureOptions,
        wide: bool,
        shift: Option<[f32; 2]>,
    ) {
        match &mut self.texture {
            Some(t) if t.size() == img.size => t.set(img, options),
            _ => self.texture = Some(ctx.load_texture(name, img, options)),
        }
        self.shown = true;
        self.wide = wide;
        self.shift = shift;
    }

    /// Over the picture drawn into `dst` (`cols` DS pixels across, the screen's column 0 at
    /// `side`), when this frame has it for that picture: slid back by its own shift when the
    /// picture is slid (`slid`), else as drawn (the attached layer holds the shadow, which the
    /// picture doesn't: it's always drawn).
    fn draw(
        &self,
        p: &egui::Painter,
        dst: egui::Rect,
        wide: bool,
        cols: f32,
        slid: bool,
        side: f64,
    ) {
        let Some(t) = self
            .texture
            .as_ref()
            .filter(|_| self.shown && self.wide == wide)
        else {
            return;
        };
        let full = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
        let ppp = p.ctx().pixels_per_point();
        let shift = self
            .shift
            .filter(|_| slid)
            .map(|s| pixel_lock(s, self.origin, dst, cols, side, ppp));
        let (r, uv) = slide(dst, full, shift, cols, false);
        p.image(t.id(), r, uv, egui::Color32::WHITE);
    }
}

/// The lighting's gain per channel on the ground around the player (the scene's pixels within
/// two of their opaque ones, `mask` 1: lit `top` over `unlit`), for the world under them (the
/// player's own pixels have their own light: the ground under them came out another shade, ghost
/// pixels at the sprite's edge); none (1) without the lighting. `w` across.
fn light_gain(top: &[u32], unlit: Option<&[u32]>, mask: &[u8], w: usize) -> [f32; 3] {
    let Some(unlit) = unlit else {
        return [1.0; 3];
    };
    let h = mask.len() / w.max(1);
    let near = |x: usize, y: usize| {
        (y.saturating_sub(2)..(y + 3).min(h))
            .any(|yy| (x.saturating_sub(2)..(x + 3).min(w)).any(|xx| mask[yy * w + xx] == 1))
    };
    let (mut lit_sum, mut raw_sum) = ([0f32; 3], [0f32; 3]);
    for y in 0..h {
        for x in 0..w {
            let i = y * w + x;
            if mask[i] != 0 || !near(x, y) {
                continue;
            }
            for (c, sh) in [16, 8, 0].into_iter().enumerate() {
                lit_sum[c] += ((top[i] >> sh) & 255) as f32;
                raw_sum[c] += ((unlit[i] >> sh) & 255) as f32;
            }
        }
    }
    let mut gain = [1f32; 3];
    for c in 0..3 {
        if raw_sum[c] > 255.0 {
            gain[c] = lit_sum[c] / raw_sum[c];
        }
    }
    gain
}

/// `c` (0x..RRGGBB) times a gain per channel.
fn scaled(c: u32, gain: [f32; 3]) -> u32 {
    let ch = |sh: u32, g: f32| ((((c >> sh) & 255) as f32 * g).min(255.0) as u32) << sh;
    ch(16, gain[0]) | ch(8, gain[1]) | ch(0, gain[2])
}

/// The lighting's gain per channel on the player's own opaque pixels (`mask` 1: lit `top` over
/// `unlit`), for their pixels behind the world; none (1) without the lighting.
fn player_gain(top: &[u32], unlit: Option<&[u32]>, mask: &[u8]) -> [f32; 3] {
    let Some(unlit) = unlit else {
        return [1.0; 3];
    };
    let (mut lit_sum, mut raw_sum) = ([0f32; 3], [0f32; 3]);
    for i in (0..mask.len()).filter(|&i| mask[i] == 1) {
        for (c, sh) in [16, 8, 0].into_iter().enumerate() {
            lit_sum[c] += ((top[i] >> sh) & 255) as f32;
            raw_sum[c] += ((unlit[i] >> sh) & 255) as f32;
        }
    }
    let mut gain = [1f32; 3];
    for c in 0..3 {
        if raw_sum[c] > 255.0 {
            gain[c] = lit_sum[c] / raw_sum[c];
        }
    }
    gain
}

/// What's in front of the player (`mask` 3 and 4: the world over their pixels, and the same
/// surface just past it), `top`'s pixels opaque, the rest transparent, `w` across, each `k`
/// times as large: drawn over the player's layer with the world's shift, so it covers them
/// exactly where the rest of it is (the two slide apart by a fraction: slivers of the player
/// showed past its edges, the user's play test).
/// `edge` (0xAARRGGBB, its own colour and coverage: `lit_edges`) where it has one: over the
/// player's layer it blends with them where they are on screen, not where the DS drew them.
fn front_layer(top: &[u32], mask: &[u8], edge: &[u32], w: usize, k: usize) -> egui::ColorImage {
    let colour = |p: u32| egui::Color32::from_rgb((p >> 16) as u8, (p >> 8) as u8, p as u8);
    let layer = (0..w * 192)
        .map(|i| match (mask[i], edge[i] >> 24) {
            (3, a) if a > 0 => {
                let e = edge[i];
                egui::Color32::from_rgba_unmultiplied(
                    (e >> 16) as u8,
                    (e >> 8) as u8,
                    e as u8,
                    a as u8,
                )
            }
            (3 | 4, _) => colour(top[i]),
            _ => egui::Color32::TRANSPARENT,
        })
        .collect();
    enlarged_layer(layer, w, 192, k)
}

/// The world's edges over the player (`mask` 3 with an `edge`, 0xAARRGGBB: colour and coverage),
/// as shown: the picture's pixel there (`top`) is the edge blended with the world under the player
/// (`under`), then lit (and whatever else the screen did to it); each channel's ratio of the two
/// lights the edge and the world under alike. `top` there becomes the world under the player; the
/// edges (alpha their coverage) are returned, else 0.
fn lit_edges(top: &mut [u32], mask: &[u8], edge: &[u32], under: &[u32]) -> Vec<u32> {
    let mut out = vec![0u32; edge.len()];
    for i in (0..edge.len()).filter(|&i| mask[i] == 3 && edge[i] >> 24 != 0) {
        let (e, u, c) = (edge[i], under[i], (edge[i] >> 24) as f32 / 255.0);
        let mut gain = [1f32; 3];
        for (g, sh) in gain.iter_mut().zip([16, 8, 0]) {
            let ch = |p: u32| ((p >> sh) & 255) as f32;
            let blended = ch(e) * c + ch(u) * (1.0 - c);
            *g = (ch(top[i]) + 1.0) / (blended + 1.0);
        }
        top[i] = scaled(u, gain);
        out[i] = (e & 0xFF00_0000) | scaled(e, gain);
    }
    out
}

/// A layer of the steady overworld's picture (UC11), `w` across, 192 rows, each pixel `k` times as
/// large, drawn over the slid picture (with its own shift, or none): where `mask` is 1, `top`'s
/// pixel, opaque; where it's 2, `trans`'s (0xAARRGGBB: a shadow the picture doesn't have); the
/// rest transparent. In `top` (the picture that's slid) the 1s take the world under them (with
/// `under`: `trans` there, the renderer's, times the lighting's gain) or else the colour of the
/// nearest scene pixel (neither this layer's 1s nor `other`), filled in from the edge: where the
/// shifts differ (about half a pixel at most) the world shows there, never this layer's pixels a
/// second time at another shift.
fn split_layer(
    top: &mut [u32],
    mask: &[u8],
    other: &[u8],
    w: usize,
    k: usize,
    trans: Option<&[u32]>,
    under: Option<[f32; 3]>,
) -> egui::ColorImage {
    let colour = |p: u32| egui::Color32::from_rgb((p >> 16) as u8, (p >> 8) as u8, p as u8);
    let h = 192;
    let mut layer = vec![egui::Color32::TRANSPARENT; w * h];
    for (i, px) in layer.iter_mut().enumerate() {
        match (mask[i], trans) {
            (1, _) => *px = colour(top[i]),
            (3, Some(t)) => *px = colour(t[i]),
            (2, Some(t)) => {
                let c = t[i];
                *px = egui::Color32::from_rgba_unmultiplied(
                    (c >> 16) as u8,
                    (c >> 8) as u8,
                    c as u8,
                    (c >> 24) as u8,
                );
            }
            _ => {}
        }
    }
    if let (Some(gain), Some(t)) = (under, trans) {
        for i in (0..w * h).filter(|&i| mask[i] == 1) {
            top[i] = scaled(t[i], gain);
        }
        return enlarged_layer(layer, w, h, k);
    }
    // Breadth first from the scene's pixels beside the layer.
    let mut done: Vec<bool> = (0..w * h).map(|i| mask[i] != 1).collect();
    let mut fill = vec![0u32; w * h];
    let mut queue = std::collections::VecDeque::new();
    for y in 0..h {
        for x in 0..w {
            let i = y * w + x;
            if mask[i] == 1 || other[i] != 0 {
                continue;
            }
            let beside = (x > 0 && mask[i - 1] == 1)
                || (x + 1 < w && mask[i + 1] == 1)
                || (y > 0 && mask[i - w] == 1)
                || (y + 1 < h && mask[i + w] == 1);
            if beside {
                fill[i] = top[i];
                queue.push_back(i);
            }
        }
    }
    while let Some(i) = queue.pop_front() {
        let (x, y) = (i % w, i / w);
        let near = [
            (x > 0).then(|| i - 1),
            (x + 1 < w).then(|| i + 1),
            (y > 0).then(|| i - w),
            (y + 1 < h).then(|| i + w),
        ];
        for n in near.into_iter().flatten() {
            if !done[n] {
                done[n] = true;
                fill[n] = fill[i];
                top[n] = fill[i];
                queue.push_back(n);
            }
        }
    }
    enlarged_layer(layer, w, h, k)
}

/// A layer's pixels (`w` by `h`) as a texture image, each `k` by `k`.
fn enlarged_layer(layer: Vec<egui::Color32>, w: usize, h: usize, k: usize) -> egui::ColorImage {
    if k <= 1 {
        return egui::ColorImage {
            size: [w, h],
            pixels: layer,
        };
    }
    let mut pixels = Vec::with_capacity(w * k * h * k);
    for row in layer.chunks_exact(w) {
        let line: Vec<_> = row
            .iter()
            .flat_map(|&p| std::iter::repeat_n(p, k))
            .collect();
        for _ in 0..k {
            pixels.extend_from_slice(&line);
        }
    }
    egui::ColorImage {
        size: [w * k, h * k],
        pixels,
    }
}

/// The game's picture (`w` by `h`, 0x00RRGGBB) as a texture image, each pixel `k` by `k`.
fn enlarged(px: &[u32], w: usize, h: usize, k: usize) -> egui::ColorImage {
    let colour = |p: u32| egui::Color32::from_rgb((p >> 16) as u8, (p >> 8) as u8, p as u8);
    if k <= 1 {
        return egui::ColorImage {
            size: [w, h],
            pixels: px.iter().map(|&p| colour(p)).collect(),
        };
    }
    let mut pixels = Vec::with_capacity(w * k * h * k);
    let mut line = Vec::with_capacity(w * k);
    for row in px.chunks_exact(w).take(h) {
        line.clear();
        for &p in row {
            line.extend(std::iter::repeat_n(colour(p), k));
        }
        for _ in 0..k {
            pixels.extend_from_slice(&line);
        }
    }
    egui::ColorImage {
        size: [w * k, h * k],
        pixels,
    }
}

/// Fills the frame beside a shown screen (`screen`, inside `frame`), left and right, row by row
/// with `rows` (`edge_rows`: none leaves that side as it is, black).
fn fill_beside(
    p: &egui::Painter,
    frame: egui::Rect,
    screen: egui::Rect,
    rows: &[Option<Vec<egui::Color32>>; 2],
) {
    // Each fill runs a point under the screen (drawn over it next), so no seam of the black
    // behind shows where the two meet at a fraction of a pixel.
    for (rows, x0, x1) in [
        (&rows[0], frame.left(), screen.left() + 1.0),
        (&rows[1], screen.right() - 1.0, frame.right()),
    ] {
        let Some(rows) = rows else { continue };
        if x1 - x0 < 1.5 || rows.is_empty() {
            continue;
        }
        let row_h = screen.height() / rows.len() as f32;
        let mut y = 0;
        while y < rows.len() {
            let mut end = y + 1;
            while end < rows.len() && rows[end] == rows[y] {
                end += 1;
            }
            let r = egui::Rect::from_min_max(
                egui::pos2(x0, screen.top() + y as f32 * row_h),
                egui::pos2(x1, screen.top() + end as f32 * row_h),
            );
            p.rect_filled(r, 0.0, rows[y]);
            y = end;
        }
    }
}

/// A screen's left and right edges, continued beside it when the wider picture isn't
/// shown: per side, each row's colour when every row's 8 edge pixels are one colour (a flat
/// screen, a fade, a bar across the screen), or none (black) where the edge is a picture.
fn edge_rows(top: &[u32]) -> [Option<Vec<egui::Color32>>; 2] {
    let side = |cols: std::ops::Range<usize>| -> Option<Vec<egui::Color32>> {
        top.chunks_exact(256)
            .map(|row| {
                let first = row[cols.start] & 0xFF_FFFF;
                row[cols.clone()]
                    .iter()
                    .all(|&p| p & 0xFF_FFFF == first)
                    .then(|| {
                        egui::Color32::from_rgb(
                            (first >> 16) as u8,
                            (first >> 8) as u8,
                            first as u8,
                        )
                    })
            })
            .collect()
    };
    [side(0..8), side(248..256)]
}

/// The steady overworld (UC11): `shift` (DS pixels) adjusted by under half a screen pixel so that
/// what it's slid for (truly at `origin`: DS screen columns and rows; the picture's column 0 is
/// the screen's `-side`) lands on a whole pixel of the screen, drawn into `dst` (`cols` DS
/// pixels across, 192 rows; `ppp` the screen's pixels per point). At a scale that isn't whole
/// (7.5), which game pixel gets one screen pixel more depends on where the picture lands within a
/// screen pixel; slid by fractions as the world moved, fine texture flickered (the user's play
/// test). Locked so, every game pixel keeps its screen pixels while it moves.
fn pixel_lock(
    shift: [f32; 2],
    origin: [f64; 2],
    dst: egui::Rect,
    cols: f32,
    side: f64,
    ppp: f32,
) -> [f32; 2] {
    let axis = |shift: f32, origin: f64, min: f32, size: f32, n: f32| {
        let s = f64::from(size * ppp / n);
        if s <= 0.0 {
            return shift;
        }
        // A quarter past a whole pixel: with nearest sampling at 7.5x (or 8x) no game pixel's edge
        // is then on a screen pixel's centre, where the last bit of arithmetic picks the side.
        let ideal = f64::from(min * ppp) + origin * s;
        let landed = (ideal - 0.25).round() + 0.25;
        (f64::from(shift) + (ideal - landed) / s) as f32
    };
    [
        axis(shift[0], origin[0] + side, dst.min.x, dst.width(), cols),
        axis(shift[1], origin[1], dst.min.y, dst.height(), 192.0),
    ]
}

/// The steady overworld (UC11): the top picture `uv` (the DS screen's 192 rows, `cols` columns
/// across), drawn into `dst`, slid by `shift` DS pixels: melonDS drew the scene that far off so
/// it moves in whole pixels, and this takes it back at the screen's resolution. The picture keeps
/// filling `dst`: past its edges the texture repeats its edge pixel (a sliver under a pixel), so
/// nothing behind it shows. With `shared`, the texture holds more than this picture (the two
/// screens stacked): there the sliver is left out instead of read from the other screen.
fn slide(
    dst: egui::Rect,
    uv: egui::Rect,
    shift: Option<[f32; 2]>,
    cols: f32,
    shared: bool,
) -> (egui::Rect, egui::Rect) {
    let Some([sx, sy]) = shift else {
        return (dst, uv);
    };
    let moved = uv.translate(egui::vec2(sx / cols * uv.width(), sy / 192.0 * uv.height()));
    if !shared {
        return (dst, moved);
    }
    let kept = moved.intersect(uv);
    if !kept.is_positive() {
        return (dst, uv);
    }
    let at = |p: egui::Pos2| {
        egui::pos2(
            dst.left() + (p.x - moved.left()) / moved.width() * dst.width(),
            dst.top() + (p.y - moved.top()) / moved.height() * dst.height(),
        )
    };
    (egui::Rect::from_min_max(at(kept.min), at(kept.max)), kept)
}

#[cfg(test)]
mod slide_tests {
    use super::*;

    #[test]
    fn slide_takes_the_shift_back_and_crops_the_edge() {
        let dst = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(512.0, 384.0));
        // Drawn 0.5 px right and 0.25 px down: shown from 0.5 px further right and 0.25 down.
        let (r, uv) = slide(dst, TOP_UV, Some([0.5, 0.25]), 256.0, true);
        assert!((uv.min.x - 0.5 / 256.0).abs() < 1e-6);
        assert!((uv.min.y - 0.25 / 384.0).abs() < 1e-6);
        assert!((uv.max.x - 1.0).abs() < 1e-6 && (uv.max.y - 0.5).abs() < 1e-6);
        // The picture keeps its scale (2 window points per DS pixel): it moved 1 point left and
        // 0.5 up, and the sliver past the right and bottom edges is left out.
        assert!((r.min.x - 0.0).abs() < 1e-3 && (r.min.y - 0.0).abs() < 1e-3);
        assert!((r.max.x - 511.0).abs() < 1e-3 && (r.max.y - 383.5).abs() < 1e-3);
        // Nothing to take back.
        assert_eq!(slide(dst, TOP_UV, None, 256.0, true), (dst, TOP_UV));
        // Its own texture: the picture still fills the frame, nothing behind it shows.
        let wide = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
        let (r, uv) = slide(dst, wide, Some([0.5, 0.25]), 342.0, false);
        assert_eq!(r, dst);
        assert!((uv.min.x - 0.5 / 342.0).abs() < 1e-6);
    }

    #[test]
    fn sharp_pixels_are_nearest_at_any_scale() {
        for scale in [7.0, 1.6, 7.5, 3.72] {
            assert_eq!(
                sharp_prescale(scale, Filter::Nearest),
                (1, egui::TextureOptions::NEAREST)
            );
        }
        assert_eq!(
            sharp_prescale(3.72, Filter::Linear),
            (1, egui::TextureOptions::LINEAR)
        );
        let img = enlarged(&[0x00FF_0000, 0x0000_00FF], 2, 1, 3);
        assert_eq!(img.size, [6, 3]);
        let red = egui::Color32::from_rgb(255, 0, 0);
        let blue = egui::Color32::from_rgb(0, 0, 255);
        assert_eq!(&img.pixels[12..18], &[red, red, red, blue, blue, blue]);
    }

    #[test]
    fn sides_continue_the_screens_edges() {
        let white = egui::Color32::from_rgb(0xF8, 0xF8, 0xF8);
        // A flat screen: both sides its colour.
        let flat = vec![0x00F8_F8F8u32; 256 * 192];
        assert_eq!(
            edge_rows(&flat),
            [Some(vec![white; 192]), Some(vec![white; 192])]
        );
        // A bar across the top of a black screen: continued row by row.
        let mut bar = vec![0u32; 256 * 192];
        bar[..256 * 20].fill(0x00F8_F8F8);
        let rows = edge_rows(&bar)[0].clone().unwrap();
        assert_eq!(
            (rows[0], rows[19], rows[20]),
            (white, white, egui::Color32::BLACK)
        );
        // A picture at the left edge: that side black, the other still continued.
        let mut picture = flat.clone();
        picture[256 * 100 + 2] = 0x0020_6020;
        assert_eq!(edge_rows(&picture)[0], None);
        assert_eq!(edge_rows(&picture)[1], Some(vec![white; 192]));
    }
}
