//! Builds melonDS's core and the shim with CMake (CMakeLists.txt here), then links them.
//!
//! melonDS is the submodule (upstream, untouched) with GoldLink's patches (`patches/*.patch`,
//! in name order) applied to a copy in OUT_DIR. Only files whose bytes changed are rewritten,
//! so CMake rebuilds what a patch or an update touched.

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
        "patches",
    ] {
        println!("cargo:rerun-if-changed={p}");
    }
    let melonds = patched_melonds(&dir);

    let target = env::var("TARGET").unwrap();
    let mut cfg = cmake::Config::new(&dir);
    cfg.build_target("goldlink_core")
        .profile("Release")
        .define("MELONDS_DIR", &melonds);
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

/// The submodule with GoldLink's patches applied: staged fresh in OUT_DIR, then synced into the
/// tree CMake builds, writing only files whose bytes differ.
fn patched_melonds(dir: &Path) -> PathBuf {
    let out = PathBuf::from(env::var("OUT_DIR").unwrap());
    let stage = out.join("melonDS-stage");
    let tree = out.join("melonDS");
    let _ = std::fs::remove_dir_all(&stage);
    copy_tree(&dir.join("melonDS"), &stage);
    let mut patches: Vec<PathBuf> = std::fs::read_dir(dir.join("patches"))
        .map(|r| r.flatten().map(|e| e.path()).collect())
        .unwrap_or_default();
    patches.retain(|p| p.extension().is_some_and(|e| e == "patch"));
    patches.sort();
    for patch in &patches {
        // `git apply` outside any repository is a plain patch tool; the ceiling keeps it from
        // finding a repository above OUT_DIR (the target folder can sit inside GoldLink's).
        let ok = Command::new("git")
            .arg("-C")
            .arg(&stage)
            .args(["apply", "--whitespace=nowarn"])
            .arg(patch)
            .env("GIT_CEILING_DIRECTORIES", &out)
            .status()
            .unwrap_or_else(|e| panic!("git apply: {e}"))
            .success();
        if !ok {
            panic!(
                "{} doesn't apply to the melonDS submodule: update the patch for this melonDS",
                patch.display()
            );
        }
    }
    sync_tree(&stage, &tree);
    tree
}

fn copy_tree(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for e in std::fs::read_dir(from).unwrap().flatten() {
        let name = e.file_name();
        if name == ".git" {
            continue;
        }
        let (src, dst) = (e.path(), to.join(&name));
        if src.is_dir() {
            copy_tree(&src, &dst);
        } else {
            std::fs::copy(&src, &dst).unwrap();
        }
    }
}

/// Makes `to` the same as `from`, touching only files that differ (and removing extras).
fn sync_tree(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    let mut seen = std::collections::HashSet::new();
    for e in std::fs::read_dir(from).unwrap().flatten() {
        let name = e.file_name();
        seen.insert(name.clone());
        let (src, dst) = (e.path(), to.join(&name));
        if src.is_dir() {
            sync_tree(&src, &dst);
        } else {
            let new = std::fs::read(&src).unwrap();
            if std::fs::read(&dst).ok().as_deref() != Some(new.as_slice()) {
                std::fs::write(&dst, new).unwrap();
            }
        }
    }
    for e in std::fs::read_dir(to).unwrap().flatten() {
        if !seen.contains(&e.file_name()) {
            let p = e.path();
            let _ = if p.is_dir() {
                std::fs::remove_dir_all(&p)
            } else {
                std::fs::remove_file(&p)
            };
        }
    }
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
