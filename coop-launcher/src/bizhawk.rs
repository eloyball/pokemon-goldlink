//! BizHawk: its config in the data folder, and starting it (docs/RELEASE_PLAN.md §4.3, §4.4).

use serde_json::{json, Map, Value};
use sha2::{Digest, Sha512};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Child, Command};

use crate::setup::{write_atomic, Paths};

const DEFAULTS: &str = "defaults/config.ini";
const BRIDGE: &str = "ExternalTools/CoopBridge.dll";

fn bridge_dll(paths: &Paths) -> PathBuf {
    paths.bizhawk().join(BRIDGE)
}

fn read_json(path: &Path) -> Result<Value, String> {
    let text = fs::read_to_string(path).map_err(|e| format!("can't read {}: {e}", path.display()))?;
    serde_json::from_str(&text).map_err(|e| format!("{} is damaged: {e}", path.display()))
}

/// BizHawk trusts an external tool by its SHA-512 (uppercase hex, `SHA512:` prefix), and asks
/// before loading one it doesn't know.
fn trust_hash(dll: &Path) -> Result<String, String> {
    let bytes = fs::read(dll).map_err(|e| format!("{} is missing ({e}): unpack the whole release", dll.display()))?;
    Ok(format!("SHA512:{}", Sha512::digest(&bytes).iter().map(|b| format!("{b:02X}")).collect::<String>()))
}

/// The bridge's full path as BizHawk names it: canonical, and on Windows without the `\\?\`
/// prefix `canonicalize` adds (BizHawk uses the plain `C:\...` form).
fn trust_key(dll: &Path) -> String {
    let full = fs::canonicalize(dll).unwrap_or_else(|_| dll.to_path_buf()).to_string_lossy().to_string();
    if cfg!(windows) {
        if let Some(rest) = full.strip_prefix(r"\\?\UNC\") {
            return format!(r"\\{rest}");
        }
        if let Some(rest) = full.strip_prefix(r"\\?\") {
            return rest.to_string();
        }
    }
    full
}

/// Writes the player's BizHawk config: the release's defaults on the first run or a reset,
/// otherwise the player's own (their control changes stay). Either way this machine's paths and
/// the bundled bridge's trust are filled in.
pub fn write_config(paths: &Paths, reset: bool) -> Result<(), String> {
    let config = paths.config();
    let mut cfg = if reset || !config.is_file() {
        read_json(&paths.install.join(DEFAULTS))?
    } else {
        read_json(&config)?
    };
    let obj = cfg.as_object_mut().ok_or("the BizHawk config isn't a JSON object")?;

    // Saves, savestates and screenshots: the NDS base path in the data folder.
    let nds = paths.bizhawk_data().join("NDS");
    fs::create_dir_all(&nds).map_err(|e| format!("can't create {}: {e}", nds.display()))?;
    let nds = nds.to_string_lossy().to_string();
    let entries = obj
        .entry("PathEntries")
        .or_insert_with(|| json!({ "Paths": [] }))
        .as_object_mut()
        .ok_or("PathEntries isn't an object")?
        .entry("Paths")
        .or_insert_with(|| json!([]))
        .as_array_mut()
        .ok_or("PathEntries.Paths isn't a list")?;
    match entries
        .iter_mut()
        .find(|e| e.get("System").and_then(Value::as_str) == Some("NDS") && e.get("Type").and_then(Value::as_str) == Some("Base"))
    {
        Some(e) => e["Path"] = Value::String(nds),
        None => entries.push(json!({ "Type": "Base", "Path": nds, "System": "NDS" })),
    }

    // Several BizHawks on one machine; BizHawk's own update check would offer a version the
    // release isn't tested on (scripts/bizhawk_defaults.py sets them too).
    obj.insert("SingleInstanceMode".into(), Value::Bool(false));
    obj.insert("UpdateAutoCheckEnabled".into(), Value::Bool(false));
    let dll = bridge_dll(paths);
    let mut trusted = Map::new();
    trusted.insert(trust_key(&dll), Value::String(trust_hash(&dll)?));
    obj.insert("TrustedExtTools".into(), Value::Object(trusted));

    let text = serde_json::to_string_pretty(&cfg).map_err(|e| e.to_string())?;
    write_atomic(&config, text.as_bytes())
}

/// What BizHawk needs from the system: Mono on Linux; on Windows the Visual C++ runtime (its
/// native cores are built with MSVC; .NET Framework 4.8 comes with Windows 10 and 11). A short
/// description of what was found, or what to install.
pub fn runtime_check() -> Result<String, String> {
    if cfg!(windows) {
        let system = std::env::var_os("SystemRoot").map(PathBuf::from).unwrap_or_else(|| PathBuf::from(r"C:\Windows"));
        let missing: Vec<&str> =
            ["vcruntime140.dll", "msvcp140.dll"].into_iter().filter(|f| !system.join("System32").join(f).is_file()).collect();
        if !missing.is_empty() {
            return Err(format!(
                "BizHawk needs the Microsoft Visual C++ Redistributable (x64), which isn't installed \
                 ({} missing). Install it from https://aka.ms/vs/17/release/vc_redist.x64.exe, then \
                 start GoldLink again.",
                missing.join(", ")
            ));
        }
        return Ok("Visual C++ runtime".into());
    }
    let out = Command::new("mono")
        .arg("--version")
        .output()
        .map_err(|_| "Mono isn't installed. BizHawk needs it on Linux: install the \"mono\" package \
                      (Arch: sudo pacman -S mono; Debian / Ubuntu: sudo apt install mono-complete; \
                      Fedora: sudo dnf install mono-complete), then start GoldLink again."
            .to_string())?;
    let text = String::from_utf8_lossy(&out.stdout);
    Ok(text.lines().next().unwrap_or("").to_string())
}

/// Starts BizHawk with the bridge and the patched game.
pub fn launch(paths: &Paths) -> Result<Child, String> {
    let program = if cfg!(windows) { paths.bizhawk().join("EmuHawk.exe") } else { paths.bizhawk().join("EmuHawkMono.sh") };
    if !program.is_file() {
        return Err(format!("{} is missing: unpack the whole release", program.display()));
    }
    Command::new(&program)
        .current_dir(paths.bizhawk())
        .arg(format!("--config={}", paths.config().display()))
        .arg("--open-ext-tool-dll=CoopBridge")
        .arg(paths.game_rom())
        .spawn()
        .map_err(|e| format!("can't start BizHawk: {e}"))
}

/// Opens a folder in the system's file manager.
pub fn open_folder(dir: &Path) {
    let _ = fs::create_dir_all(dir);
    let opener = if cfg!(windows) { "explorer" } else { "xdg-open" };
    let _ = Command::new(opener).arg(dir).spawn();
}
