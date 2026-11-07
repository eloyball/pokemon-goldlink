//! The release manifest, the folders, and the ROM setup (docs/RELEASE_PLAN.md §4.1, §4.4).

use serde::{Deserialize, Serialize};
use sha1::{Digest, Sha1};
use std::fs;
use std::path::{Path, PathBuf};

use crate::bps;

pub const RELEASE_FILE: &str = "release.json";
const STATE_FILE: &str = "launcher.json";

/// `release.json`, written by the release script beside the launcher.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Release {
    pub version: String,
    /// The relay protocol version the bridge speaks (docs/RELEASE_PLAN.md §5).
    pub wire_version: u32,
    /// The HeartGold (USA) ROM the patch applies to, and the patched game.
    pub rom_sha1: String,
    pub patched_sha1: String,
    pub patch: String,
    pub bizhawk_version: String,
    /// `owner/name` on GitHub, for the update check.
    pub repository: String,
}

/// What the launcher remembers in the data folder.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct State {
    /// The release whose patch made `rom/goldlink.nds`.
    pub patched_release: Option<String>,
    pub original_sha1: Option<String>,
}

pub struct Paths {
    /// Where the release was unpacked (the launcher's folder).
    pub install: PathBuf,
    /// The player's own files (§4.4).
    pub data: PathBuf,
}

impl Paths {
    /// The launcher's folder and the per-user data folder; `GOLDLINK_INSTALL_DIR` and
    /// `GOLDLINK_DATA_DIR` override them (development and tests).
    pub fn detect() -> Result<Paths, String> {
        let install = match std::env::var_os("GOLDLINK_INSTALL_DIR") {
            Some(dir) => PathBuf::from(dir),
            None => std::env::current_exe()
                .map_err(|e| format!("can't find the launcher's folder: {e}"))?
                .parent()
                .ok_or("can't find the launcher's folder")?
                .to_path_buf(),
        };
        let data = match std::env::var_os("GOLDLINK_DATA_DIR") {
            Some(dir) => PathBuf::from(dir),
            None => dirs::data_dir()
                .ok_or("can't find your user data folder")?
                .join(if cfg!(windows) { "GoldLink" } else { "goldlink" }),
        };
        Ok(Paths { install, data })
    }

    pub fn original_rom(&self) -> PathBuf {
        self.data.join("rom").join("original.nds")
    }
    pub fn game_rom(&self) -> PathBuf {
        self.data.join("rom").join("goldlink.nds")
    }
    pub fn bizhawk_data(&self) -> PathBuf {
        self.data.join("bizhawk")
    }
    pub fn config(&self) -> PathBuf {
        self.bizhawk_data().join("config.ini")
    }
    pub fn bizhawk(&self) -> PathBuf {
        self.install.join("BizHawk")
    }
}

pub fn sha1_hex(bytes: &[u8]) -> String {
    Sha1::digest(bytes).iter().map(|b| format!("{b:02x}")).collect()
}

/// Writes through a temporary file, so an interrupted write never leaves half a file.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let dir = path.parent().ok_or("bad path")?;
    fs::create_dir_all(dir).map_err(|e| format!("can't create {}: {e}", dir.display()))?;
    let tmp = path.with_extension("tmp");
    fs::write(&tmp, bytes).map_err(|e| format!("can't write {}: {e}", tmp.display()))?;
    fs::rename(&tmp, path).map_err(|e| format!("can't write {}: {e}", path.display()))
}

pub fn load_release(paths: &Paths) -> Result<Release, String> {
    let file = paths.install.join(RELEASE_FILE);
    let text = fs::read_to_string(&file).map_err(|e| format!("{} is missing ({e}): unpack the whole release", file.display()))?;
    serde_json::from_str(&text).map_err(|e| format!("{} is damaged: {e}", file.display()))
}

pub fn load_state(paths: &Paths) -> State {
    fs::read_to_string(paths.data.join(STATE_FILE))
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default()
}

fn save_state(paths: &Paths, state: &State) -> Result<(), String> {
    let text = serde_json::to_string_pretty(state).map_err(|e| e.to_string())?;
    write_atomic(&paths.data.join(STATE_FILE), text.as_bytes())
}

/// Where the setup stands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Status {
    /// No ROM yet: the first run.
    NeedRom,
    /// A ROM is stored, but the game was patched by another release (or not at all).
    NeedPatch,
    Ready,
}

pub fn status(paths: &Paths, release: &Release) -> Status {
    let state = load_state(paths);
    if !paths.original_rom().is_file() {
        return Status::NeedRom;
    }
    if state.patched_release.as_deref() != Some(release.version.as_str()) || !paths.game_rom().is_file() {
        return Status::NeedPatch;
    }
    Status::Ready
}

/// Checks the player's ROM, keeps a copy, and patches it (the first run, or "pick another ROM").
pub fn import_rom(paths: &Paths, release: &Release, rom: &Path) -> Result<(), String> {
    let bytes = fs::read(rom).map_err(|e| format!("can't read {}: {e}", rom.display()))?;
    let sha1 = sha1_hex(&bytes);
    if sha1.eq_ignore_ascii_case(&release.patched_sha1) {
        return Err("That's an already patched GoldLink ROM. Pick your original Pokémon HeartGold (USA) ROM.".into());
    }
    if !sha1.eq_ignore_ascii_case(&release.rom_sha1) {
        return Err(format!(
            "That isn't the ROM GoldLink needs. It needs Pokémon HeartGold (USA), unmodified \
             (SHA-1 {}); this file's SHA-1 is {sha1}.",
            release.rom_sha1
        ));
    }
    write_atomic(&paths.original_rom(), &bytes)?;
    let mut state = load_state(paths);
    state.original_sha1 = Some(sha1);
    state.patched_release = None;
    save_state(paths, &state)?;
    patch(paths, release)
}

/// Patches the stored ROM with this release's patch.
pub fn patch(paths: &Paths, release: &Release) -> Result<(), String> {
    let original = fs::read(paths.original_rom()).map_err(|e| format!("can't read your stored ROM: {e}"))?;
    let patch_file = paths.install.join(&release.patch);
    let patch = fs::read(&patch_file).map_err(|e| format!("{} is missing ({e}): unpack the whole release", patch_file.display()))?;
    let game = bps::apply(&original, &patch).map_err(|e| match e {
        bps::ApplyError::WrongSource => "Your stored ROM doesn't match this release. Pick your ROM again (Settings).".to_string(),
        other => format!("Patching failed: {other}."),
    })?;
    if !sha1_hex(&game).eq_ignore_ascii_case(&release.patched_sha1) {
        return Err("Patching gave an unexpected result: the release may be damaged. Download it again.".into());
    }
    write_atomic(&paths.game_rom(), &game)?;
    let mut state = load_state(paths);
    state.patched_release = Some(release.version.clone());
    save_state(paths, &state)
}
