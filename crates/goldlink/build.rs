//! The Windows program's icon (assets/goldlink.ico, drawn by scripts/launcher_art.py): compiled
//! into a resource with MinGW's windres and linked into goldlink.exe, so Explorer, shortcuts and
//! the taskbar show it. Other targets need nothing here. `WINDRES` names another windres.

use std::path::PathBuf;
use std::process::Command;

fn main() {
    println!("cargo:rerun-if-changed=assets/goldlink.ico");
    println!("cargo:rerun-if-env-changed=WINDRES");
    let var = |k: &str| std::env::var(k).unwrap_or_default();
    if var("CARGO_CFG_TARGET_OS") != "windows" {
        return;
    }
    if var("CARGO_CFG_TARGET_ENV") != "gnu" {
        println!(
            "cargo:warning=goldlink.exe is built without its icon (only the GNU toolchain's windres is wired up)"
        );
        return;
    }
    let windres = std::env::var("WINDRES").unwrap_or_else(|_| {
        if cfg!(windows) {
            "windres".into()
        } else {
            format!("{}-w64-mingw32-windres", var("CARGO_CFG_TARGET_ARCH"))
        }
    });
    let out = PathBuf::from(var("OUT_DIR"));
    let ico = PathBuf::from(var("CARGO_MANIFEST_DIR"))
        .join("assets")
        .join("goldlink.ico");
    // windres reads the path as a C string: forward slashes, quotes escaped
    let ico = ico
        .to_string_lossy()
        .replace('\\', "/")
        .replace('"', "\\\"");
    let rc = out.join("goldlink.rc");
    std::fs::write(&rc, format!("1 ICON \"{ico}\"\n")).expect("writing goldlink.rc");
    let obj = out.join("goldlink-icon.o");
    let status = Command::new(&windres)
        .arg(&rc)
        .args(["-O", "coff", "-o"])
        .arg(&obj)
        .status()
        .unwrap_or_else(|e| {
            panic!("can't run {windres} (MinGW's resource compiler; set WINDRES): {e}")
        });
    assert!(status.success(), "{windres} failed on {}", rc.display());
    println!("cargo:rustc-link-arg-bin=goldlink={}", obj.display());
}
