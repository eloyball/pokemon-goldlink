//! GoldLink, the unified client.
//!
//!     goldlink                      the launcher (your ROM, Play), then the game, one window
//!     goldlink --play               the same, Play pressed at once
//!     goldlink --join <host:port> <CODE>    Play, then join that world (a shortcut)
//!     goldlink --create <host:port> <CODE>  Play, then create that world
//!     goldlink --status             what's set up (version, folders, ROM)
//!     goldlink --setup <rom>        check and keep your ROM (the first run's step)
//!     goldlink --update             install a newer release, if there is one (the launcher's button)
//!     goldlink --rom <goldlink.nds> an already patched ROM, straight into the game
//!
//! The data folder (per user, or GOLDLINK_DATA_DIR) holds your ROM's copy, the cart's save,
//! `coop_config.json`, `coop_worlds.json`, `coop_saves/`, `coop_backups/` and `goldlink.toml`.

#![cfg_attr(windows, windows_subsystem = "windows")]

use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::{Arc, Mutex};

use goldlink::goldlink_bridge::Bridge;
use goldlink_runtime::GameSetup;

fn main() -> ExitCode {
    let mut rom: Option<PathBuf> = None;
    let mut play = false;
    let mut join: Option<(String, String, bool)> = None;
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--rom" => rom = args.next().map(PathBuf::from),
            "--play" => play = true,
            "--join" | "--create" => {
                let (Some(server), Some(code)) = (args.next(), args.next()) else {
                    eprintln!("usage: goldlink {a} <host:port> <WORLD CODE>");
                    return ExitCode::from(64);
                };
                join = Some((server, code.to_uppercase(), a == "--create"));
                play = true;
            }
            "--status" | "--setup" => {
                #[cfg(windows)]
                attach_console();
                let setup = if a == "--setup" {
                    args.next().map(PathBuf::from)
                } else {
                    None
                };
                return match cli(setup) {
                    Ok(()) => ExitCode::SUCCESS,
                    Err(e) => {
                        eprintln!("[-] {e}");
                        ExitCode::FAILURE
                    }
                };
            }
            "--update" => {
                #[cfg(windows)]
                attach_console();
                return match cli_update() {
                    Ok(()) => ExitCode::SUCCESS,
                    Err(e) => {
                        eprintln!("[-] {e}");
                        ExitCode::FAILURE
                    }
                };
            }
            "--version" => {
                println!("goldlink {}", env!("CARGO_PKG_VERSION"));
                return ExitCode::SUCCESS;
            }
            _ => {
                eprintln!(
                    "usage: goldlink [--play | --join <host:port> <CODE> | --create <host:port> <CODE> | --status | --setup <rom> | --update | --rom <goldlink.nds>]"
                );
                return ExitCode::from(64);
            }
        }
    }
    if let Ok(data) = goldlink::data_dir() {
        goldlink::crash::install(data, env!("CARGO_PKG_VERSION"));
    }
    let result = (|| {
        let Some(rom) = rom else {
            return goldlink::run_release(play, join);
        };
        let bytes = std::fs::read(&rom).map_err(|e| format!("{}: {e}", rom.display()))?;
        let data = goldlink::data_dir()?;
        let bridge = Bridge::new(&data, &goldlink::avatar_files())
            .map_err(|e| format!("the network: {e}"))?;
        goldlink::run(goldlink::Session {
            setup: GameSetup {
                rom: Arc::new(bytes),
                cart_save: Some(data.join(goldlink::CART_SAVE)),
                mailbox_offset: goldlink::MAILBOX_OFFSET,
                rtc: None,
            },
            data_dir: data,
            bridge: Some(Arc::new(Mutex::new(bridge))),
            hooks: Vec::new(),
            title: "GoldLink".into(),
            extra_ui: None,
            extra_input: None,
            release_look: None,
        })
    })();
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("goldlink: {e}");
            // Started from a desktop there's no terminal to read: say it in a box.
            rfd::MessageDialog::new()
                .set_level(rfd::MessageLevel::Error)
                .set_title("GoldLink")
                .set_description(format!("GoldLink couldn't start: {e}"))
                .show();
            ExitCode::FAILURE
        }
    }
}

/// `--update`: the launcher's Update button without the window (release tests, Wine).
fn cli_update() -> Result<(), String> {
    use goldlink::launcher::{Paths, load_release};
    use goldlink::update;
    let paths = Paths::detect()?;
    let release = load_release(&paths)?;
    update::clean_up(&paths.install);
    let Some(found) = update::find(&release.repository, &release.version) else {
        println!("up to date (version {}), or no answer", release.version);
        return Ok(());
    };
    if !found.installable() {
        return Err(format!(
            "GoldLink {} is out, but can't be installed from here: {}",
            found.tag, found.page
        ));
    }
    let (tx, rx) = std::sync::mpsc::channel();
    let shown = std::thread::spawn(move || {
        for stage in rx {
            if !matches!(stage, update::Stage::Downloading(..)) {
                println!("{stage:?}");
            }
        }
    });
    let program = update::install_release(&found, &paths.install, &update::RELEASE_KEY, &tx);
    drop(tx);
    shown.join().ok();
    println!("updated to {}: {}", found.tag, program?.display());
    Ok(())
}

/// `--status` / `--setup <rom>`: the launcher's steps without the window (release tests).
fn cli(setup: Option<PathBuf>) -> Result<(), String> {
    use goldlink::launcher::{Paths, import_rom, load_release};
    let paths = Paths::detect()?;
    let release = load_release(&paths)?;
    if let Some(rom) = setup {
        import_rom(&paths, &release, &rom)?;
        println!("ready: {}", paths.original_rom().display());
        return Ok(());
    }
    println!("version {}", release.version);
    println!("install {}", paths.install.display());
    println!("data {}", paths.data.display());
    println!(
        "status {}",
        if paths.original_rom().is_file() {
            "Ready"
        } else {
            "NeedRom"
        }
    );
    Ok(())
}

/// A Windows GUI program has no console: use the one it was started from, if any.
#[cfg(windows)]
#[allow(unsafe_code)]
fn attach_console() {
    unsafe extern "system" {
        fn AttachConsole(pid: u32) -> i32;
    }
    const ATTACH_PARENT_PROCESS: u32 = u32::MAX;
    // SAFETY: a plain Win32 call with a constant.
    unsafe {
        AttachConsole(ATTACH_PARENT_PROCESS);
    }
}
