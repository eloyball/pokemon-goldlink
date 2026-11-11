//! The launcher, inside the game's window (RFC §6 M4): the first run (pick your ROM), bringing a
//! BizHawk install's worlds over, and Play, which patches the ROM in memory and starts the game.
//! The old launcher's setup (coop-launcher/src/setup.rs), without BizHawk.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, TryRecvError, channel};

use eframe::egui;
use goldlink_rom::{Release, sha1_hex};

pub const RELEASE_FILE: &str = "release.json";

const ROM_DIALOG_TITLE: &str = "Your Pokémon HeartGold (USA) ROM";

/// A file or folder chooser that's open. It runs on a thread of its own, so the window keeps
/// drawing (and answering the desktop, which otherwise calls it "not responding") while the
/// player chooses; `poll` gives the answer once the chooser closes.
pub struct Choosing(Receiver<Result<Option<PathBuf>, String>>);

impl Choosing {
    /// None while the chooser is still open.
    pub fn poll(&self) -> Option<Result<Option<PathBuf>, String>> {
        match self.0.try_recv() {
            Ok(answer) => Some(answer),
            Err(TryRecvError::Empty) => None,
            Err(TryRecvError::Disconnected) => {
                Some(Err("The file chooser closed unexpectedly.".into()))
            }
        }
    }
}

/// Opens the chooser `dialog` builds.
fn choose<F>(dialog: impl FnOnce() -> F + Send + 'static) -> Choosing
where
    F: std::future::Future<Output = Option<rfd::FileHandle>> + 'static,
{
    let (tx, rx) = channel();
    let run = move || {
        let answer = run_dialog(dialog()).map(|f| f.map(|f| f.path().to_path_buf()));
        tx.send(answer).ok();
    };
    // macOS's panels belong to the main thread: there the chooser runs where it's asked for
    #[cfg(target_os = "macos")]
    run();
    #[cfg(not(target_os = "macos"))]
    if let Err(e) = std::thread::Builder::new()
        .name("file chooser".into())
        .spawn(run)
    {
        let (tx, rx) = channel();
        tx.send(Err(format!("Couldn't open the file chooser: {e}")))
            .ok();
        return Choosing(rx);
    }
    Choosing(rx)
}

/// Runs a file dialog to the end. On Linux (and the BSDs) rfd asks the desktop portal over D-Bus
/// through zbus on Tokio (rfd's `tokio` feature), which panics without a Tokio runtime. The
/// runtime lives as long as the program: ashpd keeps one D-Bus connection for the whole process,
/// and its reader runs on the runtime that opened it, so with a runtime per dialog every chooser
/// after the first waited forever for an answer that nothing read.
#[cfg(all(unix, not(target_os = "macos")))]
fn run_dialog<T>(dialog: impl std::future::Future<Output = T>) -> Result<T, String> {
    static RUNTIME: std::sync::OnceLock<Result<tokio::runtime::Runtime, String>> =
        std::sync::OnceLock::new();
    let runtime = RUNTIME
        .get_or_init(|| {
            tokio::runtime::Builder::new_multi_thread()
                .worker_threads(1)
                .thread_name("file chooser D-Bus")
                .enable_all()
                .build()
                .map_err(|e| format!("Couldn't open the file chooser: {e}"))
        })
        .as_ref()
        .map_err(Clone::clone)?;
    Ok(runtime.block_on(dialog))
}

/// Windows and macOS: the native dialogs, polled to the end here.
#[cfg(not(all(unix, not(target_os = "macos"))))]
fn run_dialog<T>(dialog: impl std::future::Future<Output = T>) -> Result<T, String> {
    Ok(pollster_block_on(dialog))
}

#[cfg(not(all(unix, not(target_os = "macos"))))]
fn pollster_block_on<T>(f: impl std::future::Future<Output = T>) -> T {
    // A minimal executor: the native dialogs complete on their own threads.
    use std::sync::Arc;
    use std::task::{Context, Poll, Wake, Waker};
    struct Thread(std::thread::Thread);
    impl Wake for Thread {
        fn wake(self: Arc<Self>) {
            self.0.unpark();
        }
    }
    let waker = Waker::from(Arc::new(Thread(std::thread::current())));
    let mut cx = Context::from_waker(&waker);
    let mut f = std::pin::pin!(f);
    loop {
        if let Poll::Ready(v) = f.as_mut().poll(&mut cx) {
            return v;
        }
        std::thread::park();
    }
}

/// The system's file chooser for the ROM.
fn choose_rom_file() -> Choosing {
    choose(|| {
        rfd::AsyncFileDialog::new()
            .set_title(ROM_DIALOG_TITLE)
            .add_filter("Nintendo DS ROM", &["nds"])
            .pick_file()
    })
}

/// The system's folder chooser (Import a save…).
pub fn choose_folder(title: &str) -> Choosing {
    let title = title.to_owned();
    choose(move || rfd::AsyncFileDialog::new().set_title(title).pick_folder())
}

pub struct Paths {
    /// Where the release was unpacked (the program's folder).
    pub install: PathBuf,
    /// The player's own files.
    pub data: PathBuf,
}

impl Paths {
    /// The program's folder and the per-user data folder; `GOLDLINK_INSTALL_DIR` and
    /// `GOLDLINK_DATA_DIR` override them (development and tests).
    pub fn detect() -> Result<Self, String> {
        let install = match std::env::var_os("GOLDLINK_INSTALL_DIR") {
            Some(d) => PathBuf::from(d),
            None => std::env::current_exe()
                .map_err(|e| format!("can't find GoldLink's folder: {e}"))?
                .parent()
                .ok_or("can't find GoldLink's folder")?
                .to_path_buf(),
        };
        Ok(Self {
            install,
            data: crate::data_dir()?,
        })
    }

    pub fn original_rom(&self) -> PathBuf {
        self.data.join("rom").join("original.nds")
    }

    pub fn provenance(&self) -> PathBuf {
        self.data.join("rom").join("provenance.json")
    }

    /// The BizHawk version's bridge folder in the same data folder.
    pub fn bizhawk(&self) -> PathBuf {
        self.data.join("bizhawk")
    }
}

pub fn load_release(paths: &Paths) -> Result<Release, String> {
    let file = paths.install.join(RELEASE_FILE);
    let text = std::fs::read_to_string(&file).map_err(|e| {
        format!(
            "{} is missing ({e}): unpack the whole release",
            file.display()
        )
    })?;
    Release::parse(&text)
}

/// Checks the player's ROM (the header says what it is when it's the wrong one) and keeps a copy,
/// with where it came from (RFC §3.2: provenance). The patch is applied at Play, in memory.
pub fn import_rom(paths: &Paths, release: &Release, rom: &Path) -> Result<(), String> {
    let bytes = std::fs::read(rom).map_err(|e| format!("can't read {}: {e}", rom.display()))?;
    goldlink_rom::check_base(&bytes, release).map_err(|e| e.to_string())?;
    goldlink_runtime::write_atomic(&paths.original_rom(), &bytes)?;
    let provenance = serde_json::json!({
        "source_name": rom.file_name().map(|n| n.to_string_lossy().into_owned()),
        "sha1": sha1_hex(&bytes),
        "bytes": bytes.len(),
        "imported_at": chrono::Utc::now().to_rfc3339(),
    });
    goldlink_runtime::write_atomic(
        &paths.provenance(),
        serde_json::to_string_pretty(&provenance)
            .unwrap_or_default()
            .as_bytes(),
    )
}

/// The game for this release: the stored ROM patched in memory and checked. Nothing is written.
pub fn prepare_game(paths: &Paths, release: &Release) -> Result<Vec<u8>, String> {
    let base = std::fs::read(paths.original_rom())
        .map_err(|e| format!("can't read your stored ROM: {e}"))?;
    let patch_file = paths.install.join(&release.patch);
    let patch = std::fs::read(&patch_file).map_err(|e| {
        format!(
            "{} is missing ({e}): unpack the whole release",
            patch_file.display()
        )
    })?;
    goldlink_rom::prepare(&base, &patch, release).map_err(|e| e.to_string())
}

/// Copies a BizHawk install's bridge files into the new client's folder, once: the player keeps
/// their place in locked worlds (the secret), their worlds list, their per-world saves and the
/// cart (RFC_DELTA §7.1). Nothing is moved or deleted. Returns what was brought over.
pub fn import_bizhawk(paths: &Paths) -> Option<String> {
    let old = paths.bizhawk();
    let config = old.join(goldlink_bridge::config::FILE);
    if !config.is_file() || paths.data.join(goldlink_bridge::config::FILE).exists() {
        return None;
    }
    let copy = |from: &Path, to: &Path| -> bool {
        to.parent().map(std::fs::create_dir_all);
        !to.exists() && std::fs::copy(from, to).is_ok()
    };
    copy(&config, &paths.data.join(goldlink_bridge::config::FILE));
    let worlds = old.join(goldlink_bridge::worlds::FILE);
    if worlds.is_file() {
        copy(&worlds, &paths.data.join(goldlink_bridge::worlds::FILE));
    }
    let mut saves = 0;
    if let Ok(dir) = std::fs::read_dir(old.join("coop_saves")) {
        for e in dir.flatten() {
            let to = paths.data.join("coop_saves").join(e.file_name());
            if e.path().is_file() && copy(&e.path(), &to) {
                saves += 1;
            }
        }
    }
    let cart = old.join("NDS").join("SaveRAM").join("goldlink.SaveRAM");
    if cart.is_file() {
        copy(&cart, &paths.data.join(crate::CART_SAVE));
    }
    Some(format!(
        "Brought over your BizHawk version's worlds, place in them and saves ({saves} files). Its backups stay with it."
    ))
}

enum Work {
    /// The ROM chooser is open.
    Choose(Choosing),
    Import(Receiver<Result<(), String>>),
    Prepare(Receiver<Result<Vec<u8>, String>>),
}

pub struct Launcher {
    pub paths: Paths,
    release: Result<Release, String>,
    work: Option<(String, Work)>,
    message: Option<(String, bool)>,
    /// Press Play at once (`goldlink --play`).
    pub autoplay: bool,
    update: Option<Receiver<Option<String>>>,
    update_note: Option<String>,
}

/// What the launcher wants next.
pub enum Next {
    Stay,
    /// Start the game with these bytes.
    Play(Vec<u8>),
}

impl Launcher {
    pub fn new(paths: Paths) -> Self {
        let release = load_release(&paths);
        let mut message = import_bizhawk(&paths).map(|m| (m, false));
        let crashes = crate::crash::take_new(&paths.data);
        if let Some(newest) = crashes.first() {
            message = Some((
                format!(
                    "GoldLink closed unexpectedly last time. A report was saved: {} (Settings → Open your GoldLink folder). Pass it on to whoever helps your group.",
                    newest.display()
                ),
                true,
            ));
        }
        // GOLDLINK_NO_UPDATE_CHECK=1 skips it (tests, or anyone who'd rather not).
        let update = match &release {
            Ok(r)
                if std::env::var_os("GOLDLINK_NO_UPDATE_CHECK").is_none()
                    && !r.repository.is_empty() =>
            {
                Some(crate::update::check(
                    r.repository.clone(),
                    r.version.clone(),
                ))
            }
            _ => None,
        };
        Self {
            paths,
            release,
            work: None,
            message,
            autoplay: false,
            update,
            update_note: None,
        }
    }

    pub fn version(&self) -> Option<&str> {
        self.release.as_ref().ok().map(|r| r.version.as_str())
    }

    fn has_rom(&self) -> bool {
        self.paths.original_rom().is_file()
    }

    fn pick_rom(&mut self) {
        if self.release.is_err() {
            return;
        }
        self.message = None;
        self.work = Some((
            "Choose your ROM in the window that opened…".into(),
            Work::Choose(choose_rom_file()),
        ));
    }

    /// The ROM the player chose: checked and copied on a thread.
    fn import_rom(&mut self, rom: PathBuf) {
        let Ok(release) = self.release.clone() else {
            return;
        };
        let paths = Paths {
            install: self.paths.install.clone(),
            data: self.paths.data.clone(),
        };
        let (tx, rx) = channel();
        std::thread::spawn(move || {
            tx.send(import_rom(&paths, &release, &rom)).ok();
        });
        self.message = None;
        self.work = Some(("Checking your ROM…".into(), Work::Import(rx)));
    }

    fn play(&mut self) {
        let Ok(release) = self.release.clone() else {
            return;
        };
        let paths = Paths {
            install: self.paths.install.clone(),
            data: self.paths.data.clone(),
        };
        let (tx, rx) = channel();
        std::thread::spawn(move || {
            tx.send(prepare_game(&paths, &release)).ok();
        });
        self.message = None;
        self.work = Some(("Preparing the game…".into(), Work::Prepare(rx)));
    }

    fn poll(&mut self) -> Next {
        let Some((_, work)) = &self.work else {
            return Next::Stay;
        };
        match work {
            Work::Choose(chooser) => match chooser.poll() {
                Some(Ok(Some(rom))) => {
                    self.work = None;
                    self.import_rom(rom);
                }
                Some(Ok(None)) => self.work = None,
                Some(Err(e)) => {
                    self.work = None;
                    self.message = Some((e, true));
                }
                None => {}
            },
            Work::Import(rx) => match rx.try_recv() {
                Ok(Ok(())) => {
                    self.work = None;
                    self.message = Some(("Your ROM is ready.".into(), false));
                }
                Ok(Err(e)) => {
                    self.work = None;
                    self.message = Some((e, true));
                }
                Err(_) => {}
            },
            Work::Prepare(rx) => match rx.try_recv() {
                Ok(Ok(game)) => {
                    self.work = None;
                    return Next::Play(game);
                }
                Ok(Err(e)) => {
                    self.work = None;
                    self.message = Some((e, true));
                }
                Err(_) => {}
            },
        }
        Next::Stay
    }

    /// The ROM screen (UC8 §2.1): your game, then PLAY.
    pub fn ui(&mut self, ctx: &egui::Context, art: &crate::theme::Art) -> Next {
        use crate::theme::{self, Kind};
        if self.autoplay && self.work.is_none() && self.release.is_ok() && self.has_rom() {
            self.autoplay = false;
            self.play();
        }
        let next = self.poll();
        if let Some(rx) = &self.update
            && let Ok(note) = rx.try_recv()
        {
            self.update_note = note;
            self.update = None;
        }
        if self.work.is_some() || self.update.is_some() {
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
        }
        egui::CentralPanel::default()
            .frame(egui::Frame::NONE)
            .show(ctx, |ui| {
                let c = theme::chrome(ui, art);
                if let Ok(r) = &self.release {
                    ui.scope_builder(egui::UiBuilder::new().max_rect(c.status), |ui| {
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.label(
                                egui::RichText::new(format!("GAME VERSION {}", r.version))
                                    .font(theme::label(14.0))
                                    .color(theme::TEXT),
                            );
                        });
                    });
                }
                let ready = self.has_rom() && self.release.is_ok();
                let w = (c.main.width() * 0.48).clamp(380.0, 520.0);
                let panel = egui::Rect::from_min_max(egui::pos2(c.main.right() - w, c.main.top()), c.main.max);
                theme::panel(ui, panel, "YOUR GAME", if ready { theme::GREEN } else { theme::EDGE }, |ui| {
                    ui.label(
                        egui::RichText::new("Six-player co-op Pokémon HeartGold")
                            .font(theme::text(21.0))
                            .color(theme::GOLD),
                    );
                    theme::hint(
                        ui,
                        "Press PLAY, then join your friends' world with its code, or create one and \
                         share the code.",
                    );
                    ui.add_space(4.0);
                    if let Err(e) = &self.release {
                        ui.colored_label(theme::RED, e.as_str());
                        return;
                    }
                    if let Some((label, _)) = &self.work {
                        ui.horizontal(|ui| {
                            ui.spinner();
                            ui.label(label.as_str());
                        });
                    } else if self.has_rom() {
                        theme::inset(ui, theme::GREEN, |ui| {
                            ui.label(
                                egui::RichText::new("POKÉMON HEARTGOLD (USA)")
                                    .font(theme::label(13.0))
                                    .color(theme::TEXT_DIM),
                            );
                            ui.label(egui::RichText::new("Ready to play").font(theme::text(26.0)).color(theme::TEXT));
                            ui.label(
                                egui::RichText::new("Your copy is checked: GoldLink makes the co-op game from it.")
                                    .font(theme::text(15.0))
                                    .color(theme::TEXT_DIM),
                            );
                        });
                    } else {
                        theme::inset(ui, theme::EDGE, |ui| {
                            theme::hint(
                                ui,
                                "GoldLink needs your own copy of Pokémon HeartGold (USA): an .nds \
                                 file. It checks it, keeps a copy in your GoldLink folder and makes \
                                 the co-op game from it.",
                            );
                        });
                    }
                    if let Some(note) = &self.update_note {
                        theme::inset(ui, theme::AMBER, |ui| {
                            ui.label(egui::RichText::new(note.as_str()).color(theme::AMBER));
                        });
                    }
                    ui.add_space(6.0);
                    let label = if self.has_rom() { "Choose another ROM…" } else { "Choose your ROM…" };
                    let kind = if self.has_rom() { Kind::Neutral } else { Kind::Primary };
                    let h = if self.has_rom() { 34.0 } else { 46.0 };
                    if theme::button(ui, label, kind, egui::vec2(ui.available_width(), h), self.work.is_none())
                        .clicked()
                    {
                        self.pick_rom();
                    }
                    if theme::button(ui, "Open your GoldLink folder", Kind::Neutral, egui::vec2(ui.available_width(), 34.0), true)
                        .on_hover_text("Saves, settings, backups")
                        .clicked()
                    {
                        open_folder(&self.paths.data);
                    }
                });
                let (tip, error) = match &self.message {
                    Some((t, e)) => (t.clone(), *e),
                    None if !self.has_rom() => ("Tip: choose your ROM to begin.".to_owned(), false),
                    None => ("Tip: your saves live in your GoldLink folder, not next to the program.".to_owned(), false),
                };
                theme::tip_line(ui, c.tip, &tip, error);
                if theme::big_button(ui, art, c.button, Some("play"), "PLAY", Kind::Go, ready && self.work.is_none())
                    .clicked()
                {
                    self.play();
                }
            });
        next
    }
}

/// Opens a folder in the system's file manager.
pub fn open_folder(dir: &Path) {
    std::fs::create_dir_all(dir).ok();
    let program = if cfg!(windows) {
        "explorer"
    } else if cfg!(target_os = "macos") {
        "open"
    } else {
        "xdg-open"
    };
    std::process::Command::new(program).arg(dir).spawn().ok();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn imports_a_bizhawk_install_once_without_touching_it() {
        let data = std::env::temp_dir().join(format!("goldlink-import-{}", std::process::id()));
        std::fs::remove_dir_all(&data).ok();
        let paths = Paths {
            install: data.clone(),
            data: data.clone(),
        };
        let old = paths.bizhawk();
        std::fs::create_dir_all(old.join("coop_saves")).unwrap();
        std::fs::create_dir_all(old.join("NDS/SaveRAM")).unwrap();
        std::fs::write(
            old.join("coop_config.json"),
            "{\"player_secret\": \"00000000000000AB\"}",
        )
        .unwrap();
        std::fs::write(old.join("coop_worlds.json"), "{\"worlds\": []}").unwrap();
        std::fs::write(old.join("coop_saves/HELLOK-09370FEE.sav"), [1u8; 4]).unwrap();
        std::fs::write(
            old.join("coop_saves/sram_owner.txt"),
            "HELLOK-09370FEE\nabc\n",
        )
        .unwrap();
        std::fs::write(old.join("NDS/SaveRAM/goldlink.SaveRAM"), [2u8; 4]).unwrap();
        let note = import_bizhawk(&paths).unwrap();
        assert!(note.contains("2 files"), "{note}");
        assert_eq!(
            std::fs::read(data.join("coop_saves/HELLOK-09370FEE.sav")).unwrap(),
            [1u8; 4]
        );
        assert_eq!(
            std::fs::read(data.join(crate::CART_SAVE)).unwrap(),
            [2u8; 4]
        );
        let cfg = goldlink_bridge::config::NetConfig::load_or_create(&data);
        assert_eq!(cfg.player_secret, 0xAB, "the secret came over");
        assert!(
            old.join("coop_config.json").is_file(),
            "the BizHawk files stay"
        );
        assert!(import_bizhawk(&paths).is_none(), "only once");
        std::fs::remove_dir_all(&data).ok();
    }
}

#[cfg(test)]
mod chooser_tests {
    use std::time::{Duration, Instant};

    /// Needs a desktop portal that answers by itself: dev/test_file_chooser.sh runs it against a
    /// fake one on a private D-Bus session (GOLDLINK_CHOOSER_EXPECT is what it picks). Only the
    /// first chooser in a program used to answer (the portal's D-Bus connection died with the
    /// first dialog's runtime), and the window's thread waited for every one.
    #[test]
    #[ignore]
    fn choosers_answer_every_time() {
        let expect = std::path::PathBuf::from(
            std::env::var("GOLDLINK_CHOOSER_EXPECT").expect("run by dev/test_file_chooser.sh"),
        );
        for i in 0..3 {
            let asked = Instant::now();
            let chooser = super::choose_folder("Your old GoldLink folder");
            assert!(
                asked.elapsed() < Duration::from_millis(100),
                "chooser {i} held up the window's thread"
            );
            let answer = loop {
                if let Some(answer) = chooser.poll() {
                    break answer;
                }
                assert!(
                    asked.elapsed() < Duration::from_secs(10),
                    "chooser {i} never answered"
                );
                std::thread::sleep(Duration::from_millis(20));
            };
            assert_eq!(answer, Ok(Some(expect.clone())), "chooser {i}");
        }
    }
}
