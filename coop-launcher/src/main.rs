// GoldLink, the launcher (docs/RELEASE_PLAN.md §4): checks and patches the player's ROM, writes
// BizHawk's config and starts BizHawk with the Co-op Bridge.
//
//   GoldLink                                    the window
//   GoldLink --status | --setup <rom> | --play  the same steps without it (tests, scripts)
//   GoldLink --make-patch <source> <target> <out.bps>   release script only (§4.5)
#![cfg_attr(windows, windows_subsystem = "windows")]

mod bizhawk;
mod bps;
mod gui;
mod setup;

use std::path::Path;
use std::process::ExitCode;

fn make_patch(source: &Path, target: &Path, out: &Path) -> Result<(), String> {
    let s = std::fs::read(source).map_err(|e| format!("{}: {e}", source.display()))?;
    let t = std::fs::read(target).map_err(|e| format!("{}: {e}", target.display()))?;
    let patch = bps::create(&s, &t);
    // The round trip, before anything is written.
    if bps::apply(&s, &patch).map_err(|e| e.to_string())? != t {
        return Err("the patch doesn't reproduce the target".into());
    }
    setup::write_atomic(out, &patch)?;
    println!(
        "{}: {} bytes ({:.1} % of the target); source SHA-1 {}, target SHA-1 {}",
        out.display(),
        patch.len(),
        patch.len() as f64 * 100.0 / t.len().max(1) as f64,
        setup::sha1_hex(&s),
        setup::sha1_hex(&t)
    );
    Ok(())
}

fn cli(args: &[String]) -> Result<(), String> {
    if args[0] == "--make-patch" {
        let [_, s, t, o] = args else { return Err("usage: --make-patch <source> <target> <out.bps>".into()) };
        return make_patch(Path::new(s), Path::new(t), Path::new(o));
    }
    let paths = setup::Paths::detect()?;
    let release = setup::load_release(&paths)?;
    match (args[0].as_str(), args.get(1)) {
        ("--status", _) => {
            println!("version {}", release.version);
            println!("install {}", paths.install.display());
            println!("data {}", paths.data.display());
            println!("status {:?}", setup::status(&paths, &release));
            println!("mono {:?}", bizhawk::mono_version());
        }
        ("--setup", Some(rom)) => {
            bizhawk::mono_version()?;
            setup::import_rom(&paths, &release, Path::new(rom))?;
            bizhawk::write_config(&paths, false)?;
            println!("ready: {}", paths.game_rom().display());
        }
        ("--play", _) => {
            if setup::status(&paths, &release) != setup::Status::Ready {
                return Err("not set up yet (--setup <rom>)".into());
            }
            bizhawk::write_config(&paths, false)?;
            let child = bizhawk::launch(&paths)?;
            println!("BizHawk started (pid {})", child.id());
        }
        _ => return Err(format!("unknown arguments: {}", args.join(" "))),
    }
    Ok(())
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if !args.is_empty() {
        return match cli(&args) {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("[-] {e}");
                ExitCode::FAILURE
            }
        };
    }
    let paths = match setup::Paths::detect() {
        Ok(p) => p,
        Err(e) => {
            eprintln!("[-] {e}");
            return ExitCode::FAILURE;
        }
    };
    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default().with_inner_size([460.0, 340.0]).with_title("GoldLink"),
        ..Default::default()
    };
    match eframe::run_native("GoldLink", options, Box::new(|_cc| Ok(Box::new(gui::App::new(paths))))) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("[-] {e}");
            ExitCode::FAILURE
        }
    }
}
