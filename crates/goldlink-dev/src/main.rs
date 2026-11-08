//! goldlink-dev: GoldLink's developer binary (DECISIONS D2; never packaged). For now (UC1):
//! headless runs of a ROM, and BizHawk's Lua probes on the new core (LUA_SHIM_SPEC).
//!
//!     goldlink-dev --headless --rom <patched.nds> [--save <file>] [--frames N] [--mailbox]
//!                  [--lua <script.lua>] [--rtc 2026-11-07T12:00:00] [--uncapped] [--no-bridge]
//!
//!     goldlink-dev --window --rom <patched.nds> [--data <dir>] [--rtc …] [--offline-world [--world CODE-UID]]
//!
//! opens the release window with the real bridge (and its dev hooks, `devbridge.rs`), or with a
//! stand-in for it (`--offline-world`, `offline.rs`).
//!
//! It also takes EmuHawk's arguments, so copies of the probe runners only swap the program:
//!
//!     goldlink-dev --config=<instance>/config.ini --lua=<script.lua> [--open-ext-tool-dll=CoopBridge] <rom>
//!
//! (the save is <NDS base path>/SaveRAM/<rom name>.SaveRAM, as BizHawk finds it).
//!
//! Exit status: 0 done (client.exit() or the frames ran), 2 a Lua error, 3 the ROM or save
//! couldn't be loaded, 64 bad arguments. Runners read the verdict from the probe's log.

mod devbridge;
mod lua;
mod machine;
mod offline;

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use machine::Machine;

/// coop_offsets.json `main_ram_offset` (hg-engine pins the mailbox there).
const MAILBOX: usize = 0x3D8060;

#[derive(Default, Debug)]
struct Args {
    rom: Option<PathBuf>,
    save: Option<PathBuf>,
    lua: Option<PathBuf>,
    frames: Option<u32>,
    rtc: Option<chrono::NaiveDateTime>,
    mailbox: bool,
    uncapped: bool,
    bridge: bool,
    window: bool,
    offline_world: bool,
    config_dir: Option<PathBuf>,
    data: Option<PathBuf>,
    world: Option<String>,
}

fn usage(err: &str) -> ExitCode {
    eprintln!(
        "goldlink-dev: {err}\nusage: goldlink-dev --headless --rom <nds> [--save <file>] [--frames N] [--mailbox] [--lua <script>] [--rtc <YYYY-MM-DDTHH:MM:SS>] [--uncapped]\n   or: goldlink-dev --config=<config.ini> --lua=<script> [--open-ext-tool-dll=CoopBridge] <rom>"
    );
    ExitCode::from(64)
}

/// BizHawk's save file for `rom` under the instance config's NDS base path.
fn bizhawk_save_path(config: &Path, rom: &Path) -> Result<PathBuf, String> {
    let text = std::fs::read_to_string(config).map_err(|e| format!("{}: {e}", config.display()))?;
    let cfg: serde_json::Value =
        serde_json::from_str(&text).map_err(|e| format!("{}: {e}", config.display()))?;
    let paths = cfg["PathEntries"]["Paths"]
        .as_array()
        .ok_or("config.ini has no PathEntries")?;
    let entry = |kind: &str| {
        paths
            .iter()
            .find(|e| e["System"] == "NDS" && e["Type"] == kind)
            .and_then(|e| e["Path"].as_str())
            .map(str::to_owned)
    };
    let base = PathBuf::from(entry("Base").ok_or("config.ini has no NDS base path")?);
    let save_dir = entry("Save RAM").unwrap_or_else(|| "./SaveRAM".into());
    let stem = rom.file_stem().ok_or("the ROM has no file name")?;
    Ok(base.join(save_dir).join(stem).with_extension("SaveRAM"))
}

fn parse() -> Result<Args, String> {
    let mut args = Args { ..Args::default() };
    let mut config: Option<PathBuf> = None;
    let mut bizhawk_bridge = false;
    let mut it = std::env::args().skip(1);
    while let Some(a) = it.next() {
        let mut value = |name: &str| it.next().ok_or(format!("{name} needs a value"));
        match a.as_str() {
            "--headless" => {}
            "--window" => args.window = true,
            "--offline-world" => args.offline_world = true,
            "--data" => args.data = Some(value("--data")?.into()),
            "--world" => args.world = Some(value("--world")?),
            "--rom" => args.rom = Some(value("--rom")?.into()),
            "--save" => args.save = Some(value("--save")?.into()),
            "--lua" => args.lua = Some(value("--lua")?.into()),
            "--frames" => {
                args.frames = Some(
                    value("--frames")?
                        .parse()
                        .map_err(|_| "--frames takes a number")?,
                )
            }
            "--rtc" => {
                args.rtc = Some(
                    chrono::NaiveDateTime::parse_from_str(&value("--rtc")?, "%Y-%m-%dT%H:%M:%S")
                        .map_err(|_| "--rtc takes YYYY-MM-DDTHH:MM:SS")?,
                )
            }
            "--mailbox" => args.mailbox = true,
            "--uncapped" => args.uncapped = true,
            "--no-bridge" => args.bridge = false,
            "--bridge" => args.bridge = true,
            _ if a.starts_with("--config=") => config = Some(a["--config=".len()..].into()),
            _ if a.starts_with("--lua=") => args.lua = Some(a["--lua=".len()..].into()),
            _ if a.starts_with("--open-ext-tool-dll=") => bizhawk_bridge = true,
            _ if !a.starts_with("--") && args.rom.is_none() => args.rom = Some(a.into()),
            _ => return Err(format!("unknown argument {a}")),
        }
    }
    if let Some(config) = config {
        // EmuHawk's form: no external tool = the script plays the bridge. The bridge's files
        // live beside the instance's config.ini, as the C# bridge's do.
        args.bridge = bizhawk_bridge;
        args.config_dir = config.parent().map(Path::to_path_buf);
        let rom = args.rom.as_ref().ok_or("no ROM given")?;
        args.save = Some(bizhawk_save_path(&config, rom)?);
    }
    Ok(args)
}

fn main() -> ExitCode {
    let args = match parse() {
        Ok(a) => a,
        Err(e) => return usage(&e),
    };
    let Some(rom_path) = args.rom.clone() else {
        return usage("no ROM given (--rom)");
    };
    if args.window {
        let data = match args.data.clone().map_or_else(goldlink::data_dir, Ok) {
            Ok(d) => d,
            Err(e) => return usage(&e),
        };
        let result = if args.offline_world {
            let world = args
                .world
                .clone()
                .unwrap_or_else(|| "OFFLINE-00000000".into());
            offline::run(rom_path, data, world, args.rtc)
        } else {
            window_with_bridge(rom_path, data, args.rtc)
        };
        return match result {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("goldlink-dev: {e}");
                ExitCode::from(3)
            }
        };
    }
    goldlink_core::set_log_sink(Some(|level, msg| {
        if level >= 2 {
            eprintln!("melonDS: {msg}");
        }
    }));
    let rom = match std::fs::read(&rom_path) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("goldlink-dev: {}: {e}", rom_path.display());
            return ExitCode::from(3);
        }
    };
    let mut machine = match Machine::new(
        rom,
        args.save.clone(),
        args.rtc,
        args.uncapped || args.lua.is_none(),
    ) {
        Ok(m) => m,
        Err(e) => {
            eprintln!("goldlink-dev: {e}");
            return ExitCode::from(3);
        }
    };
    if args.bridge {
        let Some(dir) = args.config_dir.clone().or(args.data.clone()) else {
            return usage(
                "the bridge needs a folder for its files (EmuHawk's --config=, or --data)",
            );
        };
        match devbridge::new(&dir) {
            Ok(d) => machine.bridge = Some(d),
            Err(e) => {
                eprintln!("goldlink-dev: {e}");
                return ExitCode::from(3);
            }
        }
    }

    let (status, machine) = match &args.lua {
        Some(script) => match lua::run(script, machine) {
            Ok((lua::Outcome::Exited, m)) => (ExitCode::SUCCESS, m),
            Ok((lua::Outcome::Error, m)) => (ExitCode::from(2), m),
            Err(e) => {
                eprintln!("goldlink-dev: {e}");
                return ExitCode::from(2);
            }
        },
        None => {
            let mut m = machine;
            for _ in 0..args.frames.unwrap_or(600) {
                m.frame();
            }
            (ExitCode::SUCCESS, m)
        }
    };
    if args.mailbox {
        match machine.nds.mailbox(MAILBOX) {
            Some(h) => println!(
                "mailbox: COOP v{} at 0x{MAILBOX:X}, heartbeat_rom {}, heartbeat_bridge {}, after {} frames",
                h.version,
                h.heartbeat_rom,
                h.heartbeat_bridge,
                machine.nds.frame_count()
            ),
            None => println!(
                "mailbox: not published after {} frames",
                machine.nds.frame_count()
            ),
        }
    }
    // BizHawk writes the SaveRAM when it closes the game; probes read it afterwards.
    if let Err(e) = machine.flush_save() {
        eprintln!("goldlink-dev: saving: {e}");
    }
    status
}

/// The window with the real bridge, its files in `data` (as the release program's).
fn window_with_bridge(
    rom: PathBuf,
    data: PathBuf,
    rtc: Option<chrono::NaiveDateTime>,
) -> Result<(), String> {
    let bytes = std::fs::read(&rom).map_err(|e| format!("{}: {e}", rom.display()))?;
    let ticker = devbridge::new(&data)?;
    let bridge = ticker.bridge.clone();
    goldlink::run(goldlink::Session {
        setup: goldlink_runtime::GameSetup {
            rom: std::sync::Arc::new(bytes),
            cart_save: Some(data.join(goldlink::CART_SAVE)),
            mailbox_offset: MAILBOX,
            rtc,
        },
        data_dir: data,
        bridge: Some(bridge),
        hooks: vec![Box::new(devbridge::TickHook(ticker))],
        title: "GoldLink (dev)".into(),
    })
}
