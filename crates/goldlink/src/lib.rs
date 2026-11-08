//! GoldLink's game window (the unified client). The `goldlink` binary is the release program;
//! `goldlink-dev` opens the same window with its dev tools added as [`FrameHook`]s (D2).

pub mod app;
pub mod crash;
pub mod input;
pub mod launcher;
pub mod layout;
pub mod update;
pub mod world_ui;

use std::path::{Path, PathBuf};

use goldlink_bridge::{BridgeHook, SharedBridge};
use goldlink_runtime::config::Config;
use goldlink_runtime::{Emulator, FrameHook, GameSetup, Runner};

pub use goldlink_bridge;
pub use goldlink_runtime;

/// The co-op mailbox's main-RAM offset (coop_offsets.json `main_ram_offset`, pinned by
/// hg-engine's linker script; release.json carries it from UC7).
pub const MAILBOX_OFFSET: usize = 0x3D8060;

/// The cart's save file in the data folder (the BizHawk install's `.SaveRAM`).
pub const CART_SAVE: &str = "cart.sav";

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

/// Where `coop_avatars.json` may be: beside the program (a release), at the repository's root
/// (a build in `target/<profile>/`), or in the working folder.
pub fn avatar_files() -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Some(dir) = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(Path::to_path_buf))
    {
        out.push(dir.join(goldlink_bridge::avatars::FILE));
        out.push(dir.join("../..").join(goldlink_bridge::avatars::FILE));
    }
    out.push(PathBuf::from(goldlink_bridge::avatars::FILE));
    out
}

/// More UI drawn over the game each frame (the dev binary's panels; the release has none).
pub type ExtraUi = Box<dyn FnMut(&eframe::egui::Context)>;

/// One game in a window.
pub struct Session {
    pub setup: GameSetup,
    /// Settings (`goldlink.toml`) live here.
    pub data_dir: PathBuf,
    /// The co-op bridge; None plays without one (dev tools stand in).
    pub bridge: Option<SharedBridge>,
    pub hooks: Vec<Box<dyn FrameHook>>,
    pub title: String,
    pub extra_ui: Option<ExtraUi>,
}

/// Boots the game and shows it in a window until the player closes it. The save is written on
/// the way out.
pub fn run(session: Session) -> Result<(), String> {
    let Session {
        setup,
        data_dir,
        bridge,
        mut hooks,
        title,
        extra_ui,
    } = session;
    let (config, note) = Config::load(&data_dir);
    let emu = Emulator::new(setup)?;
    if let Some(b) = &bridge {
        hooks.insert(0, Box::new(BridgeHook { bridge: b.clone() }));
    }
    let runner = Runner::start(emu, hooks);
    let size = layout::window_size(&config.video, 2.0);
    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_inner_size(size)
            .with_min_inner_size(layout::window_size(&config.video, 1.0))
            .with_title(&title),
        #[cfg(windows)]
        renderer: eframe::Renderer::Wgpu,
        ..Default::default()
    };
    let notes: Vec<String> = note.into_iter().collect();
    let mut parts = Some((runner, bridge));
    // A graphics driver without what the renderer needs makes eframe panic, not fail.
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        eframe::run_native(
            &title,
            options,
            Box::new(|_cc| {
                let (runner, bridge) = parts.take().expect("one window");
                let mut app = app::GameApp::new(runner, bridge, config, data_dir, notes);
                app.extra_ui = extra_ui;
                Ok(Box::new(app))
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
    if let Some((r, _)) = parts {
        r.stop();
    }
    Ok(())
}

/// The release program's window: the launcher first, then the game in the same window.
struct Shell {
    launcher: launcher::Launcher,
    game: Option<app::GameApp>,
    config: Option<Config>,
    notes: Vec<String>,
}

impl Shell {
    fn start_game(&mut self, ctx: &eframe::egui::Context, rom: Vec<u8>) -> Result<(), String> {
        let data = self.launcher.paths.data.clone();
        let bridge = goldlink_bridge::Bridge::new(&data, &avatar_files())
            .map_err(|e| format!("the network: {e}"))?;
        let bridge: SharedBridge = std::sync::Arc::new(std::sync::Mutex::new(bridge));
        let emu = Emulator::new(GameSetup {
            rom: std::sync::Arc::new(rom),
            cart_save: Some(data.join(CART_SAVE)),
            mailbox_offset: MAILBOX_OFFSET,
            rtc: None,
        })?;
        let runner = Runner::start(
            emu,
            vec![Box::new(BridgeHook {
                bridge: bridge.clone(),
            })],
        );
        let config = self.config.take().unwrap_or_default();
        let size = layout::window_size(&config.video, 2.0);
        ctx.send_viewport_cmd(eframe::egui::ViewportCommand::InnerSize(size));
        ctx.send_viewport_cmd(eframe::egui::ViewportCommand::MinInnerSize(
            layout::window_size(&config.video, 1.0),
        ));
        self.game = Some(app::GameApp::new(
            runner,
            Some(bridge),
            config,
            data,
            std::mem::take(&mut self.notes),
        ));
        Ok(())
    }
}

impl eframe::App for Shell {
    fn update(&mut self, ctx: &eframe::egui::Context, _frame: &mut eframe::Frame) {
        if let Some(g) = self.game.as_mut() {
            g.ui(ctx);
            return;
        }
        if let launcher::Next::Play(rom) = self.launcher.ui(ctx)
            && let Err(e) = self.start_game(ctx, rom)
        {
            self.notes.push(e);
        }
    }

    #[cfg(not(windows))]
    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        if let Some(g) = self.game.as_mut() {
            g.shutdown();
        }
    }

    #[cfg(windows)]
    fn on_exit(&mut self) {
        if let Some(g) = self.game.as_mut() {
            g.shutdown();
        }
    }
}

/// The release program: the launcher (first run, the ROM, Play), then the game. `autoplay`
/// presses Play at once (`--play`).
pub fn run_release(autoplay: bool) -> Result<(), String> {
    let paths = launcher::Paths::detect()?;
    let (config, note) = Config::load(&paths.data);
    let title = "GoldLink";
    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_inner_size([460.0, 360.0])
            .with_title(title),
        #[cfg(windows)]
        renderer: eframe::Renderer::Wgpu,
        ..Default::default()
    };
    let mut launcher = launcher::Launcher::new(paths);
    launcher.autoplay = autoplay;
    let shell = Shell {
        launcher,
        game: None,
        config: Some(config),
        notes: note.into_iter().collect(),
    };
    let mut shell = Some(shell);
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        eframe::run_native(
            title,
            options,
            Box::new(|_cc| Ok(Box::new(shell.take().expect("one window")))),
        )
    }))
    .unwrap_or_else(|_| {
        Err(eframe::Error::AppCreation(
            "the graphics driver can't draw the window".into(),
        ))
    })
    .map_err(|e| e.to_string())
}
