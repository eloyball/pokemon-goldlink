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

use crate::input::{Controls, touch_at};
use crate::layout;
use crate::theme::{self, Kind};
use crate::world_ui::{self, WorldPanel};
use goldlink_bridge::SharedBridge;

/// The release program's bar above the screens (UC8 §2.1): the world, who's online, Leave.
pub const BAR: f32 = 38.0;

/// What the release program's game window asks of the shell.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GameAction {
    /// Leave the world: the game saves, then back to the launcher.
    Leave,
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
    settings_open: bool,
    messages: VecDeque<Message>,
    /// Drawn after everything else (dev panels).
    pub extra_ui: Option<crate::ExtraUi>,
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
        Self {
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
            settings_open: false,
            messages,
            extra_ui: None,
            release_ui: false,
            menu_open: false,
            details_open: false,
            world_code: String::new(),
            leaving: None,
        }
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
        app.say("Esc: menu · F3: details", false);
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
            keys |= self.controls.keyboard(&held);
        }
        let blocked = self.blocked();
        if let Some(g) = &mut self.gilrs {
            while g.next_event().is_some() {}
            if !blocked {
                keys |= self.controls.gamepads(g);
            }
        }
        let touch = match (down, pos) {
            (true, Some(p)) if !self.blocked() && !over_panel => touch_at(self.bottom, p),
            _ => None,
        };
        *lock(&runner.shared.input) = Input { keys, touch };
    }

    fn update_texture(&mut self, ctx: &egui::Context) {
        let Some(runner) = &self.runner else { return };
        let image = {
            let frame = lock(&runner.shared.frame);
            if frame.seq == self.last_seq {
                None
            } else {
                self.last_seq = frame.seq;
                let px: Vec<egui::Color32> = frame
                    .pixels
                    .iter()
                    .map(|&p| egui::Color32::from_rgb((p >> 16) as u8, (p >> 8) as u8, p as u8))
                    .collect();
                Some(egui::ColorImage {
                    size: [256, 384],
                    pixels: px,
                })
            }
        };
        let options = match self.config.video.filter {
            Filter::Nearest => egui::TextureOptions::NEAREST,
            Filter::Linear => egui::TextureOptions::LINEAR,
        };
        match (&mut self.texture, image) {
            (Some(t), Some(img)) => t.set(img, options),
            (None, img) => {
                let img =
                    img.unwrap_or_else(|| egui::ColorImage::new([256, 384], egui::Color32::BLACK));
                self.texture = Some(ctx.load_texture("screens", img, options));
            }
            (Some(_), None) => {}
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
                });
                changed |= ui.add(egui::Slider::new(&mut v.gap, 0..=96).text("Gap between the screens")).changed();
                ui.horizontal(|ui| {
                    ui.label("Pixels:");
                    changed |= ui.radio_value(&mut v.filter, Filter::Nearest, "Sharp").changed();
                    changed |= ui.radio_value(&mut v.filter, Filter::Linear, "Smooth").changed();
                });
                changed |= ui.checkbox(&mut v.integer_scale, "Whole-number scaling only").changed();
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
        } else if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            match self.world.as_mut() {
                Some(w) if w.open => w.open = false,
                _ => self.settings_open = !self.settings_open,
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
                let (top, bottom) = layout::screens(area, &self.config.video);
                self.bottom = bottom;
                if let Some(t) = &self.texture {
                    let p = ui.painter();
                    let top_uv =
                        egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 0.5));
                    let bottom_uv =
                        egui::Rect::from_min_max(egui::pos2(0.0, 0.5), egui::pos2(1.0, 1.0));
                    p.image(t.id(), top, top_uv, egui::Color32::WHITE);
                    p.image(t.id(), bottom, bottom_uv, egui::Color32::WHITE);
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
