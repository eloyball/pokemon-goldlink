//! GoldLink, the unified client.
//!
//!     goldlink --rom <goldlink.nds>
//!
//! The launcher's screens (ROM setup) move in at UC4; until then this opens the patched ROM
//! directly. The data folder (the launcher's, or GOLDLINK_DATA_DIR) holds the cart's save,
//! `coop_config.json`, `coop_worlds.json`, `coop_saves/` and `coop_backups/`.

#![cfg_attr(windows, windows_subsystem = "windows")]

use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::{Arc, Mutex};

use goldlink::goldlink_bridge::Bridge;
use goldlink_runtime::GameSetup;

fn main() -> ExitCode {
    let mut rom: Option<PathBuf> = None;
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--rom" => rom = args.next().map(PathBuf::from),
            "--version" => {
                println!("goldlink {}", env!("CARGO_PKG_VERSION"));
                return ExitCode::SUCCESS;
            }
            _ => {
                eprintln!("usage: goldlink --rom <goldlink.nds>");
                return ExitCode::from(64);
            }
        }
    }
    let result = (|| {
        let rom = rom.ok_or("no ROM given (--rom)")?;
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
        })
    })();
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("goldlink: {e}");
            ExitCode::FAILURE
        }
    }
}
