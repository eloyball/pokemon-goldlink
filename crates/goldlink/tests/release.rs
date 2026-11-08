//! The launcher's ROM steps against a real release (ignored: it needs your base ROM and a built
//! release package, never in the repository):
//!
//!     cargo test -p goldlink --test release -- --ignored
//!
//! `GOLDLINK_BASE_ROM` (default hg-engine/rom.nds) and `GOLDLINK_RELEASE_TAR` (default the v0.3
//! Linux package in the master checkout's build/release/).

use std::path::PathBuf;
use std::process::Command;

use goldlink::launcher::{Paths, import_rom, load_release, prepare_game};

fn repo() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

#[test]
#[ignore = "needs the base ROM and a release package"]
fn first_run_then_play_patches_in_memory() {
    let tar = std::env::var_os("GOLDLINK_RELEASE_TAR")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(
                "/home/eloyball/src/pokemon-hg/build/release/v0.3/GoldLink-v0.3-linux.tar.gz",
            )
        });
    let base = std::env::var_os("GOLDLINK_BASE_ROM")
        .map(PathBuf::from)
        .unwrap_or_else(|| repo().join("hg-engine/rom.nds"));
    let dir = std::env::temp_dir().join(format!("goldlink-release-{}", std::process::id()));
    std::fs::remove_dir_all(&dir).ok();
    std::fs::create_dir_all(&dir).unwrap();
    let ok = Command::new("tar")
        .arg("xzf")
        .arg(&tar)
        .arg("-C")
        .arg(&dir)
        .args([
            "--strip-components=1",
            "--wildcards",
            "*/release.json",
            "*/goldlink.bps",
        ])
        .status()
        .unwrap()
        .success();
    assert!(ok, "unpacking {}", tar.display());
    let paths = Paths {
        install: dir.clone(),
        data: dir.join("data"),
    };
    let release = load_release(&paths).unwrap();

    // A wrong pick is explained: the release's own patch file isn't a ROM.
    let err = import_rom(&paths, &release, &dir.join("goldlink.bps")).unwrap_err();
    assert!(err.contains("isn't a Nintendo DS ROM"), "{err}");

    import_rom(&paths, &release, &base).unwrap();
    assert!(paths.original_rom().is_file());
    let prov: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(paths.provenance()).unwrap()).unwrap();
    assert_eq!(
        prov["sha1"].as_str().unwrap(),
        release.rom_sha1.to_ascii_lowercase()
    );

    let game = prepare_game(&paths, &release).unwrap();
    assert_eq!(
        goldlink_rom::sha1_hex(&game),
        release.patched_sha1.to_ascii_lowercase()
    );
    // Nothing patched on disk: only the copy of the player's ROM and its record.
    let files: Vec<_> = std::fs::read_dir(dir.join("data/rom"))
        .unwrap()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(files.len(), 2, "{files:?}");
    std::fs::remove_dir_all(&dir).ok();
}
