//! The launcher's one window (docs/RELEASE_PLAN.md §4.2).

use eframe::egui;
use std::path::PathBuf;
use std::process::Child;
use std::sync::mpsc::{channel, Receiver};
use std::time::Duration;

use crate::bizhawk;
use crate::setup::{self, Paths, Release, Status};

enum Message {
    Info(String),
    Error(String),
}

pub struct App {
    paths: Paths,
    release: Result<Release, String>,
    mono: Result<String, String>,
    status: Status,
    work: Option<(String, Receiver<Result<(), String>>)>,
    message: Option<Message>,
    bizhawk: Option<Child>,
}

impl App {
    pub fn new(paths: Paths) -> App {
        let release = setup::load_release(&paths);
        let status = match &release {
            Ok(r) => setup::status(&paths, r),
            Err(_) => Status::NeedRom,
        };
        let mut app = App { paths, release, mono: bizhawk::mono_version(), status, work: None, message: None, bizhawk: None };
        if app.status == Status::NeedPatch && app.ready_to_work() {
            app.start_patch(); // a new release: bring the game up to date at once
        }
        app
    }

    fn ready_to_work(&self) -> bool {
        self.release.is_ok() && self.mono.is_ok() && self.work.is_none()
    }

    fn run(&mut self, label: &str, job: impl FnOnce(Paths, Release) -> Result<(), String> + Send + 'static) {
        let Ok(release) = self.release.clone() else { return };
        let paths = Paths { install: self.paths.install.clone(), data: self.paths.data.clone() };
        let (tx, rx) = channel();
        std::thread::spawn(move || {
            let _ = tx.send(job(paths, release));
        });
        self.message = None;
        self.work = Some((label.to_string(), rx));
    }

    fn start_patch(&mut self) {
        self.run("Updating your game for this release…", |paths, release| {
            setup::patch(&paths, &release)?;
            bizhawk::write_config(&paths, false)
        });
    }

    fn pick_rom(&mut self) {
        let picked: Option<PathBuf> = rfd::FileDialog::new()
            .set_title("Your Pokémon HeartGold (USA) ROM")
            .add_filter("Nintendo DS ROM", &["nds"])
            .pick_file();
        if let Some(rom) = picked {
            self.run("Checking and patching your ROM…", move |paths, release| {
                setup::import_rom(&paths, &release, &rom)?;
                bizhawk::write_config(&paths, false)
            });
        }
    }

    fn play(&mut self) {
        let result = bizhawk::write_config(&self.paths, false).and_then(|_| bizhawk::launch(&self.paths));
        match result {
            Ok(child) => {
                self.bizhawk = Some(child);
                self.message = Some(Message::Info(
                    "BizHawk is starting. In the Co-op Bridge window, pick your avatar, then enter the \
                     server address and room code your group uses."
                        .into(),
                ));
            }
            Err(e) => self.message = Some(Message::Error(e)),
        }
    }

    fn poll(&mut self) {
        if let Some((_, rx)) = &self.work {
            if let Ok(result) = rx.try_recv() {
                self.work = None;
                match result {
                    Ok(()) => self.message = Some(Message::Info("Your game is ready.".into())),
                    Err(e) => self.message = Some(Message::Error(e)),
                }
                if let Ok(r) = &self.release {
                    self.status = setup::status(&self.paths, r);
                }
            }
        }
        if let Some(child) = &mut self.bizhawk {
            if !matches!(child.try_wait(), Ok(None)) {
                self.bizhawk = None;
            }
        }
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.poll();
        if self.work.is_some() || self.bizhawk.is_some() {
            ctx.request_repaint_after(Duration::from_millis(250));
        }
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.add_space(4.0);
            ui.heading("GoldLink");
            match &self.release {
                Ok(r) => ui.weak(format!("Version {} · co-op Pokémon HeartGold", r.version)),
                Err(e) => {
                    ui.colored_label(ui.visuals().error_fg_color, e.as_str());
                    return;
                }
            };
            ui.separator();

            if let Err(e) = &self.mono {
                ui.colored_label(ui.visuals().error_fg_color, e.as_str());
                return;
            }

            if let Some((label, _)) = &self.work {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label(label.as_str());
                });
            } else {
                match self.status {
                    Status::NeedRom => {
                        ui.label(
                            "GoldLink needs your own copy of Pokémon HeartGold (USA). It checks it, \
                             keeps a copy in your GoldLink folder and makes the co-op game from it.",
                        );
                        ui.add_space(6.0);
                        if ui.button("Choose your ROM…").clicked() {
                            self.pick_rom();
                        }
                    }
                    Status::NeedPatch => {
                        if ui.button("Update your game for this release").clicked() {
                            self.start_patch();
                        }
                    }
                    Status::Ready => {
                        let running = self.bizhawk.is_some();
                        let play = egui::Button::new(egui::RichText::new(if running { "Playing" } else { "Play" }).size(22.0))
                            .min_size(egui::vec2(160.0, 44.0));
                        if ui.add_enabled(!running, play).clicked() {
                            self.play();
                        }
                    }
                }
            }

            ui.add_space(8.0);
            match &self.message {
                Some(Message::Info(text)) => {
                    ui.label(text.as_str());
                }
                Some(Message::Error(text)) => {
                    ui.colored_label(ui.visuals().error_fg_color, text.as_str());
                }
                None => {}
            }

            ui.add_space(8.0);
            egui::CollapsingHeader::new("Settings").show(ui, |ui| {
                if ui.button("Open your GoldLink folder (saves, settings)").clicked() {
                    bizhawk::open_folder(&self.paths.data);
                }
                if ui.add_enabled(self.work.is_none(), egui::Button::new("Choose another ROM…")).clicked() {
                    self.pick_rom();
                }
                let reset = egui::Button::new("Reset BizHawk settings (controls, layout) to GoldLink's");
                if ui.add_enabled(self.work.is_none() && self.bizhawk.is_none(), reset).clicked() {
                    self.message = Some(match bizhawk::write_config(&self.paths, true) {
                        Ok(()) => Message::Info("BizHawk's settings are back to GoldLink's defaults. Your saves are kept.".into()),
                        Err(e) => Message::Error(e),
                    });
                }
            });
        });
    }
}
