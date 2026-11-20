//! GoldLink's installer (`GoldLink-Setup.exe`, published with each release): finds the newest
//! player release on GitHub, downloads it, checks it is signed by the release key (the launcher's
//! updater, crates/goldlink-update), and installs it for this user, as a program: the Start
//! menu, a desktop shortcut if asked, Windows' apps list with an uninstaller. The launcher keeps
//! it up to date from then on. In GoldLink's look (the launcher's theme.rs, included as it is).
//!
//!   GoldLink-Setup.exe                    the installer's window
//!   GoldLink-Setup.exe --install <dir> [--desktop]   without a window (the tests; prints its steps)
//!   GoldLink-Uninstall.exe --uninstall [--quiet]      the uninstaller (the apps list starts it)
//!
//! `GOLDLINK_SETUP_DIR` names the default folder and `GOLDLINK_SETUP_PRESS=install` presses
//! INSTALL once the release is found (the tests' headless screenshots: no clicks there);
//! `GOLDLINK_UPDATE_API` points the release list elsewhere (the updater's).

#![cfg_attr(windows, windows_subsystem = "windows")]

use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::mpsc::{Receiver, TryRecvError, channel};
use std::time::Duration;

use eframe::egui::{self, Color32, RichText, vec2};
use goldlink_update::{self as update, Available, Stage};

mod system;
#[allow(dead_code)]
#[path = "../../goldlink/src/theme.rs"]
mod theme;

/// Where GoldLink's releases are (scripts/package_client.sh's REPOSITORY).
const REPOSITORY: &str = "eloyball/pokemon-goldlink";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let has = |flag: &str| args.iter().any(|a| a == flag);
    match args.first().map(String::as_str) {
        Some("--install") => quiet_install(args.get(1).map(PathBuf::from), has("--desktop")),
        Some("--uninstall") if has("--quiet") => quiet_uninstall(),
        Some("--uninstall") => window(Mode::Uninstall),
        _ => window(Mode::Install),
    }
}

/// Installs `release` into `dir` on its own thread: the files (the updater's download, checks
/// and install), then Windows' side (system.rs). Its stages come back on the receiver.
fn start_install(release: Available, dir: PathBuf, desktop: bool) -> Receiver<Stage> {
    let (tx, rx) = channel();
    std::thread::spawn(move || {
        let result = (|| -> Result<PathBuf, String> {
            std::fs::create_dir_all(&dir)
                .map_err(|e| format!("Couldn't make {}: {e}", dir.display()))?;
            let program = update::install_release(&release, &dir, &update::RELEASE_KEY, &tx)?;
            // An earlier version's files, renamed aside by the install.
            update::clean_up(&dir);
            system::integrate(&dir, &release.tag, &release.page, desktop)?;
            Ok(program)
        })();
        tx.send(match result {
            Ok(program) => Stage::Done(program),
            Err(e) => Stage::Failed(e),
        })
        .ok();
    });
    rx
}

/// The newest release (any version: "0.0" is older than all).
fn newest() -> Option<Available> {
    update::find(REPOSITORY, "0.0")
}

fn quiet_install(dir: Option<PathBuf>, desktop: bool) -> ExitCode {
    let dir = dir.unwrap_or_else(system::default_dir);
    let Some(release) = newest() else {
        eprintln!("GoldLink's releases couldn't be reached");
        return ExitCode::FAILURE;
    };
    eprintln!("installing GoldLink {} into {}", release.tag, dir.display());
    let rx = start_install(release, dir, desktop);
    let mut shown = 0;
    for stage in rx {
        match stage {
            Stage::Downloading(done, total) if total > 0 && done * 10 / total > shown => {
                shown = done * 10 / total;
                eprintln!("downloading: {} of {} MB", done >> 20, total >> 20);
            }
            Stage::Downloading(..) => {}
            Stage::Checking => eprintln!("checking the signature and the checksum"),
            Stage::Installing => eprintln!("installing"),
            Stage::Done(program) => {
                eprintln!("installed: {}", program.display());
                return ExitCode::SUCCESS;
            }
            Stage::Failed(e) => {
                eprintln!("not installed: {e}");
                return ExitCode::FAILURE;
            }
        }
    }
    ExitCode::FAILURE
}

/// The uninstaller's folder: its own.
fn own_dir() -> Option<PathBuf> {
    Some(std::env::current_exe().ok()?.parent()?.to_path_buf())
}

fn quiet_uninstall() -> ExitCode {
    match own_dir()
        .ok_or_else(|| "can't find the program's folder".to_owned())
        .and_then(|d| system::uninstall(&d))
    {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("not uninstalled: {e}");
            ExitCode::FAILURE
        }
    }
}

#[derive(Clone, Copy)]
enum Mode {
    Install,
    Uninstall,
}

fn window(mode: Mode) -> ExitCode {
    let title = match mode {
        Mode::Install => "Install GoldLink",
        Mode::Uninstall => "Uninstall GoldLink",
    };
    let icon =
        eframe::icon_data::from_png_bytes(include_bytes!("../../goldlink/assets/goldlink.png"))
            .ok();
    let mut viewport = egui::ViewportBuilder::default()
        .with_inner_size(vec2(760.0, 480.0))
        .with_resizable(false)
        .with_title(title);
    if let Some(icon) = icon {
        viewport = viewport.with_icon(icon);
    }
    let options = eframe::NativeOptions {
        viewport,
        renderer: eframe::Renderer::Wgpu,
        ..Default::default()
    };
    let ran = eframe::run_native(
        title,
        options,
        Box::new(move |cc| {
            cc.egui_ctx.set_theme(egui::ThemePreference::Dark);
            theme::install(&cc.egui_ctx);
            Ok(Box::new(Setup::new(&cc.egui_ctx, mode)))
        }),
    );
    if ran.is_ok() {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}

enum Page {
    Looking(Receiver<Option<Available>>),
    Ready(Available),
    Installing {
        release: Available,
        rx: Receiver<Stage>,
        stage: Stage,
    },
    Done {
        release: Available,
        program: PathBuf,
    },
    Failed(String),
    Uninstall(PathBuf),
    Removed(Result<(), String>),
}

struct Setup {
    art: theme::Art,
    page: Page,
    dir: PathBuf,
    desktop: bool,
    choosing: Option<Receiver<Option<PathBuf>>>,
    press: bool,
}

fn look() -> Page {
    let (tx, rx) = channel();
    std::thread::spawn(move || {
        tx.send(newest()).ok();
    });
    Page::Looking(rx)
}

impl Setup {
    fn new(ctx: &egui::Context, mode: Mode) -> Self {
        let page = match mode {
            Mode::Install => look(),
            Mode::Uninstall => Page::Uninstall(own_dir().unwrap_or_default()),
        };
        Self {
            art: theme::Art::load(ctx),
            page,
            dir: system::installed_dir().unwrap_or_else(system::default_dir),
            desktop: true,
            choosing: None,
            press: std::env::var("GOLDLINK_SETUP_PRESS").is_ok_and(|v| v == "install"),
        }
    }

    fn poll(&mut self) {
        if let Page::Looking(rx) = &self.page {
            match rx.try_recv() {
                Ok(Some(release)) => self.page = Page::Ready(release),
                Ok(None) | Err(TryRecvError::Disconnected) => {
                    self.page = Page::Failed(
                        "GoldLink's releases couldn't be reached (GitHub): check the connection, then try again.".into(),
                    );
                }
                Err(TryRecvError::Empty) => {}
            }
        }
        if let Page::Installing { release, rx, stage } = &mut self.page {
            while let Ok(s) = rx.try_recv() {
                *stage = s;
            }
            match stage.clone() {
                Stage::Done(program) => {
                    let release = release.clone();
                    self.page = Page::Done { release, program };
                }
                Stage::Failed(e) => self.page = Page::Failed(e),
                _ => {}
            }
        }
        if let Some(rx) = &self.choosing {
            match rx.try_recv() {
                Ok(picked) => {
                    if let Some(p) = picked {
                        self.dir = system::own_folder(&p);
                    }
                    self.choosing = None;
                }
                Err(TryRecvError::Disconnected) => self.choosing = None,
                Err(TryRecvError::Empty) => {}
            }
        }
    }

    fn install(&mut self, release: Available) {
        let rx = start_install(release.clone(), self.dir.clone(), self.desktop);
        self.page = Page::Installing {
            release,
            rx,
            stage: Stage::Downloading(0, 0),
        };
    }

    fn choose_folder(&mut self) {
        let (tx, rx) = channel();
        let start = self.dir.parent().map(Path::to_path_buf);
        std::thread::spawn(move || {
            let mut dialog = rfd::FileDialog::new().set_title("Where to install GoldLink");
            if let Some(s) = start.filter(|s| s.is_dir()) {
                dialog = dialog.set_directory(s);
            }
            tx.send(dialog.pick_folder()).ok();
        });
        self.choosing = Some(rx);
    }
}

fn mb(bytes: u64) -> String {
    format!("{:.1} MB", bytes as f64 / (1 << 20) as f64)
}

/// A line of text in a panel.
fn line(ui: &mut egui::Ui, text: impl Into<String>, size: f32, colour: Color32) {
    ui.label(RichText::new(text).font(theme::text(size)).color(colour));
}

/// One line of text cut with "…" to the room there is (a long folder).
fn line_cut(ui: &mut egui::Ui, text: impl Into<String>, size: f32, colour: Color32) {
    ui.add(egui::Label::new(RichText::new(text).font(theme::text(size)).color(colour)).truncate());
}

/// The download's bar: gold over an inset, `share` of it full.
fn bar(ui: &mut egui::Ui, share: f32) {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 22.0), egui::Sense::hover());
    let p = ui.painter();
    p.rect_filled(rect, egui::CornerRadius::ZERO, theme::INSET);
    let full = egui::Rect::from_min_size(
        rect.min,
        vec2(rect.width() * share.clamp(0.0, 1.0), rect.height()),
    );
    p.rect_filled(full.shrink(3.0), egui::CornerRadius::ZERO, theme::GOLD);
    theme::border(p, rect, 2.0, theme::EDGE);
}

impl eframe::App for Setup {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.poll();
        if self.press {
            if let Page::Ready(release) = &self.page {
                self.press = false;
                let release = release.clone();
                self.install(release);
            }
        }
        let mut action: Option<Action> = None;
        egui::CentralPanel::default().frame(egui::Frame::NONE).show(ctx, |ui| {
            let c = theme::chrome(ui, &self.art);
            ui.painter().text(
                egui::pos2(c.status.right() - 4.0, c.status.center().y),
                egui::Align2::RIGHT_CENTER,
                "SETUP",
                theme::label(14.0),
                theme::TEXT_DIM,
            );
            let data = system::data_dir().display().to_string();
            let (tip, error) = match &self.page {
                Page::Looking(_) => {
                    theme::panel(ui, c.main, "Install", theme::EDGE, |ui| {
                        line(ui, format!("Looking for the newest GoldLink{}", theme::dots(ui)), 22.0, theme::TEXT);
                    });
                    theme::big_button(ui, &self.art, c.button, None, "INSTALL", theme::Kind::Go, false);
                    ("GoldLink is downloaded from its releases on GitHub.".to_owned(), false)
                }
                Page::Ready(release) => {
                    let installed = system::installed_version(&self.dir);
                    theme::panel(ui, c.main, "Install", theme::EDGE, |ui| {
                        line(ui, format!("GoldLink {}", release.tag), 26.0, theme::GOLD);
                        line(ui, "The newest release: its signature is checked before it's installed.", 16.0, theme::TEXT_DIM);
                        ui.add_space(14.0);
                        line(ui, "Install to", 16.0, theme::TEXT_DIM);
                        ui.horizontal(|ui| {
                            let w = ui.available_width() - 150.0;
                            ui.allocate_ui(vec2(w, 40.0), |ui| {
                                theme::inset(ui, theme::EDGE, |ui| {
                                    line_cut(ui, self.dir.display().to_string(), 16.0, theme::TEXT);
                                });
                            });
                            if theme::button(ui, "CHANGE…", theme::Kind::Neutral, vec2(136.0, 40.0), self.choosing.is_none()).clicked() {
                                action = Some(Action::Choose);
                            }
                        });
                        if let Some(v) = &installed {
                            line(ui, format!("GoldLink v{} is installed there: this replaces it.", v.trim_start_matches('v')), 15.0, theme::AMBER);
                        }
                        ui.add_space(10.0);
                        ui.checkbox(&mut self.desktop, RichText::new("A shortcut on the desktop").font(theme::text(17.0)).color(theme::TEXT));
                        ui.add_space(6.0);
                        line(ui, format!("Your saves and settings stay in {data}."), 15.0, theme::TEXT_FAINT);
                    });
                    let label = if installed.is_some() { "UPDATE" } else { "INSTALL" };
                    if theme::big_button(ui, &self.art, c.button, None, label, theme::Kind::Go, true).clicked() {
                        action = Some(Action::Install(release.clone()));
                    }
                    ("For you only: no administrator needed. GoldLink keeps itself up to date.".to_owned(), false)
                }
                Page::Installing { release, stage, .. } => {
                    theme::panel(ui, c.main, "Installing", theme::EDGE, |ui| {
                        line(ui, format!("GoldLink {}", release.tag), 26.0, theme::GOLD);
                        ui.add_space(12.0);
                        let (text, share) = match stage {
                            Stage::Downloading(done, total) if *total > 0 => (
                                format!("Downloading: {} of {}", mb(*done), mb(*total)),
                                *done as f32 / *total as f32,
                            ),
                            Stage::Downloading(..) => (format!("Downloading{}", theme::dots(ui)), 0.0),
                            Stage::Checking => (format!("Checking the signature{}", theme::dots(ui)), 1.0),
                            _ => (format!("Putting it in place{}", theme::dots(ui)), 1.0),
                        };
                        line(ui, text, 18.0, theme::TEXT);
                        ui.add_space(8.0);
                        bar(ui, share);
                    });
                    theme::big_button(ui, &self.art, c.button, None, "INSTALL", theme::Kind::Go, false);
                    ("Checked against GoldLink's release key before it's put in place.".to_owned(), false)
                }
                Page::Done { release, .. } => {
                    theme::panel(ui, c.main, "Ready", theme::GREEN, |ui| {
                        line(ui, format!("GoldLink {} is installed.", release.tag), 26.0, theme::GOLD);
                        ui.add_space(12.0);
                        let place = if cfg!(windows) {
                            if self.desktop { "Find it in the Start menu and on your desktop." } else { "Find it in the Start menu." }
                        } else {
                            "Installed."
                        };
                        line(ui, place, 18.0, theme::TEXT);
                        line(ui, "Its launcher brings each new version: no need to run this again.", 16.0, theme::TEXT_DIM);
                        ui.add_space(18.0);
                        if theme::button(ui, "CLOSE", theme::Kind::Neutral, vec2(136.0, 40.0), true).clicked() {
                            action = Some(Action::Close);
                        }
                    });
                    if theme::big_button(ui, &self.art, c.button, Some("play"), "PLAY", theme::Kind::Go, true).clicked() {
                        action = Some(Action::Play);
                    }
                    ("Uninstall it from Windows' apps list whenever you like.".to_owned(), false)
                }
                Page::Failed(e) => {
                    theme::panel(ui, c.main, "Not installed", theme::RED, |ui| {
                        line(ui, e.clone(), 18.0, theme::RED);
                        ui.add_space(10.0);
                        line(ui, "Nothing was changed.", 16.0, theme::TEXT_DIM);
                    });
                    if theme::big_button(ui, &self.art, c.button, None, "RETRY", theme::Kind::Primary, true).clicked() {
                        action = Some(Action::Retry);
                    }
                    ("The releases page has the same files to install by hand.".to_owned(), true)
                }
                Page::Uninstall(dir) => {
                    theme::panel(ui, c.main, "Uninstall", theme::RED, |ui| {
                        line(ui, "Remove GoldLink from this computer?", 24.0, theme::GOLD);
                        ui.add_space(10.0);
                        line(ui, format!("The program ({}), its shortcuts and its place in Windows' apps go.", dir.display()), 16.0, theme::TEXT);
                        ui.add_space(6.0);
                        line(ui, format!("Your saves and settings stay in {data}."), 16.0, theme::TEXT_DIM);
                        ui.add_space(18.0);
                        if theme::button(ui, "CANCEL", theme::Kind::Neutral, vec2(136.0, 40.0), true).clicked() {
                            action = Some(Action::Close);
                        }
                    });
                    if theme::big_button(ui, &self.art, c.button, None, "REMOVE", theme::Kind::Danger, true).clicked() {
                        action = Some(Action::Uninstall(dir.clone()));
                    }
                    ("Close GoldLink first.".to_owned(), false)
                }
                Page::Removed(result) => {
                    let ok = result.is_ok();
                    theme::panel(ui, c.main, if ok { "Uninstalled" } else { "Not uninstalled" }, if ok { theme::GREEN } else { theme::RED }, |ui| {
                        match result {
                            Ok(()) => {
                                line(ui, "GoldLink is removed.", 24.0, theme::GOLD);
                                ui.add_space(10.0);
                                line(ui, format!("Your saves and settings are still in {data}."), 16.0, theme::TEXT_DIM);
                            }
                            Err(e) => line(ui, e.clone(), 18.0, theme::RED),
                        }
                    });
                    if theme::big_button(ui, &self.art, c.button, None, "CLOSE", theme::Kind::Neutral, true).clicked() {
                        action = Some(Action::Close);
                    }
                    (String::new(), !ok)
                }
            };
            theme::tip_line(ui, c.tip, &tip, error);
        });
        match action {
            Some(Action::Install(release)) => self.install(release),
            Some(Action::Choose) => self.choose_folder(),
            Some(Action::Retry) => self.page = look(),
            Some(Action::Uninstall(dir)) => self.page = Page::Removed(system::uninstall(&dir)),
            Some(Action::Play) => {
                if let Page::Done { program, .. } = &self.page {
                    let started = std::process::Command::new(program)
                        .current_dir(program.parent().unwrap_or(Path::new(".")))
                        .spawn();
                    match started {
                        Ok(_) => ctx.send_viewport_cmd(egui::ViewportCommand::Close),
                        Err(e) => {
                            self.page = Page::Failed(format!(
                                "GoldLink is installed but didn't start ({e})."
                            ))
                        }
                    }
                }
            }
            Some(Action::Close) => ctx.send_viewport_cmd(egui::ViewportCommand::Close),
            None => {}
        }
        if matches!(self.page, Page::Looking(_) | Page::Installing { .. })
            || self.choosing.is_some()
        {
            ctx.request_repaint_after(Duration::from_millis(100));
        }
    }
}

enum Action {
    Install(Available),
    Choose,
    Retry,
    Uninstall(PathBuf),
    Play,
    Close,
}
