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

/// Fast-forward speed while the key is held (percent).
const FAST_FORWARD: f64 = 400.0;

struct Message {
    text: String,
    error: bool,
    until: Instant,
}

pub struct GameApp {
    runner: Option<Runner>,
    config: Config,
    config_dir: PathBuf,
    controls: Controls,
    audio: Option<AudioOut>,
    gilrs: Option<gilrs::Gilrs>,
    texture: Option<egui::TextureHandle>,
    last_seq: u64,
    bottom: egui::Rect,
    settings_open: bool,
    fast: bool,
    /// Fast-forward is allowed (in a world it becomes the world's decision, UC5).
    pub fast_forward_allowed: bool,
    messages: VecDeque<Message>,
}

impl GameApp {
    pub fn new(runner: Runner, config: Config, config_dir: PathBuf, notes: Vec<String>) -> Self {
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
            controls: Controls::new(&config.keys, &config.pad, &config.fast_forward_keys),
            runner: Some(runner),
            config,
            config_dir,
            audio,
            gilrs,
            texture: None,
            last_seq: u64::MAX,
            bottom: egui::Rect::NOTHING,
            settings_open: false,
            fast: false,
            fast_forward_allowed: true,
            messages,
        }
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
        let (held, down, pos, focused) = ctx.input(|i| {
            (
                i.keys_down.clone(),
                i.pointer.primary_down(),
                i.pointer.interact_pos(),
                i.focused,
            )
        });
        let mut keys = goldlink_core::Keys::NONE;
        let mut fast = false;
        if focused && !self.settings_open {
            keys |= self.controls.keyboard(&held);
            fast = self.controls.fast_forward(&held);
        }
        if let Some(g) = &mut self.gilrs {
            while g.next_event().is_some() {}
            if !self.settings_open {
                keys |= self.controls.gamepads(g);
            }
        }
        let touch = match (down, pos) {
            (true, Some(p)) if !self.settings_open => touch_at(self.bottom, p),
            _ => None,
        };
        *lock(&runner.shared.input) = Input { keys, touch };

        let fast = fast && self.fast_forward_allowed;
        if fast != self.fast {
            self.fast = fast;
            runner.send(Command::Speed(Some(if fast {
                FAST_FORWARD
            } else {
                100.0
            })));
        }
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
        egui::Window::new("Settings")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
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
                ui.label("Controls: A = X or K, B = Z or J, X = V, Y = C, L = Q, R = E, Start = Enter, Select = Space, arrows or WASD, the mouse on the bottom screen. Hold Tab to speed up.");
                ui.small(format!("Change them in {}", self.config_dir.join(goldlink_runtime::config::FILE).display()));
                ui.separator();
                ui.horizontal(|ui| {
                    if ui.button("Back to the game").clicked() {
                        self.settings_open = false;
                    }
                    if ui.button("Quit").clicked() {
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
        if let Some(r) = self.runner.take() {
            r.stop();
        }
    }
}

impl eframe::App for GameApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.settings_open = !self.settings_open;
        }
        self.feed_input(ctx);
        self.collect_events();
        self.update_texture(ctx);

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
        // The emulator runs on its own clock; draw whenever the display can.
        ctx.request_repaint();
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
