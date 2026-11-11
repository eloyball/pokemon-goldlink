//! The launcher, inside the game's window (RFC §6 M4): the first run (pick your ROM), bringing a
//! BizHawk install's worlds over, and Play, which patches the ROM in memory and starts the game.
//! The old launcher's setup (coop-launcher/src/setup.rs), without BizHawk.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, channel};

use eframe::egui;
use goldlink_rom::{Release, sha1_hex};

pub const RELEASE_FILE: &str = "release.json";

const ROM_DIALOG_TITLE: &str = "Your Pokémon HeartGold (USA) ROM";

/// The system's file chooser for the ROM. On Linux (and the BSDs) rfd asks the desktop portal
/// over D-Bus through zbus on Tokio (rfd's `tokio` feature), which panics without a Tokio runtime
/// driving it: the UI thread has none, so the dialog gets its own for as long as it's open.
#[cfg(all(unix, not(target_os = "macos")))]
fn pick_rom_file() -> Result<Option<PathBuf>, String> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| format!("Couldn't open the file chooser: {e}"))?;
    let picked = runtime.block_on(
        rfd::AsyncFileDialog::new()
            .set_title(ROM_DIALOG_TITLE)
            .add_filter("Nintendo DS ROM", &["nds"])
            .pick_file(),
    );
    Ok(picked.map(|f| f.path().to_path_buf()))
}

/// Windows and macOS: the native dialogs, no async runtime involved.
#[cfg(not(all(unix, not(target_os = "macos"))))]
fn pick_rom_file() -> Result<Option<PathBuf>, String> {
    Ok(rfd::FileDialog::new()
        .set_title(ROM_DIALOG_TITLE)
        .add_filter("Nintendo DS ROM", &["nds"])
        .pick_file())
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
        let Ok(release) = self.release.clone() else {
            return;
        };
        let rom = match pick_rom_file() {
            Ok(Some(rom)) => rom,
            Ok(None) => return,
            Err(e) => {
                self.message = Some((e, true));
                return;
            }
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

    pub fn ui(&mut self, ctx: &egui::Context) -> Next {
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
            if let Some(note) = &self.update_note {
                ui.colored_label(egui::Color32::from_rgb(0xF5, 0x9E, 0x0B), note.as_str());
            }
            ui.separator();
            if let Some((label, _)) = &self.work {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label(label.as_str());
                });
            } else if !self.has_rom() {
                ui.label(
                    "GoldLink needs your own copy of Pokémon HeartGold (USA). It checks it, keeps a \
                     copy in your GoldLink folder and makes the co-op game from it.",
                );
                ui.add_space(6.0);
                if ui.button("Choose your ROM…").clicked() {
                    self.pick_rom();
                }
            } else {
                let play = egui::Button::new(egui::RichText::new("Play").size(22.0))
                    .min_size(egui::vec2(160.0, 44.0));
                if ui.add(play).clicked() {
                    self.play();
                }
            }
            ui.add_space(8.0);
            if let Some((text, error)) = &self.message {
                if *error {
                    ui.colored_label(ui.visuals().error_fg_color, text.as_str());
                } else {
                    ui.label(text.as_str());
                }
            }
            ui.add_space(8.0);
            egui::CollapsingHeader::new("Settings").show(ui, |ui| {
                if ui.button("Open your GoldLink folder (saves, settings)").clicked() {
                    open_folder(&self.paths.data);
                }
                if ui
                    .add_enabled(self.work.is_none(), egui::Button::new("Choose another ROM…"))
                    .clicked()
                {
                    self.pick_rom();
                }
            });
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
