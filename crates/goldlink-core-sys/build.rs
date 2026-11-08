//! Builds melonDS's core and the shim with CMake (CMakeLists.txt here), then links them.

use std::env;
use std::path::{Path, PathBuf};
use std::process::Command;

fn main() {
    let dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    if !dir.join("melonDS/src/NDS.h").is_file() {
        panic!(
            "melonDS is missing: run `git submodule update --init crates/goldlink-core-sys/melonDS`"
        );
    }
    for p in [
        "CMakeLists.txt",
        "shim",
        "melonDS/CMakeLists.txt",
        "melonDS/src",
        "melonDS/cmake",
    ] {
        println!("cargo:rerun-if-changed={p}");
    }

    let target = env::var("TARGET").unwrap();
    let mut cfg = cmake::Config::new(&dir);
    cfg.build_target("goldlink_core").profile("Release");
    if target.contains("windows-gnu") && !cfg!(windows) {
        // Cross-compiling for Windows with MinGW from Linux.
        cfg.define("CMAKE_SYSTEM_NAME", "Windows")
            .define("CMAKE_C_COMPILER", "x86_64-w64-mingw32-gcc")
            .define("CMAKE_CXX_COMPILER", "x86_64-w64-mingw32-g++")
            .define("CMAKE_RC_COMPILER", "x86_64-w64-mingw32-windres");
    }
    let out = cfg.build().join("build");

    // goldlink_core, then melonDS's core, then what the core needs (static link order).
    for lib in ["goldlink_core", "core", "teakra"] {
        let file = find_lib(&out, lib)
            .unwrap_or_else(|| panic!("lib{lib} not found under {}", out.display()));
        println!(
            "cargo:rustc-link-search=native={}",
            file.parent().unwrap().display()
        );
        println!("cargo:rustc-link-lib=static={lib}");
    }
    if target.contains("windows-gnu") {
        // The C++ runtime statically, so the .exe needs no MinGW DLLs.
        for lib in ["stdc++", "winpthread"] {
            if let Some(path) = mingw_file(&format!("lib{lib}.a")) {
                println!(
                    "cargo:rustc-link-search=native={}",
                    path.parent().unwrap().display()
                );
            }
            println!("cargo:rustc-link-lib=static={lib}");
        }
        for lib in ["ole32", "comctl32", "wsock32", "ws2_32"] {
            println!("cargo:rustc-link-lib=dylib={lib}");
        }
    } else if target.contains("apple") {
        println!("cargo:rustc-link-lib=dylib=c++");
    } else {
        for lib in ["stdc++", "m", "rt", "pthread"] {
            println!("cargo:rustc-link-lib=dylib={lib}");
        }
    }
    println!("cargo:include={}", dir.join("shim").display());
}

/// `lib<name>.a` (or `<name>.lib`) anywhere under the CMake build directory.
fn find_lib(root: &Path, name: &str) -> Option<PathBuf> {
    let want = [format!("lib{name}.a"), format!("{name}.lib")];
    let mut stack = vec![root.to_path_buf()];
    while let Some(d) = stack.pop() {
        for entry in std::fs::read_dir(&d).ok()?.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if want
                .iter()
                .any(|w| path.file_name().is_some_and(|f| f == w.as_str()))
            {
                return Some(path);
            }
        }
    }
    None
}

fn mingw_file(name: &str) -> Option<PathBuf> {
    let out = Command::new("x86_64-w64-mingw32-g++")
        .arg(format!("-print-file-name={name}"))
        .output()
        .ok()?;
    let path = PathBuf::from(String::from_utf8(out.stdout).ok()?.trim());
    path.is_absolute().then_some(path)
}
