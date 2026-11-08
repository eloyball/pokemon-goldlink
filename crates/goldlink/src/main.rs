//! GoldLink, the unified client (UC2: the game in a window, no co-op yet).
//!
//!     goldlink --rom <goldlink.nds> [--world CODE-UID]
//!
//! The launcher's screens (ROM setup, worlds) move in at UC4; until then this opens the patched
//! ROM directly. Saves go to <data>/coop_saves/<world>.sav, the data folder being the launcher's
//! (or GOLDLINK_DATA_DIR).

#![cfg_attr(windows, windows_subsystem = "windows")]

use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;

use goldlink_runtime::GameSetup;

fn main() -> ExitCode {
    let mut rom: Option<PathBuf> = None;
    let mut world = String::from("unassigned");
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--rom" => rom = args.next().map(PathBuf::from),
            "--world" => world = args.next().unwrap_or(world),
            "--version" => {
                println!("goldlink {}", env!("CARGO_PKG_VERSION"));
                return ExitCode::SUCCESS;
            }
            _ => {
                eprintln!("usage: goldlink --rom <goldlink.nds> [--world CODE-UID]");
                return ExitCode::from(64);
            }
        }
    }
    let result = (|| {
        let rom = rom.ok_or("no ROM given (--rom)")?;
        let bytes = std::fs::read(&rom).map_err(|e| format!("{}: {e}", rom.display()))?;
        let setup = GameSetup {
            rom: Arc::new(bytes),
            data_dir: goldlink::data_dir()?,
            world,
            mailbox_offset: goldlink::MAILBOX_OFFSET,
            rtc: None,
            backups: true,
        };
        goldlink::run(setup, Vec::new(), "GoldLink")
    })();
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("goldlink: {e}");
            ExitCode::FAILURE
        }
    }
}
