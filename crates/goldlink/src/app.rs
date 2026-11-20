//! The game window (RFC §2.2, §4.4, UC2): both screens, the stylus, keyboard and gamepads,
//! sound, and a settings panel on Esc. No emulator chrome: no menus, rewind or savestate keys.

use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
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

/// The first Pokémon's choice in the one-screen layout: the Pokémon as the top screen shows it
/// (whatever the ball holds, a randomised world's too), in a round bubble over the top middle of
/// the touch screen `r`, at the touch screen's scale (the user: the Poké Balls and the Pokémon at
/// once; the middle holds the ball's button). Its middle and radius (points).
fn starter_bubble(
    p: &egui::Painter,
    t: &egui::TextureHandle,
    prescale: usize,
    r: egui::Rect,
) -> (egui::Pos2, f32) {
    let k = r.height() / 192.0;
    let (sx, sy, sr) = STARTER_SPOT;
    let radius = sr * k;
    // Over the touch screen's top 14 rows, between the balls.
    let centre = egui::pos2(r.center().x, r.top() + 14.0 * k - radius);
    let uv = screen_uvs(prescale).0;
    let at = |x: f32, y: f32| {
        egui::pos2(
            uv.min.x + x / 256.0 * uv.width(),
            uv.min.y + y / 192.0 * uv.height(),
        )
    };
    p.circle_filled(centre, radius + 6.0, egui::Color32::from_black_alpha(120));
    let mut mesh = egui::Mesh::with_texture(t.id());
    const SIDES: u32 = 72;
    mesh.vertices.push(egui::epaint::Vertex {
        pos: centre,
        uv: at(sx, sy),
        color: egui::Color32::WHITE,
    });
    for i in 0..=SIDES {
        let a = i as f32 / SIDES as f32 * std::f32::consts::TAU;
        let (c, s) = (a.cos(), a.sin());
        mesh.vertices.push(egui::epaint::Vertex {
            pos: centre + egui::vec2(c, s) * radius,
            uv: at(sx + c * sr, sy + s * sr),
            color: egui::Color32::WHITE,
        });
        if i > 0 {
            mesh.add_triangle(0, i, i + 1);
        }
    }
    p.add(egui::Shape::mesh(mesh));
    p.circle_stroke(
        centre,
        radius + 1.5,
        egui::Stroke::new(3.0_f32, theme::EDGE),
    );
    (centre, radius)
}

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

/// The Shadows setting's hint (both settings screens).
pub const SHADOWS_HINT: &str = "Outdoors, things cast shadows from the sun, the moon and lit windows and lamps; off: the game's own shadows";

/// Why the revamped lighting can't run here, for the settings (BACKLOG #12): they showed it
/// ticked while the GPU had turned it off.
pub fn lighting_unavailable() -> Option<String> {
    use goldlink_light::Unavailable;
    const RESTART: &str = "If you just updated your graphics driver, restart your computer.";
    Some(match goldlink_light::unavailable()? {
        Unavailable::NoGpu => {
            format!("Revamped lighting is unavailable: no graphics card here can run it. {RESTART}")
        }
        Unavailable::Unsupported { gl: true } => format!(
            "Revamped lighting is unavailable: your graphics driver doesn't support it. {RESTART}"
        ),
        Unavailable::Unsupported { gl: false } => {
            "Revamped lighting is unavailable: your graphics driver doesn't support it.".into()
        }
        Unavailable::Stopped => {
            "Revamped lighting stopped: the graphics card stopped responding. Restart GoldLink to try again."
                .into()
        }
    })
}

/// What the release program's game window asks of the shell.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GameAction {
    /// Leave the world: the game saves, then back to the launcher.
    Leave,
}

/// The running shoes' tap from the start menu pop-up: B closes the menu, then (back in the
/// overworld) the game's own button on the field's touch screen is tapped.
#[derive(Debug, Clone, Copy, PartialEq)]
enum ShoesTap {
    Closing,
    Waiting,
    Tapping,
}

/// Where HeartGold draws its running shoes button on the field's touch screen (DS pixels: the
/// button's dark outline from x 184 to the screen's edge, y 89 to 130), and where a tap toggles
/// running (pret overlay 28's hitbox: x 178-252, y 86-134).
const SHOES_BUTTON: (usize, usize, usize, usize) = (184, 89, 256, 131);
const SHOES_TAP_AT: (u16, u16) = (215, 110);

/// The game's running shoes button from the touch screen's pixels (`bottom`, 256 x 192,
/// 0x00RRGGBB), when the game draws it (its dark outline at the top, the bottom and the left; the
/// game hides it while cycling or in a script).
fn shoes_icon(bottom: &[u32]) -> Option<egui::ColorImage> {
    let (x0, y0, x1, y1) = SHOES_BUTTON;
    let dark = |x: usize, y: usize| {
        let p = bottom[y * 256 + x];
        ((p >> 16) & 255) + ((p >> 8) & 255) + (p & 255) < 260
    };
    let outlined = (x0 + 6..x1)
        .step_by(9)
        .all(|x| dark(x, y0) && dark(x, y1 - 2))
        && (y0 + 4..y1 - 4).step_by(6).all(|y| dark(x0, y));
    if !outlined || dark(x0 + 8, y0 + 6) {
        return None;
    }
    let pixels = (y0..y1)
        .flat_map(|y| (x0..x1).map(move |x| (x, y)))
        .map(|(x, y)| {
            let p = bottom[y * 256 + x];
            egui::Color32::from_rgb((p >> 16) as u8, (p >> 8) as u8, p as u8)
        })
        .collect();
    Some(egui::ColorImage {
        size: [x1 - x0, y1 - y0],
        pixels,
    })
}

/// The one-screen layout's state (UC9 §3).
#[derive(Debug, Default)]
struct OneScreen {
    /// Tab: the touch screen shown (or hidden) by hand, until the scene changes.
    by_hand: Option<bool>,
    /// The scene the last frame showed.
    scene: Option<Scene>,
    /// The scene the shown application was opened from (the start menu's, or a script's).
    app_from: Option<Scene>,
    /// The game shows the choice of the first Pokémon (the bridge's `choosing_starter`).
    starter: bool,
    /// The starter's bubble over the touch screen (its middle and radius, points): clicks there
    /// aren't touches.
    bubble: Option<(egui::Pos2, f32)>,
    /// A touch the HUD is making for the player: the DS point and when it began.
    tap: Option<((u16, u16), Instant)>,
}

impl OneScreen {
    fn follow(&mut self, scene: Scene) {
        if scene != Scene::Changing && self.scene != Some(scene) {
            if scene == Scene::App {
                self.app_from = self.scene;
            }
            self.scene = Some(scene);
            self.by_hand = None;
        }
    }

    /// Whether the touch screen shows Elm's Poké Balls (the first Pokémon's choice, from the
    /// lab's script, not an application of the start menu there).
    fn choosing_starter(&self, scene: Scene) -> bool {
        self.starter && scene == Scene::App && self.app_from != Some(Scene::StartMenu)
    }
}

/// Where the first Pokémon's choice shows the Pokémon on the top screen (DS pixels): its 80 x 80
/// sprite stands in the middle of its round stage, above Elm's text box.
const STARTER_SPOT: (f32, f32, f32) = (128.0, 82.0, 46.0);

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
    /// A game window reaching into the band's rows (the level-up's stats box): drawn over the
    /// band, as the game draws it over its text box (`hud::window_under_band`).
    band_window: Option<[usize; 4]>,
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
    edge_fill: [[Option<EdgeFill>; 2]; 2],
    /// The steady overworld's shift for the top picture on screen (UC11): slid back by it.
    steady_shift: Option<[f32; 2]>,
    /// The running shoes (`Frame::running_shoes`): None until the player has them.
    running_shoes: Option<bool>,
    /// The touch screen shows the start menu's panel (`hud::start_menu_panel`).
    start_panel: bool,
    /// Frames left to show both screens in the one-screen layout: the Pokégear is up (a phone
    /// call's words are on the top screen, its choices on the touch screen; the user, 2026-11-18).
    pokegear_hold: u8,
    /// The touch screen's choice buttons the game waits on (the nurse's YES and NO, the PC's
    /// list) and their words: the one-screen layout lists them in a white box over the picture
    /// (the user, 2026-11-18).
    choices: Vec<hud::TouchChoice>,
    choice_tex: Vec<egui::TextureHandle>,
    /// The open menu's words as the ROM has them (`menu_text`), the words the textures show, and
    /// the text boxes' font they're written in.
    menu_words: Vec<Vec<u16>>,
    choice_words: Vec<Vec<u16>>,
    game_font: Option<std::sync::Arc<goldlink_bridge::menu_text::GameFont>>,
    /// And where the world's origin truly is in it (DS screen columns and rows): see `pixel_lock`.
    steady_origin: [f64; 2],
    /// With it, over the slid picture (split_layer): what the camera follows (the player, their
    /// shadow), slid back by its own shift, and the 2D drawn over the scene (a text box, a
    /// banner), where it is.
    /// The steady overworld's two pictures put together round the player (UC12).
    attached: AttachedView,
    fixed: Layer,
    /// hg-engine's MEGA button, as the touch screen shows it (None: not offered; else lit), and
    /// its Mega symbol cut from there.
    mega: Option<bool>,
    mega_icon: Option<egui::TextureHandle>,
    /// The game's running shoes button as it last drew it on the field's touch screen.
    shoes_icon: Option<egui::TextureHandle>,
    /// The start menu pop-up's shoes button was clicked: the menu closing, then the game's own
    /// button tapped once back in the overworld (`feed_input`).
    shoes_tap: Option<(ShoesTap, Instant)>,
    /// The lighting's buffers and worker threads (UC11), from the first lit frame.
    /// The lighting's thread (UC14), with the lighting on.
    light_thread: Option<LightThread>,
    /// No usable GPU for the lighting (UC14): it's off for this session (said once).
    no_gpu: bool,
    /// Each emulated frame lit and shown before the next is taken (the dev tour's films).
    pub frames_in_step: bool,
    /// Tab was pressed for the game since the last frame ([`GameApp::take_tab`]).
    tab: bool,

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
            band_window: None,
            wide: None,
            wide_side: 0,
            shown_scale: 1.0,
            prescale: 1,
            steady_shift: None,
            running_shoes: None,
            start_panel: false,
            pokegear_hold: 0,
            choices: Vec::new(),
            choice_tex: Vec::new(),
            menu_words: Vec::new(),
            choice_words: Vec::new(),
            game_font: None,
            steady_origin: [0.0; 2],
            attached: AttachedView::default(),
            fixed: Layer::default(),
            edge_fill: [[None, None], [None, None]],
            mega: None,
            mega_icon: None,
            shoes_icon: None,
            shoes_tap: None,
            light_thread: None,
            no_gpu: false,
            frames_in_step: false,
            tab: false,

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
            r.send(Command::Shadows(v.shadows));
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
        let on_bubble = |p: egui::Pos2| self.one.bubble.is_some_and(|(c, r)| p.distance(c) <= r);
        let mut touch = match (down, pos) {
            (true, Some(p)) if !self.blocked() && !over_panel && !on_bubble(p) => {
                touch_at(self.bottom, p)
            }
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
        // The start menu pop-up's shoes button: B closes the menu; back in the overworld, the
        // game's own button is tapped (the game toggles running, with its animation).
        if let Some((step, since)) = self.shoes_tap {
            let t = since.elapsed();
            match step {
                ShoesTap::Closing if t < Duration::from_millis(150) => {
                    keys |= goldlink_core::Keys::B
                }
                ShoesTap::Closing => self.shoes_tap = Some((ShoesTap::Waiting, Instant::now())),
                ShoesTap::Waiting
                    if self.one.scene == Some(Scene::Overworld)
                        && t > Duration::from_millis(300) =>
                {
                    self.shoes_tap = Some((ShoesTap::Tapping, Instant::now()))
                }
                ShoesTap::Waiting if t > Duration::from_secs(3) => self.shoes_tap = None,
                ShoesTap::Waiting => {}
                ShoesTap::Tapping if t < TAP => touch = Some(SHOES_TAP_AT),
                ShoesTap::Tapping => self.shoes_tap = None,
            }
        }
        *lock(&runner.shared.input) = Input { keys, touch };
    }

    fn update_texture(&mut self, ctx: &egui::Context) {
        let Some(runner) = &self.runner else { return };
        let lit = self.config.video.lighting && !self.no_gpu;
        // A new frame from the emulation: what the HUD reads at once; the picture lit on the
        // lighting thread (UC14), so the window never waits for it (at 240 Hz it has 4 ms a
        // frame, and lighting a frame took 6).
        let taken = {
            let frame = lock(&runner.shared.frame);
            if frame.seq == self.last_seq {
                None
            } else {
                self.last_seq = frame.seq;
                // What the HUD reads, from the game's own pixels.
                if self.config.video.layout == Layout::OneScreen {
                    self.mega = hud::mega_state(&frame.pixels[256 * 192..]);
                    self.start_panel = hud::start_menu_panel(&frame.pixels[256 * 192..]);
                    // The Pokégear (a phone call among it) shows both screens for a while after
                    // its frame was last seen (its fades don't flip the layout).
                    let bottom = &frame.pixels[256 * 192..];
                    if hud::pokegear_shown(bottom) || hud::phone_call_shown(bottom) {
                        self.pokegear_hold = 15;
                    } else {
                        self.pokegear_hold = self.pokegear_hold.saturating_sub(1);
                    }
                    self.choices = hud::touch_choices(bottom);
                    if frame.running_shoes.is_some()
                        && let Some(img) = shoes_icon(&frame.pixels[256 * 192..])
                    {
                        let nearest = egui::TextureOptions::NEAREST;
                        match &mut self.shoes_icon {
                            Some(t) => t.set(img, nearest),
                            None => {
                                self.shoes_icon = Some(ctx.load_texture("shoes-icon", img, nearest))
                            }
                        }
                    }
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
                    self.band_window = hud::window_under_band(&frame.pixels[..256 * 192]);
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
                self.running_shoes = frame.running_shoes;
                Some(Shown {
                    pixels: frame.pixels.clone(),
                    wide,
                    side: frame.wide_side,
                    light: if lit { frame.light.clone() } else { None },
                    fixed: frame.fixed.clone(),
                    attached: frame.attached.clone(),
                    steady_shift: frame.steady_shift,
                    steady_origin: frame.steady_origin,
                    edge_fill: [
                        edge_rows(&frame.pixels[..256 * 192]),
                        edge_rows(&frame.pixels[256 * 192..256 * 384]),
                    ],
                    unlit: None,
                    light_maps: None,
                })
            }
        };
        let shown = match (taken, lit) {
            (taken, false) => {
                self.light_thread = None;
                taken
            }
            (taken, true) => {
                let worker = self
                    .light_thread
                    .get_or_insert_with(|| LightThread::start(ctx.clone()));
                // Not lit while minimized: each lit frame asks for a frame of the window.
                if let Some(s) = taken.filter(|_| !minimized(ctx)) {
                    worker.send(s, self.config.video.shadows);
                    if self.frames_in_step {
                        worker.wait();
                    }
                }
                let (lit, gone) = (worker.take(), worker.no_gpu());
                if gone {
                    // No usable GPU: the picture as the game draws it, said once.
                    self.no_gpu = true;
                    self.light_thread = None;
                    self.say(
                        "Your graphics card can't run GoldLink's lighting, so it's off.",
                        false,
                    );
                }
                lit
            }
        };
        let (k, options) = sharp_prescale(self.shown_scale, self.config.video.filter);
        let Some(Shown {
            mut pixels,
            mut wide,
            side,
            fixed,
            attached,
            steady_shift,
            steady_origin,
            edge_fill,
            unlit,
            light_maps,
            ..
        }) = shown
        else {
            if self.texture.is_none() {
                let img = egui::ColorImage::new([256, 384], egui::Color32::BLACK);
                self.texture = Some(ctx.load_texture("screens", img, options));
            }
            return;
        };
        // What the picture shown is: its wider sides, its slide, its edges.
        self.wide_side = side;
        self.steady_shift = steady_shift;
        self.steady_origin = steady_origin;
        self.edge_fill = edge_fill;
        let image = |px: &[u32], w: usize, h: usize| enlarged(px, w, h, k);
        // The steady overworld's layers (UC11, UC12): the player and the world put together at
        // the screen's resolution round the player, slid with the picture, and the 2D over the
        // scene, not slid.
        self.attached.data = None;
        self.fixed.shown = false;
        if self.steady_shift.is_some() && (!fixed.is_empty() || attached.is_some()) {
            let full = 256 + 2 * side;
            let (top, w): (&mut [u32], usize) = match wide.as_deref_mut() {
                Some(wide) => (wide, full),
                None => (&mut pixels[..256 * 192], 256),
            };
            let fixed_mask = fit(&fixed, w);
            if let (Some(a), Some(world)) = (attached, self.steady_shift) {
                self.attached.prepare(
                    a,
                    top,
                    unlit.as_deref(),
                    light_maps,
                    w,
                    world,
                    self.steady_origin,
                );
            }
            if !fixed.is_empty() {
                let none = vec![0; w * 192];
                let img = split_layer(top, &fixed_mask, &none, w, k);
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
                let why = lighting_unavailable();
                changed |= ui
                    .add_enabled(why.is_none(), egui::Checkbox::new(&mut v.lighting, "Revamped lighting"))
                    .on_hover_text("Light and shade in the overworld, a soft colour grade elsewhere")
                    .on_disabled_hover_text(why.as_deref().unwrap_or_default())
                    .changed();
                changed |= ui
                    .add_enabled(v.lighting && why.is_none(), egui::Checkbox::new(&mut v.shadows, "Dynamic shadows"))
                    .on_hover_text(SHADOWS_HINT)
                    .on_disabled_hover_text(why.as_deref().unwrap_or("With the revamped lighting on"))
                    .changed();
                if let Some(why) = &why {
                    ui.add(
                        egui::Label::new(egui::RichText::new(why).color(egui::Color32::from_rgb(255, 120, 110)))
                            .wrap(),
                    );
                }
                changed |= ui
                    .checkbox(&mut v.steady, "Smooth camera")
                    .on_hover_text("The camera glides smoothly as you move, the town's pixels kept still; off: the game's own camera, which shimmers")
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
    /// The window is opening again on a new GPU device (KNOWN_RISKS #138): the textures made for
    /// the old one are gone, so they are made again; the lighting's thread starts again for the
    /// new window.
    pub fn forget_textures(&mut self) {
        self.texture = None;
        self.textbox = None;
        self.wide = None;
        self.choice_tex.clear();
        self.choice_words.clear();
        self.mega_icon = None;
        self.shoes_icon = None;
        self.attached.texture = None;
        self.attached.made = None;
        self.fixed.texture = None;
        self.light_thread = None;
        // The lighting tries the GPU again (its own device may have gone with the window's).
        self.no_gpu = false;
        if let Some(w) = self.world.as_mut() {
            w.forget_textures();
        }
    }

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

    /// The running shoes' button in the start menu's pop-up (the touch screen `r`), where the
    /// game's own sits on the field's touch screen and as the game last drew it there (orange:
    /// running; grey: walking): the game hides its own while the menu is open, and the one-screen
    /// layout shows the touch screen only then, so it couldn't be reached. A click closes the menu
    /// and taps the game's button (`ShoesTap`). A panel of its own: the click isn't a touch.
    fn running_toggle(&mut self, ctx: &egui::Context, r: egui::Rect, on: bool) {
        let at = |x: usize, y: usize| {
            r.min + egui::vec2(x as f32 / 256.0 * r.width(), y as f32 / 192.0 * r.height())
        };
        let (x0, y0, x1, y1) = SHOES_BUTTON;
        let area = egui::Rect::from_min_max(at(x0, y0), at(x1, y1));
        let icon = self.shoes_icon.as_ref().map(|t| t.id());
        let mut clicked = false;
        egui::Area::new(egui::Id::new("running-shoes"))
            .order(egui::Order::Foreground)
            .fixed_pos(area.min)
            .show(ctx, |ui| {
                let hint = if on {
                    "Running (click to walk instead)"
                } else {
                    "Walking (click to run instead)"
                };
                let resp = match icon {
                    Some(id) => {
                        let (rect, resp) =
                            ui.allocate_exact_size(area.size(), egui::Sense::click());
                        let full =
                            egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
                        ui.painter().image(id, rect, full, egui::Color32::WHITE);
                        if resp.hovered() {
                            ui.painter().rect_filled(
                                rect,
                                0.0,
                                egui::Color32::from_white_alpha(28),
                            );
                        }
                        resp
                    }
                    // Before the game has drawn its own (the menu opened right away).
                    None => theme::button(
                        ui,
                        if on { "Running: on" } else { "Running: off" },
                        if on { Kind::Primary } else { Kind::Neutral },
                        area.size(),
                        true,
                    ),
                };
                clicked = resp.on_hover_text(hint).clicked();
            });
        if clicked && self.shoes_tap.is_none() {
            self.shoes_tap = Some((ShoesTap::Closing, Instant::now()));
        }
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
        let fill_sides = |screen: egui::Rect, rows: &[Option<EdgeFill>; 2]| {
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
                    #[cfg(feature = "dev-hooks")]
                    let b = &crate::dev_hooks::hud_text(b);
                    if let Some(at) = hud::draw(ui, frame, b, mega) {
                        self.one.tap = Some((at, Instant::now()));
                    }
                } else if let Some(tb) = &self.textbox {
                    hud::draw_text_band(ui, frame, tb);
                }
                // A game window reaching into the band (the stats box after a level-up): over
                // the band, from the picture shown, as the game draws it over its text box.
                if let Some([x0, y0, x1, y1]) = self.band_window {
                    // From a row above the band: its gold top line is wider than a pixel.
                    let y0 = (y0 as f32).max(hud::SCENE_ROWS - 1.0);
                    let r = egui::Rect::from_min_max(
                        egui::pos2(middle.left() + x0 as f32 * s, frame.top() + y0 * s),
                        egui::pos2(middle.left() + x1 as f32 * s, frame.top() + y1 as f32 * s),
                    );
                    let (tex, uv) = match &wide {
                        Some(w) => (
                            w.id(),
                            egui::Rect::from_min_max(
                                egui::pos2(
                                    (r.left() - frame.left()) / frame.width(),
                                    (r.top() - frame.top()) / frame.height(),
                                ),
                                egui::pos2(
                                    (r.right() - frame.left()) / frame.width(),
                                    (r.bottom() - frame.top()) / frame.height(),
                                ),
                            ),
                        ),
                        None => (
                            t.id(),
                            egui::Rect::from_min_max(
                                egui::pos2(x0 as f32 / 256.0, y0 / 384.0),
                                egui::pos2(x1 as f32 / 256.0, y1 as f32 / 384.0),
                            ),
                        ),
                    };
                    p.image(tex, r, uv, egui::Color32::WHITE);
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
                        let slid = shift.is_some();
                        self.attached
                            .draw(ui.ctx(), &p, frame, true, cols, slid, side);
                        self.fixed
                            .draw(&p, frame, true, cols, shift.is_some(), side);
                    }
                    None => {
                        let locked = shift.map(|s| pixel_lock(s, origin, middle, 256.0, 0.0, ppp));
                        let (r, uv) =
                            slide(middle, screen_uvs(self.prescale).0, locked, 256.0, true);
                        p.image(t.id(), r, uv, egui::Color32::WHITE);
                        let slid = shift.is_some();
                        self.attached
                            .draw(ui.ctx(), &p, middle, false, 256.0, slid, 0.0);
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
        self.one.bubble = None;
        if modal {
            // The touch screen over the dimmed picture, leaving the picture's edges showing (the
            // first Pokémon's choice: darker, as the smaller touch screen leaves more of it).
            let starter = self.one.choosing_starter(scene);
            let dim = if starter { 200 } else { 150 };
            p.rect_filled(frame, 0.0, egui::Color32::from_black_alpha(dim));
            let r = if starter {
                // Smaller and lower, under the Pokémon's bubble.
                let size = middle.size() * 0.66;
                egui::Rect::from_min_size(
                    egui::pos2(
                        frame.center().x - size.x / 2.0,
                        frame.bottom() - frame.height() * 0.03 - size.y,
                    ),
                    size,
                )
            } else {
                egui::Rect::from_center_size(frame.center(), middle.size() * 0.86)
            };
            p.rect_filled(r.expand(6.0), 6.0, egui::Color32::from_black_alpha(120));
            p.image(t.id(), r, screen_uvs(self.prescale).1, egui::Color32::WHITE);
            p.rect_stroke(
                r.expand(1.5),
                3.0,
                egui::Stroke::new(3.0_f32, theme::EDGE),
                egui::StrokeKind::Outside,
            );
            self.bottom = r;
            if starter {
                self.one.bubble = Some(starter_bubble(&p, &t, self.prescale, r));
            }
            // Only over the start menu's own panel: its Save screen takes the touch screen (the
            // user, 2026-11-18: the button sat on top of it).
            if scene == Scene::StartMenu
                && self.start_panel
                && let Some(on) = self.running_shoes
            {
                self.running_toggle(ui.ctx(), r, on);
            }
        } else if scene == Scene::Overworld && self.game_textbox && !self.choices.is_empty() {
            self.draw_choices(ui, middle);
        }
    }

    /// The open menu's choices in a white box at the right of the top screen (`top`), over the
    /// game's text box: the ROM's words (`self.menu_words`) in the text boxes' font and colours,
    /// a ▶ at the one the touch screen shows selected (`self.choices`). Only when the two agree
    /// on how many there are. A click taps that choice's button.
    fn draw_choices(&mut self, ui: &egui::Ui, top: egui::Rect) {
        let n = self.choices.len();
        let Some(font) = self.game_font.clone() else {
            return;
        };
        if n == 0 || self.menu_words.len() != n {
            return;
        }
        if self.choice_words != self.menu_words || self.choice_tex.len() < n {
            for (k, words) in self.menu_words.iter().enumerate() {
                let (w, px) = font.draw(words);
                let mut img = egui::ColorImage::new([w, font.height], egui::Color32::TRANSPARENT);
                for (out, v) in img.pixels.iter_mut().zip(px) {
                    *out = match v {
                        1 => hud::TEXT_INK,
                        2 => hud::TEXT_SHADOW,
                        _ => egui::Color32::TRANSPARENT,
                    };
                }
                let nearest = egui::TextureOptions::NEAREST;
                match self.choice_tex.get_mut(k) {
                    Some(t) => t.set(img, nearest),
                    None => self.choice_tex.push(ui.ctx().load_texture(
                        format!("choice-{k}"),
                        img,
                        nearest,
                    )),
                }
            }
            self.choice_words = self.menu_words.clone();
        }
        let s = top.height() / 192.0;
        let sizes: Vec<egui::Vec2> = self.choice_tex[..n].iter().map(|t| t.size_vec2()).collect();
        // As the game's own menu windows: 8 pixels in from the frame, a line per choice.
        let (pad, cursor) = (8.0, 10.0);
        let line = font.height as f32;
        let text = sizes.iter().map(|v| v.x).fold(0.0, f32::max);
        let size = egui::vec2(2.0 * pad + cursor + text, 2.0 * pad - 4.0 + line * n as f32) * s;
        let rect = egui::Rect::from_min_size(
            egui::pos2(
                top.right() - 6.0 * s - size.x,
                top.top() + 142.0 * s - size.y,
            ),
            size,
        );
        let p = ui.painter();
        p.rect_filled(rect, 3.0 * s, egui::Color32::from_rgb(251, 251, 251));
        p.rect_stroke(
            rect,
            3.0 * s,
            egui::Stroke::new(2.0 * s, egui::Color32::from_rgb(72, 72, 80)),
            egui::StrokeKind::Inside,
        );
        let full = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
        for k in 0..n {
            let c = self.choices[k];
            let y = rect.top() + (pad - 2.0 + line * k as f32) * s;
            let label = egui::Rect::from_min_size(
                egui::pos2(rect.left() + (pad + cursor) * s, y),
                sizes[k] * s,
            );
            p.image(self.choice_tex[k].id(), label, full, egui::Color32::WHITE);
            if c.selected {
                let (x, m) = (rect.left() + pad * s, y + (line / 2.0 - 1.0) * s);
                p.add(egui::Shape::convex_polygon(
                    vec![
                        egui::pos2(x, m - 4.0 * s),
                        egui::pos2(x + 6.0 * s, m),
                        egui::pos2(x, m + 4.0 * s),
                    ],
                    hud::TEXT_INK,
                    egui::Stroke::NONE,
                ));
            }
            let row = egui::Rect::from_min_max(
                egui::pos2(rect.left(), y),
                egui::pos2(rect.right(), y + line * s),
            );
            if ui
                .interact(
                    row,
                    egui::Id::new(("touch-choice", k)),
                    egui::Sense::click(),
                )
                .clicked()
            {
                let at = (((c.x0 + c.x1) / 2) as u16, ((c.y0 + c.y1) / 2) as u16);
                self.one.tap = Some((at, Instant::now()));
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
                if self.config.video.layout == Layout::OneScreen {
                    if b.menu_words() != self.menu_words.as_slice() {
                        self.menu_words = b.menu_words().to_vec();
                    }
                    if self.game_font.is_none() {
                        self.game_font = b.game_font();
                    }
                }
                let one = self.config.video.layout == Layout::OneScreen;
                self.one.starter = one && naming.is_none() && b.choosing_starter();
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
            if std::mem::take(&mut self.tab) && !self.blocked() {
                let shown = self.bottom != egui::Rect::NOTHING;
                self.one.by_hand = Some(!shown);
                // The keys go to the game again (no button keeps the keyboard).
                ctx.memory_mut(|m| m.stop_text_input());
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
                let both = self.pokegear_hold > 0;
                if self.config.video.layout == Layout::OneScreen && !both {
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
                        let slid = shift.is_some();
                        self.attached.draw(ctx, p, top, false, 256.0, slid, 0.0);
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
        if minimized(ctx) {
            // Nothing to see: a few frames a second keep the game's events going (the emulator
            // runs on its own thread), with far less for the GPU to do (KNOWN_RISKS #138).
            ctx.request_repaint_after(MINIMIZED_FRAME);
        } else {
            // The emulator runs on its own clock; draw whenever the display can.
            ctx.request_repaint();
        }
        action
    }

    /// Tab (shows or hides the touch screen in the one-screen layout) is the game's while no
    /// menu is open: taken out of the window's input before egui sees it, as egui moves its
    /// keyboard focus to the next button on Tab, and while a button had it the game's keys were
    /// ignored until a click on the picture (the user). Called before each frame.
    pub fn take_tab(&mut self, raw: &mut egui::RawInput) {
        if self.blocked() || self.config.video.layout != Layout::OneScreen {
            return;
        }
        raw.events.retain(|e| match e {
            egui::Event::Key {
                key: egui::Key::Tab,
                pressed,
                repeat,
                ..
            } => {
                if *pressed && !*repeat {
                    self.tab = true;
                }
                false
            }
            _ => true,
        });
    }

    /// Leaves the world and writes the save (the window closes).
    pub fn shutdown(&mut self) {
        self.stop();
    }
}

impl eframe::App for GameApp {
    fn update(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        if let Some(rs) = frame.wgpu_render_state() {
            crate::gpu_watch::watch(rs);
            crate::composite::install(rs);
        }
        let _ = self.ui(ctx);
    }

    fn raw_input_hook(&mut self, _ctx: &egui::Context, raw: &mut egui::RawInput) {
        if let Some(extra) = self.extra_input.as_mut() {
            extra(raw);
        }
        self.take_tab(raw);
    }

    fn on_exit(&mut self) {
        self.stop();
    }
}

impl Drop for GameApp {
    fn drop(&mut self) {
        self.stop();
    }
}

/// The window's frame interval while it's minimized.
const MINIMIZED_FRAME: Duration = Duration::from_millis(250);

fn minimized(ctx: &egui::Context) -> bool {
    ctx.input(|i| i.viewport().minimized == Some(true))
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
    /// Where in the picture it goes (DS pixels: left, top, right, bottom), when not all of it.
    rect: Option<[f32; 4]>,
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
        match self.rect {
            // The picture's point (DS pixels) as the whole picture shows it.
            Some([x0, y0, x1, y1]) => {
                let at = |x: f32, y: f32| {
                    egui::pos2(
                        r.min.x + (x / cols - uv.min.x) / uv.width() * r.width(),
                        r.min.y + (y / 192.0 - uv.min.y) / uv.height() * r.height(),
                    )
                };
                let to = egui::Rect::from_min_max(at(x0, y0), at(x1, y1));
                p.image(t.id(), to, full, egui::Color32::WHITE);
            }
            None => {
                p.image(t.id(), r, uv, egui::Color32::WHITE);
            }
        }
    }
}

/// The steady overworld's two pictures (UC12, `Nds::attached`) put together round the player at
/// the screen's own pixels: the world where the picture shows it, the attached picture (the
/// player, their shadow, a mount, a follower) where it truly is (each landed on the screen's
/// pixels as the picture is: `pixel_lock`), a fraction of a pixel off the world's pixels while it
/// moves and none at rest. As the DS puts them together in one picture: the nearer shows; the
/// attached picture's outline where the world beside it is farther; a world's edge over it
/// blended with it. Lit as the top picture (each DS pixel's gain), transparent where the top
/// screen doesn't show the 3D as drawn (2D, fades: the picture's own pixels show there).
#[derive(Default)]
struct AttachedView {
    data: Option<AttachedData>,
    texture: Option<egui::TextureHandle>,
    /// What the texture was made for: the frame's data, the place, the pixels per point.
    made: Option<(usize, [f32; 4], f32)>,
    frames: usize,
}

struct AttachedData {
    a: goldlink_core::Attached,
    /// The lighting's gain per DS pixel (`w` x 192; empty: none), for the CPU's composite.
    gain: Vec<[f32; 3]>,
    /// The crop the GPU's composite reads (UC14), when it's ready.
    gpu: Option<Arc<crate::composite::Frame>>,
    w: usize,
    world: [f32; 2],
    world_origin: [f64; 2],
    /// Where the attached picture has something (DS pixels: left, top, right, bottom).
    bbox: [usize; 4],
}

impl AttachedView {
    #[allow(clippy::too_many_arguments)]
    fn prepare(
        &mut self,
        a: goldlink_core::Attached,
        top: &[u32],
        unlit: Option<&[u32]>,
        light_maps: Option<goldlink_light::LightMaps>,
        w: usize,
        world: [f32; 2],
        world_origin: [f64; 2],
    ) {
        let cols = a.plain.len() / 192;
        if cols < w {
            return;
        }
        let off = (cols - w) / 2;
        let (mut x0, mut y0, mut x1, mut y1) = (usize::MAX, usize::MAX, 0, 0);
        for y in 0..192 {
            for x in 0..w {
                if a.att[y * cols + x + off] >> 24 != 0 {
                    (x0, y0, x1, y1) = (x0.min(x), y0.min(y), x1.max(x + 1), y1.max(y + 1));
                }
            }
        }
        if x0 == usize::MAX {
            return;
        }
        let ch = |p: u32, sh: u32| ((p >> sh) & 255) as f32;
        // The CPU's composite's gain (the GPU's lights each source as itself).
        let gain = match unlit.filter(|_| !crate::composite::ready()) {
            Some(u) => (0..w * 192)
                .map(|k| [16, 8, 0].map(|sh| (ch(top[k], sh) + 1.0) / (ch(u[k], sh) + 1.0)))
                .collect(),
            None => Vec::new(),
        };
        self.frames += 1;
        let gpu = crate::composite::ready().then(|| {
            Arc::new(gpu_frame(
                &a,
                w,
                [x0, y0, x1, y1],
                light_maps.as_ref(),
                self.frames as u64,
            ))
        });
        self.data = Some(AttachedData {
            a,
            gain,
            gpu,
            w,
            world,
            world_origin,
            bbox: [x0, y0, x1, y1],
        });
    }

    /// Over the picture drawn into `dst` (`cols` DS pixels across, the screen's column 0 at
    /// `side`), slid (`slid`) as the picture is: unslid the picture already shows the two as the
    /// DS puts them together.
    #[allow(clippy::too_many_arguments)]
    fn draw(
        &mut self,
        ctx: &egui::Context,
        p: &egui::Painter,
        dst: egui::Rect,
        wide: bool,
        cols: f32,
        slid: bool,
        side: f64,
    ) {
        let Some(d) = self.data.as_ref().filter(|d| slid && (d.w != 256) == wide) else {
            return;
        };
        let ppp = ctx.pixels_per_point();
        let full = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
        let wl = pixel_lock(d.world, d.world_origin, dst, cols, side, ppp);
        // At rest (the same shift as the world's) on the world's pixels exactly: both land on the
        // screen's pixels each by its own reference, and at a scale that isn't whole they came out
        // a fraction of a screen pixel apart (a line of the sprite a row late). Moving, by its own:
        // it stays put on screen while the world slides (by the world's it shook by a screen pixel).
        let al = if d.a.shift == d.world {
            wl
        } else {
            pixel_lock(d.a.shift, d.a.origin, dst, cols, side, ppp)
        };
        let (_, uw) = slide(dst, full, Some(wl), cols, false);
        let (_, ua) = slide(dst, full, Some(al), cols, false);
        // The attached picture's point less the world's at the same place on screen.
        let off = [(uw.min.x - ua.min.x) * cols, (uw.min.y - ua.min.y) * 192.0];
        let m = 2.0 + off[0].abs().max(off[1].abs()).ceil();
        let [bx0, by0, bx1, by1] = d.bbox.map(|v| v as f32);
        let (rx0, ry0) = ((bx0 - m).max(0.0), (by0 - m).max(0.0));
        let (rx1, ry1) = ((bx1 + m).min(d.w as f32), (by1 + m).min(192.0));
        // The picture's point (DS pixels) on screen (points), and back.
        let sx = |x: f32| dst.min.x + (x / cols - uw.min.x) / uw.width() * dst.width();
        let sy = |y: f32| dst.min.y + (y / 192.0 - uw.min.y) / uw.height() * dst.height();
        let wx = |s: f32| (uw.min.x + (s - dst.min.x) / dst.width() * uw.width()) * cols;
        let wy = |s: f32| (uw.min.y + (s - dst.min.y) / dst.height() * uw.height()) * 192.0;
        let (px0, py0) = (
            (sx(rx0) * ppp).floor() as i64,
            (sy(ry0) * ppp).floor() as i64,
        );
        let (px1, py1) = ((sx(rx1) * ppp).ceil() as i64, (sy(ry1) * ppp).ceil() as i64);
        if px1 <= px0 || py1 <= py0 {
            return;
        }
        let (pw, ph) = ((px1 - px0) as usize, (py1 - py0) as usize);
        let to = egui::Rect::from_min_max(
            egui::pos2(px0 as f32 / ppp, py0 as f32 / ppp),
            egui::pos2(px1 as f32 / ppp, py1 as f32 / ppp),
        );
        // On the window's GPU (UC14): each screen pixel's world point is linear in its physical
        // pixel (the image's `wx`, `wy` at the pixel's centre).
        if let Some(frame) = d.gpu.clone().filter(|_| crate::composite::ready()) {
            let cols_plain = d.a.plain.len() / 192;
            let (ax, bx) = (wx(0.0), wx(1.0 / ppp) - wx(0.0));
            let (ay, by) = (wy(0.0), wy(1.0 / ppp) - wy(0.0));
            let draw = crate::composite::Draw {
                frame,
                map: [ax, bx, ay, by],
                off,
                w: d.w as i32,
                o: ((cols_plain - d.w) / 2) as i32,
                from: (goldlink_core::Nds::MAX_WIDE_SIDE - d.a.side) as i32,
            };
            p.add(draw.callback(to));
            return;
        }
        let key = (self.frames, [px0 as f32, py0 as f32, off[0], off[1]], ppp);
        if self.made != Some(key) || self.texture.is_none() {
            let img = d.image(
                pw,
                ph,
                |i| wx((px0 as f32 + i as f32 + 0.5) / ppp),
                |j| wy((py0 as f32 + j as f32 + 0.5) / ppp),
                off,
            );
            match &mut self.texture {
                Some(t) if t.size() == img.size => t.set(img, egui::TextureOptions::NEAREST),
                _ => {
                    self.texture =
                        Some(ctx.load_texture("attached", img, egui::TextureOptions::NEAREST))
                }
            }
            self.made = Some(key);
        }
        if let Some(t) = &self.texture {
            p.image(t.id(), to, full, egui::Color32::WHITE);
        }
    }
}

impl AttachedData {
    /// `pw` x `ph` screen pixels, the world's point (DS pixels) of each column and row's centre
    /// `xs(i)`, `ys(j)`; the attached picture's is that less `off`. The two put together by the
    /// renderer's own `Composite` (patch 0026), as the DS would draw them in one picture.
    fn image(
        &self,
        pw: usize,
        ph: usize,
        xs: impl Fn(usize) -> f32,
        ys: impl Fn(usize) -> f32,
        off: [f32; 2],
    ) -> egui::ColorImage {
        let a = &self.a;
        let w = self.w;
        let cols = a.plain.len() / 192;
        let o = (cols - w) / 2;
        let cx: Vec<f32> = (0..pw).map(&xs).collect();
        // Each pair of DS pixels (the world's, the attached picture's) once: a DS pixel spans
        // several screen pixels, all with the same few pairs.
        let mut pairs: std::collections::HashMap<[i32; 4], usize> =
            std::collections::HashMap::new();
        let (mut world, mut att, mut at) = (Vec::new(), Vec::new(), Vec::new());
        for jy in 0..ph {
            let yf = ys(jy);
            if yf < 0.0 || yf >= 192.0 {
                continue;
            }
            for (ix, &xf) in cx.iter().enumerate() {
                if xf < 0.0 || xf as usize >= w {
                    continue;
                }
                let (wx, wy) = (xf as usize, yf as usize);
                if a.plain[wy * cols + wx + o] == 0 {
                    continue;
                }
                let key = [
                    (wx + o) as i32,
                    wy as i32,
                    (xf - off[0]).floor() as i32 + o as i32,
                    (yf - off[1]).floor() as i32,
                ];
                let next = pairs.len();
                let n = *pairs.entry(key).or_insert(next);
                if n == next {
                    world.extend([key[0], key[1]]);
                    att.extend([key[2], key[3]]);
                }
                at.push((jy * pw + ix, wy * w + wx, n));
            }
        }
        let mut colours = Vec::new();
        a.pictures
            .composite(a.side, &world, &att, off, &mut colours);
        let mut out = vec![egui::Color32::TRANSPARENT; pw * ph];
        for &(pix, k, n) in &at {
            let c = colours[n];
            let c = match self.gain.get(k) {
                Some(g) => scaled(c, *g),
                None => c,
            };
            out[pix] = egui::Color32::from_rgb((c >> 16) as u8, (c >> 8) as u8, c as u8);
        }
        egui::ColorImage {
            size: [pw, ph],
            pixels: out,
        }
    }
}

/// `c` (0x..RRGGBB) times a gain per channel.
fn scaled(c: u32, gain: [f32; 3]) -> u32 {
    let ch = |sh: u32, g: f32| ((((c >> sh) & 255) as f32 * g).min(255.0) as u32) << sh;
    ch(16, gain[0]) | ch(8, gain[1]) | ch(0, gain[2])
}

/// A layer of the steady overworld's picture (UC11), `w` across, 192 rows, each pixel `k` times as
/// large, drawn over the slid picture (with its own shift, or none): where `mask` is 1, `top`'s
/// pixel, opaque; where it's 2, `trans`'s (0xAARRGGBB: a shadow the picture doesn't have); the
/// rest transparent. In `top` (the picture that's slid) the 1s take the world under them (with
/// `under`: `trans` there, the renderer's, times the lighting's gain) or else the colour of the
/// nearest scene pixel (neither this layer's 1s nor `other`), filled in from the edge: where the
/// shifts differ (about half a pixel at most) the world shows there, never this layer's pixels a
/// second time at another shift.
fn split_layer(top: &mut [u32], mask: &[u8], other: &[u8], w: usize, k: usize) -> egui::ColorImage {
    let colour = |p: u32| egui::Color32::from_rgb((p >> 16) as u8, (p >> 8) as u8, p as u8);
    let h = 192;
    let mut layer = vec![egui::Color32::TRANSPARENT; w * h];
    for (i, px) in layer.iter_mut().enumerate() {
        if mask[i] == 1 {
            *px = colour(top[i]);
        }
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

/// A side of a screen continued beside it (`edge_rows`): each row's colour, and the pixels that
/// close something reaching the screen's edge (its outline mirrored), from the edge outward.
#[derive(Debug, Clone, PartialEq)]
struct EdgeFill {
    rows: Vec<egui::Color32>,
    caps: Vec<Vec<egui::Color32>>,
}

/// Fills the frame beside a shown screen (`screen`, inside `frame`), left and right, row by row
/// with `rows` (`edge_rows`: none leaves that side as it is, black), then the caps beside the edge.
fn fill_beside(
    p: &egui::Painter,
    frame: egui::Rect,
    screen: egui::Rect,
    rows: &[Option<EdgeFill>; 2],
) {
    // Each fill runs a point under the screen (drawn over it next), so no seam of the black
    // behind shows where the two meet at a fraction of a pixel.
    for (side, (fill, x0, x1)) in [
        (&rows[0], frame.left(), screen.left() + 1.0),
        (&rows[1], screen.right() - 1.0, frame.right()),
    ]
    .into_iter()
    .enumerate()
    {
        let Some(fill) = fill else { continue };
        let rows = &fill.rows;
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
        // The caps on whole screen pixels (a fraction blurs a one-pixel outline).
        let px = screen.width() / 256.0;
        let ppp = p.ctx().pixels_per_point();
        let snap = |v: f32| (v * ppp).round() / ppp;
        // Column by column, each run of rows of one colour as one rectangle (no seams between
        // rows).
        let width = fill.caps.iter().map(Vec::len).max().unwrap_or(0);
        for i in 0..width {
            let (a, b) = if side == 0 {
                (
                    screen.left() - (i + 1) as f32 * px,
                    screen.left() - i as f32 * px,
                )
            } else {
                (
                    screen.right() + i as f32 * px,
                    screen.right() + (i + 1) as f32 * px,
                )
            };
            let at = |y: usize| fill.caps[y].get(i).copied();
            let mut y = 0;
            while y < fill.caps.len() {
                let Some(c) = at(y) else {
                    y += 1;
                    continue;
                };
                let end = (y..fill.caps.len())
                    .find(|&e| at(e) != Some(c))
                    .unwrap_or(fill.caps.len());
                let r = egui::Rect::from_min_max(
                    egui::pos2(snap(a), snap(screen.top() + y as f32 * row_h)),
                    egui::pos2(snap(b), snap(screen.top() + end as f32 * row_h)),
                );
                p.rect_filled(r, 0.0, c);
                y = end;
            }
        }
    }
}

/// A screen's left and right edges, continued beside it when the wider picture isn't
/// shown: per side, each row's colour when its 4 edge pixels are one colour (a flat screen, a
/// fade, a bar across the screen), the nearest such row's where they aren't (a text box reaching
/// the edge), or none (black) when most rows' aren't (the edge is a picture). Something drawn
/// over the background up to the edge (rows of another colour, the background showing again
/// further in: Oak's speech's "Touch" button) isn't stretched across the side: its rows take the
/// background's colour and it is closed at the edge with its own outline, mirrored.
fn edge_rows(top: &[u32]) -> [Option<EdgeFill>; 2] {
    let colour = |p: u32| egui::Color32::from_rgb((p >> 16) as u8, (p >> 8) as u8, p as u8);
    let at = |x: usize, y: usize| top[y * 256 + x] & 0xFF_FFFF;
    let side = |left: bool| -> Option<EdgeFill> {
        // The outermost 4 columns (the name keyboard's panel comes within 8 of the right edge:
        // that side stayed black while the left one continued the background).
        let cols = if left { 0..4 } else { 252..256 };
        let mut rows: Vec<Option<u32>> = (0..192)
            .map(|y| {
                let first = at(cols.start, y);
                cols.clone().all(|x| at(x, y) == first).then_some(first)
            })
            .collect();
        if rows.iter().filter(|r| r.is_some()).count() * 2 < rows.len() {
            return None; // a picture
        }
        // A row whose edge has no one colour (a text box reaching the screen's edge, Oak's
        // speech: the whole side came out black) takes the nearest row's that has one.
        let known = rows.clone();
        for (i, r) in rows.iter_mut().enumerate() {
            if r.is_none() {
                *r = (1..192).find_map(|d| {
                    let up = i.checked_sub(d).and_then(|j| known[j]);
                    up.or_else(|| known.get(i + d).copied().flatten())
                });
            }
        }
        let mut rows: Vec<u32> = rows.into_iter().map(Option::unwrap_or_default).collect();
        let mut caps = vec![Vec::new(); 192];
        // The background: the side's commonest colour.
        let mut count = std::collections::HashMap::new();
        for &c in &rows {
            *count.entry(c).or_insert(0) += 1;
        }
        let bg = count.into_iter().max_by_key(|&(_, n)| n).map(|(c, _)| c)?;
        // Inward from the edge, the first column of the background on row y.
        let inner = |y: usize| -> Option<usize> {
            (1..200).find(|&d| at(if left { d } else { 255 - d }, y) == bg)
        };
        let mut y = 0;
        while y < 192 {
            if rows[y] == bg {
                y += 1;
                continue;
            }
            let end = (y..192).find(|&e| rows[e] == bg).unwrap_or(192);
            let found: Vec<Option<usize>> = (y..end).map(inner).collect();
            if found.iter().filter(|f| f.is_some()).count() * 2 > found.len() {
                // Closed with its own frame: its border inward from the top edge (and from the
                // bottom edge) at the screen's edge column, up to its inside, becomes the end's
                // columns outward from the screen's edge; rows inside the top or bottom border
                // take that border's colour across the end (the corners).
                let ex = if left { 0 } else { 255 };
                let h = end - y;
                let profile = |from: usize, step: isize| -> Vec<u32> {
                    let inside = at(ex, (from as isize + step * (h as isize / 4)) as usize);
                    (0..h.min(6))
                        .map(|i| at(ex, (from as isize + step * i as isize) as usize))
                        .take_while(|&c| c != inside)
                        .collect()
                };
                let (top_frame, bottom_frame) = (profile(y, 1), profile(end - 1, -1));
                let k = top_frame.len().max(bottom_frame.len());
                for row in y..end {
                    rows[row] = bg;
                    let (t, b) = (row - y, end - 1 - row);
                    let (frame, d) = if t <= b {
                        (&top_frame, t)
                    } else {
                        (&bottom_frame, b)
                    };
                    caps[row] = (0..k)
                        .map(|j| {
                            let q = k - 1 - j; // 0: the outermost column
                            let c = if d < q && d < frame.len() {
                                frame[d]
                            } else if q < frame.len() {
                                frame[q]
                            } else {
                                at(ex, row)
                            };
                            colour(c)
                        })
                        .collect();
                }
            }
            y = end;
        }
        Some(EdgeFill {
            rows: rows.into_iter().map(colour).collect(),
            caps,
        })
    };
    [side(true), side(false)]
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

/// The lighting (UC11): the overworld lit on the top screen (the wider picture when it's
/// shown, else the screen), everything else (battles, menus, the touch screen) graded.
fn light(
    lighting: &mut goldlink_light::Lighting,
    shadows: bool,
    pixels: &mut [u32],
    wide: Option<&mut [u32]>,
    side: usize,
    light: Option<&goldlink_runtime::LightFrame>,
    attached: Option<&goldlink_core::Attached>,
) {
    use goldlink_light::{Phase, Place, Scene};
    // Where the attached picture shows on top, opaque, on the lighting's columns (UC13): its
    // pixel is there and the picture shows its colour (not something drawn in front of the
    // player: a roof's pixels lit as the player's left a ghost of them on it, the user's look).
    let width = goldlink_core::LightBuffers::WIDTH;
    let (shown, shown_w): (&[u32], usize) = match wide.as_deref() {
        Some(w) => (w, 256 + 2 * side),
        None => (&pixels[..256 * 192], 256),
    };
    let player: Vec<u8> = attached.map_or_else(Vec::new, |a| {
        let cols = a.plain.len() / 192;
        let off = cols as isize - width as isize;
        let soff = shown_w as isize - width as isize;
        let near = |p: u32, q: u32| {
            [16, 8, 0]
                .iter()
                .all(|&s| ((p >> s) & 255).abs_diff((q >> s) & 255) <= 12)
        };
        (0..width * 192)
            .map(|i| {
                let y = i / width;
                let (x, sx) = (
                    (i % width) as isize + off / 2,
                    (i % width) as isize + soff / 2,
                );
                let at = (x >= 0 && (x as usize) < cols).then(|| a.att[y * cols + x as usize]);
                let on =
                    (sx >= 0 && (sx as usize) < shown_w).then(|| shown[y * shown_w + sx as usize]);
                u8::from(match (at, on) {
                    (Some(c), Some(s)) => c >> 24 >= 0xF0 && near(c, s),
                    _ => false,
                })
            })
            .collect()
    });
    let hour = light.map_or_else(|| chrono::Timelike::hour(&chrono::Local::now()), |l| l.hour);
    let phase = Phase::lighting_of_hour(hour);
    let world = light.filter(|l| l.overworld());
    // The picture's polygons, as the lighting takes them: the cast shadows' (UC13, with the
    // Shadows setting) and the cards' (sprites aren't light sources).
    let (polys, verts): (Vec<_>, Vec<_>) = world.map_or_else(Default::default, |l| {
        let g = &l.buffers.geometry;
        (
            g.polys
                .iter()
                .map(|p| goldlink_light::ShadowPoly {
                    first_vert: p.first_vert,
                    num_verts: p.num_verts,
                    attr: p.attr,
                    tex_param: p.tex_param,
                    tex: p.tex,
                    flags: p.flags,
                    object: p.object,
                })
                .collect(),
            g.verts
                .iter()
                .map(|v| goldlink_light::ShadowVert {
                    clip: v.clip,
                    st: v.st,
                    screen: v.screen,
                })
                .collect(),
        )
    });
    let texs: Vec<_> = world.map_or_else(Vec::new, |l| {
        l.buffers
            .geometry
            .texs
            .iter()
            .map(|t| goldlink_light::ShadowTex {
                width: t.width,
                height: t.height,
                offset: t.offset,
            })
            .collect()
    });
    let scene = world.map(|l| {
        (
            Scene {
                depth: &l.buffers.depth,
                layer: &l.buffers.layer,
                proj: l.buffers.proj,
                width: goldlink_core::LightBuffers::WIDTH,
                fade: &l.buffers.fade,
                geometry: goldlink_light::Geometry {
                    polys: &polys,
                    verts: &verts,
                    texs: &texs,
                    alpha: &l.buffers.geometry.alpha,
                },
                hours: l.hour as f32 + l.minute as f32 / 60.0,
                attached: &player,
                sprite_tint: l.buffers.sprite_tint,
                shadows,
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
                    lighting.overworld(wide, w, middle - side, s, *place, phase);
                }
                None => lighting.grade_faded(wide, w, phase, fade),
            }
            // The DS screen alone shows where the wider picture doesn't (menus, the title).
            lighting.grade_faded(top, 256, phase, fade);
        }
        None => match &scene {
            Some((s, place)) => {
                lighting.overworld(top, 256, middle, s, *place, phase);
            }
            None => lighting.grade_faded(top, 256, phase, fade),
        },
    }
    lighting.grade(bottom, 256, 192, phase);
}

/// A frame taken from the emulation, as the window shows it (lit on the lighting thread).
struct Shown {
    pixels: Vec<u32>,
    wide: Option<Vec<u32>>,
    side: usize,
    light: Option<goldlink_runtime::LightFrame>,
    fixed: Vec<u8>,
    attached: Option<goldlink_core::Attached>,
    steady_shift: Option<[f32; 2]>,
    steady_origin: [f64; 2],
    edge_fill: [[Option<EdgeFill>; 2]; 2],
    /// The top picture before the lighting, with the attached picture: the two put together are
    /// lit as the top picture is (each DS pixel's gain).
    unlit: Option<Vec<u32>>,
    /// Each pixel's light as the world and as the player (UC14): the two put together on the GPU
    /// lit each as itself.
    light_maps: Option<goldlink_light::LightMaps>,
}

/// A frame waiting to be lit, and whether the casters cast (the Shadows setting).
type Job = Arc<(Mutex<Option<(Shown, bool)>>, Condvar)>;
/// The frame lit last.
type Done = Arc<(Mutex<Option<Shown>>, Condvar)>;

/// The lighting's own thread (UC14): each frame lit there (its CPU part and the GPU's, the GPU
/// being the window's), the window taking the latest when it's done. Frames the lighting falls
/// behind on are skipped, never queued.
struct LightThread {
    job: Job,
    done: Done,
    stop: Arc<AtomicBool>,
    /// No usable GPU (or it failed): nothing is lit.
    no_gpu: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl LightThread {
    fn start(ctx: egui::Context) -> Self {
        let job: Job = Arc::default();
        let done: Done = Arc::default();
        let stop = Arc::new(AtomicBool::new(false));
        let no_gpu = Arc::new(AtomicBool::new(false));
        let thread = {
            let (job, done, stop, no_gpu) =
                (job.clone(), done.clone(), stop.clone(), no_gpu.clone());
            std::thread::Builder::new()
                .name("goldlink-lighting".into())
                .spawn(move || {
                    let mut lighting = goldlink_light::Lighting::new();
                    if !lighting.use_gpu() {
                        no_gpu.store(true, Ordering::Relaxed);
                        ctx.request_repaint();
                        return;
                    }
                    loop {
                        let (mut s, shadows) = {
                            let mut g = lock(&job.0);
                            loop {
                                if stop.load(Ordering::Relaxed) {
                                    return;
                                }
                                if let Some(j) = g.take() {
                                    break j;
                                }
                                g = job.1.wait(g).unwrap_or_else(|e| e.into_inner());
                            }
                        };
                        s.unlit = s
                            .attached
                            .as_ref()
                            .filter(|_| s.steady_shift.is_some())
                            .map(|_| match &s.wide {
                                Some(w) => w.clone(),
                                None => s.pixels[..256 * 192].to_vec(),
                            });
                        // A panic in the lighting (a GPU error wgpu panics on) turns the
                        // lighting off, not the game (KNOWN_RISKS #138).
                        let ok = crate::crash::caught(|| {
                            light(
                                &mut lighting,
                                shadows,
                                &mut s.pixels,
                                s.wide.as_deref_mut(),
                                s.side,
                                s.light.as_ref(),
                                s.attached.as_ref(),
                            )
                        });
                        if ok.is_none() {
                            no_gpu.store(true, Ordering::Relaxed);
                            ctx.request_repaint();
                            return;
                        }
                        s.light_maps = lighting.take_light_maps();
                        if !lighting.has_gpu() {
                            no_gpu.store(true, Ordering::Relaxed);
                        }
                        *lock(&done.0) = Some(s);
                        done.1.notify_all();
                        ctx.request_repaint();
                    }
                })
                .expect("the lighting's thread starts")
        };
        Self {
            job,
            done,
            stop,
            no_gpu,
            thread: Some(thread),
        }
    }

    fn no_gpu(&self) -> bool {
        self.no_gpu.load(Ordering::Relaxed)
    }

    /// Lights `s` next (in place of a frame not started yet).
    fn send(&self, s: Shown, shadows: bool) {
        *lock(&self.job.0) = Some((s, shadows));
        self.job.1.notify_one();
    }

    /// Waits (a moment at most) for the frame sent last.
    fn wait(&self) {
        let deadline = Instant::now() + Duration::from_millis(500);
        let mut g = lock(&self.done.0);
        while g.is_none() && !self.no_gpu() {
            let now = Instant::now();
            if now >= deadline {
                break;
            }
            g = self
                .done
                .1
                .wait_timeout(g, deadline - now)
                .map_or_else(|e| e.into_inner().0, |r| r.0);
        }
    }

    /// The frame lit last, once.
    fn take(&self) -> Option<Shown> {
        lock(&self.done.0).take()
    }
}

impl Drop for LightThread {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        self.job.1.notify_all();
        if let Some(t) = self.thread.take() {
            // A frame on a stuck GPU can take long: never wait for ever (after a crash this
            // waited with the window still up, so it never came back; KNOWN_RISKS #138).
            let until = Instant::now() + Duration::from_secs(1);
            while !t.is_finished() && Instant::now() < until {
                std::thread::sleep(Duration::from_millis(5));
            }
            if t.is_finished() {
                let _ = t.join();
            } else {
                log::warn!("the lighting's thread didn't stop in time: left to end on its own");
            }
        }
    }
}

/// The crop of the two pictures round the player (`bbox`: shown columns and rows) the GPU's
/// composite reads (UC14, composite.wgsl's planes), with the lights and the plain mask on the
/// arrays' columns.
fn gpu_frame(
    a: &goldlink_core::Attached,
    w: usize,
    bbox: [usize; 4],
    lights: Option<&goldlink_light::LightMaps>,
    seq: u64,
) -> crate::composite::Frame {
    use crate::composite::PLANES;
    let p = &a.pictures;
    let stride = p.stride().max(1) as usize;
    let cols = a.plain.len() / 192;
    let o = (cols - w) / 2;
    let from = goldlink_core::Nds::MAX_WIDE_SIDE - a.side;
    // A margin past the player: the glide (a pixel or two), the edges' neighbours.
    const M: usize = 8;
    let shift = o + from;
    let x0 = (bbox[0] + shift).saturating_sub(M);
    let y0 = bbox[1].saturating_sub(M);
    let x1 = (bbox[2] + shift + M).min(stride);
    let y1 = (bbox[3] + M).min(192);
    let (cw, ch) = (x1.saturating_sub(x0).max(1), y1.saturating_sub(y0).max(1));
    let cells = cw * ch;
    let lights = lights.filter(|l| l.width == w && l.data.len() >= w * 192);
    let mut words = Vec::with_capacity(cells * 28);
    let mut offsets = [0u32; PLANES];
    let at = |x: usize, y: usize| y * stride + x;
    for (plane, start) in offsets.iter_mut().enumerate() {
        *start = words.len() as u32;
        for y in y0..y0 + ch {
            for x in x0..x0 + cw {
                let i = at(x, y);
                let u = |k: usize| p.u32_plane(k).get(i).copied().unwrap_or(0);
                let f = |k: usize| p.f32_plane(k).get(i).copied().unwrap_or(1.0).to_bits();
                let f3 = |k: usize, c: usize| {
                    p.f32_plane(k)
                        .get(i * 3 + c)
                        .copied()
                        .unwrap_or(0.0)
                        .to_bits()
                };
                // The shown column of the arrays' column x.
                let sx = x as isize - shift as isize;
                // The pixel's lights and bloom (zeros off the picture).
                let light = || -> [u32; 6] {
                    match lights {
                        Some(l) if sx >= 0 && (sx as usize) < w => l.data[y * w + sx as usize],
                        _ => [0; 6],
                    }
                };
                match plane {
                    0..=10 => words.push(u(plane)),
                    11 => words.push(f(0)),
                    12 => words.extend([f3(1, 0), f3(1, 1), f3(1, 2)]),
                    13 => words.push(u(11)),
                    14 => words.push(u(12)),
                    15 => words.push(f(2)),
                    16 => words.extend([f3(3, 0), f3(3, 1), f3(3, 2)]),
                    17 => {
                        let v = light();
                        words.extend([v[0], v[1], v[4], v[5]]);
                    }
                    18 => {
                        let v = light();
                        words.extend([v[2], v[3]]);
                    }
                    _ => {
                        let px = x as isize - from as isize;
                        let plain = if px >= 0 && (px as usize) < cols {
                            a.plain[y * cols + px as usize]
                        } else {
                            0
                        };
                        words.push(u32::from(plain));
                    }
                }
            }
        }
    }
    let (disp_cnt, edges, left, right) = p.display();
    crate::composite::Frame {
        seq,
        words,
        offsets,
        crop: [x0 as i32, y0 as i32, cw as i32, ch as i32],
        disp_cnt,
        edges,
        left,
        right,
        render_width: goldlink_core::Nds::RENDER_WIDTH as u32,
        lit: lights.is_some(),
        grade: lights.map_or([1.0; 12], |l| l.grade),
    }
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
        let rows = |f: &Option<EdgeFill>| f.as_ref().map(|f| f.rows.clone());
        assert_eq!(rows(&edge_rows(&flat)[0]), Some(vec![white; 192]));
        assert_eq!(rows(&edge_rows(&flat)[1]), Some(vec![white; 192]));
        // A bar across the top of a black screen: continued row by row.
        let mut bar = vec![0u32; 256 * 192];
        bar[..256 * 20].fill(0x00F8_F8F8);
        let bars = rows(&edge_rows(&bar)[0]).unwrap();
        assert_eq!(
            (bars[0], bars[19], bars[20]),
            (white, white, egui::Color32::BLACK)
        );
        // A text box reaching the left edge (Oak's speech): its rows take the nearest row's colour.
        let mut text = flat.clone();
        for y in 160..180 {
            text[256 * y + 2] = 0x0020_6020;
        }
        assert_eq!(rows(&edge_rows(&text)[0]), Some(vec![white; 192]));
        // A picture at the left edge (most rows): that side black, the other still continued.
        let mut picture = flat.clone();
        for y in 0..150 {
            picture[256 * y + 2] = 0x0020_6020;
        }
        assert_eq!(edge_rows(&picture)[0], None);
        assert_eq!(rows(&edge_rows(&picture)[1]), Some(vec![white; 192]));
        // A button reaching the right edge over the background, outlined: the side continues the
        // background and the button is closed with its outline.
        let mut button = flat.clone();
        for y in 170..186 {
            for x in 200..256 {
                let edge = y == 170 || y == 185 || x == 200;
                button[256 * y + x] = if edge { 0x0030_3030 } else { 0x00E0_4060 };
            }
        }
        let right = edge_rows(&button)[1].clone().unwrap();
        assert_eq!(right.rows, vec![white; 192]);
        assert_eq!(
            right.caps[178],
            vec![egui::Color32::from_rgb(0x30, 0x30, 0x30)]
        );
    }
}
