//! GoldLink's game window (the unified client). The `goldlink` binary is the release program;
//! `goldlink-dev` opens the same window with its dev tools added as [`FrameHook`]s (D2).

pub mod app;
pub mod input;
pub mod layout;

use std::path::PathBuf;

use goldlink_runtime::config::Config;
use goldlink_runtime::{Emulator, FrameHook, GameSetup, Runner};

pub use goldlink_runtime;

/// The co-op mailbox's main-RAM offset (coop_offsets.json `main_ram_offset`, pinned by
/// hg-engine's linker script; release.json carries it from UC7).
pub const MAILBOX_OFFSET: usize = 0x3D8060;

/// The player's data folder: `GOLDLINK_DATA_DIR`, or the launcher's per-user folder.
pub fn data_dir() -> Result<PathBuf, String> {
    match std::env::var_os("GOLDLINK_DATA_DIR") {
        Some(d) => Ok(PathBuf::from(d)),
        None => dirs::data_local_dir()
            .map(|d| {
                d.join(if cfg!(windows) {
                    "GoldLink"
                } else {
                    "goldlink"
                })
            })
            .ok_or_else(|| "can't find your user data folder".into()),
    }
}

/// Boots the game and shows it in a window until the player closes it. The save is written on
/// the way out.
pub fn run(setup: GameSetup, hooks: Vec<Box<dyn FrameHook>>, title: &str) -> Result<(), String> {
    let config_dir = setup.data_dir.clone();
    let (config, note) = Config::load(&config_dir);
    let emu = Emulator::new(setup)?;
    let runner = Runner::start(emu, hooks);
    let size = layout::window_size(&config.video, 2.0);
    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_inner_size(size)
            .with_min_inner_size(layout::window_size(&config.video, 1.0))
            .with_title(title),
        #[cfg(windows)]
        renderer: eframe::Renderer::Wgpu,
        ..Default::default()
    };
    let notes: Vec<String> = note.into_iter().collect();
    // A graphics driver without what the renderer needs makes eframe panic, not fail.
    let mut runner = Some(runner);
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        eframe::run_native(
            title,
            options,
            Box::new(|_cc| {
                Ok(Box::new(app::GameApp::new(
                    runner.take().expect("one window"),
                    config,
                    config_dir,
                    notes,
                )))
            }),
        )
    }))
    .unwrap_or_else(|_| {
        Err(eframe::Error::AppCreation(
            "the graphics driver can't draw the window".into(),
        ))
    })
    .map_err(|e| e.to_string())?;
    // If the window never opened, the runner is still here: stop it so the save is written.
    if let Some(r) = runner {
        r.stop();
    }
    Ok(())
}
