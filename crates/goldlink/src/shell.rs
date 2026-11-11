//! The release program's one window (UC8 §2.1, §2.2): the ROM screen, then the launcher; the
//! game takes the window over once a world is joined and gives it back after Leave, once the
//! game has saved. The bridge lives as long as the window; the emulator only while a world (or a
//! backup being restored) needs it.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use eframe::egui::{self, ViewportCommand, vec2};
use goldlink_bridge::{BridgeHook, LinkState, SharedBridge};
use goldlink_runtime::config::Config;
use goldlink_runtime::{Emulator, GameSetup, Runner};

use crate::app::{GameAction, GameApp};
use crate::launcher::{self, Launcher, Next};
use crate::lobby::{self, Action, Lobby};
use crate::theme::{self, Art, Kind};
use crate::{CART_SAVE, MAILBOX_OFFSET, avatar_files};

const LAUNCHER_SIZE: egui::Vec2 = egui::vec2(980.0, 600.0);
const LAUNCHER_MIN: egui::Vec2 = egui::vec2(860.0, 540.0);
/// A restored backup must be loaded by then, and saved by the ROM after that.
const RESTORE_LOAD: Duration = Duration::from_secs(20);
const RESTORE_SAVE: Duration = Duration::from_secs(90);

enum Stage {
    /// The ROM screen.
    Rom,
    Lobby,
    /// The emulator boots hidden and muted while the bridge joins and loads the world's save.
    Connecting {
        runner: Runner,
        code: String,
    },
    /// A started world whose save isn't here (UC8 §2.3): nothing is shown of the game.
    Missing {
        runner: Runner,
        code: String,
        server: String,
    },
    Playing {
        game: Box<GameApp>,
        code: String,
    },
    /// Leave was asked: the game shows until its disconnect save is written (`saves`, the ROM's
    /// save count when Leave began, moves; the bridge has kept it then, and the ROM only waits
    /// for A to restart) or it's at the title without a session.
    Leaving {
        game: Box<GameApp>,
        code: String,
        error: Option<String>,
        forced: bool,
        saves: u32,
        /// The game had a session (a save in play) when Leave began: there's a save to wait for.
        played: bool,
    },
    /// A backup goes in, hidden; the ROM saves it (`loaded`: its save count once loaded).
    Restoring {
        runner: Runner,
        since: Instant,
        loaded: Option<u32>,
    },
}

pub struct Shell {
    art: Option<Art>,
    rom: Launcher,
    lobby: Option<Lobby>,
    bridge: Option<SharedBridge>,
    stage: Stage,
    config: Config,
    data: PathBuf,
    /// The co-op game made at PLAY (patched in memory).
    game_rom: Option<Arc<Vec<u8>>>,
    notes: Vec<String>,
    version: String,
    /// `--join` / `--create`: the world to go to once at the launcher.
    auto_join: Option<(String, String, bool)>,
}

impl Shell {
    fn bridge(&mut self) -> Result<SharedBridge, String> {
        if let Some(b) = &self.bridge {
            return Ok(b.clone());
        }
        let b = goldlink_bridge::Bridge::new(&self.data, &avatar_files())
            .map_err(|e| format!("the network: {e}"))?;
        let b: SharedBridge = Arc::new(Mutex::new(b));
        self.bridge = Some(b.clone());
        Ok(b)
    }

    fn start_emulator(&self, bridge: &SharedBridge) -> Result<Runner, String> {
        let rom = self.game_rom.clone().ok_or("the game isn't prepared")?;
        goldlink_bridge::hook::lock(bridge).forget_game();
        let emu = Emulator::new(GameSetup {
            rom,
            cart_save: Some(self.data.join(CART_SAVE)),
            mailbox_offset: MAILBOX_OFFSET,
            rtc: None,
        })?;
        Ok(Runner::start(
            emu,
            vec![Box::new(BridgeHook {
                bridge: bridge.clone(),
            })],
        ))
    }

    fn resize(ctx: &egui::Context, size: egui::Vec2, min: egui::Vec2) {
        ctx.send_viewport_cmd(ViewportCommand::MinInnerSize(min));
        ctx.send_viewport_cmd(ViewportCommand::InnerSize(size));
    }

    fn back_to_lobby(&mut self, ctx: &egui::Context, message: Option<(String, bool)>) {
        Self::resize(ctx, LAUNCHER_SIZE, LAUNCHER_MIN);
        if let (Some(l), Some(b)) = (self.lobby.as_mut(), self.bridge.as_ref()) {
            if message.is_some() {
                l.message = message;
            }
            l.after_session(b);
        }
        self.stage = Stage::Lobby;
    }

    /// An error on the launcher's tip line.
    fn say(&mut self, e: String) {
        if let Some(l) = self.lobby.as_mut() {
            l.message = Some((e, true));
        }
    }

    fn join(&mut self, server: &str, code: &str, create: bool) -> Result<Stage, String> {
        let bridge = self.bridge()?;
        let runner = self.start_emulator(&bridge)?;
        goldlink_bridge::hook::lock(&bridge).join(server, code, create);
        Ok(Stage::Connecting {
            runner,
            code: code.to_owned(),
        })
    }

    fn view(&self) -> Option<goldlink_bridge::View> {
        self.bridge
            .as_ref()
            .map(|b| goldlink_bridge::hook::lock(b).view())
    }

    fn step(&mut self, ctx: &egui::Context) {
        let art = self.art.get_or_insert_with(|| Art::load(ctx));
        // Closing the window: from a game, it's Leave (save first); otherwise GoldLink quits.
        let close = ctx.input(|i| i.viewport().close_requested());
        let stage = std::mem::replace(&mut self.stage, Stage::Rom);
        self.stage = match stage {
            Stage::Rom => {
                if let Next::Play(rom) = self.rom.ui(ctx, art) {
                    self.game_rom = Some(Arc::new(rom));
                    match self.bridge() {
                        Ok(b) => {
                            if self.lobby.is_none() {
                                self.lobby = Some(Lobby::new(&b));
                            }
                            Self::resize(ctx, LAUNCHER_SIZE, LAUNCHER_MIN);
                            Stage::Lobby
                        }
                        Err(e) => {
                            self.notes.push(e);
                            Stage::Rom
                        }
                    }
                } else {
                    Stage::Rom
                }
            }
            Stage::Lobby => {
                let Some(bridge) = self.bridge.clone() else {
                    self.stage = Stage::Rom;
                    return;
                };
                let lobby = self.lobby.get_or_insert_with(|| Lobby::new(&bridge));
                let art = self.art.as_ref().expect("loaded above");
                let asked = lobby.ui(
                    ctx,
                    art,
                    &bridge,
                    &mut self.config,
                    &self.data,
                    &self.version,
                );
                let asked = match self.auto_join.take() {
                    Some((server, code, create)) => Action::Join {
                        server,
                        code,
                        create,
                    },
                    None => asked,
                };
                self.act(ctx, &bridge, asked)
            }
            Stage::Connecting { runner, code } => self.connecting(ctx, runner, code, close),
            Stage::Missing {
                runner,
                code,
                server,
            } => self.missing(ctx, runner, code, server, close),
            Stage::Playing { mut game, code } => {
                let asked = game.ui(ctx) == Some(GameAction::Leave);
                if close {
                    ctx.send_viewport_cmd(ViewportCommand::CancelClose);
                }
                let view = self.view();
                let lost = view.as_ref().is_some_and(|v| v.link == LinkState::Offline);
                if asked || close || lost {
                    let error = view
                        .filter(|v| lost && v.message_is_error)
                        .map(|v| v.message);
                    if !lost {
                        if let Some(b) = &self.bridge {
                            goldlink_bridge::hook::lock(b).leave();
                        }
                    }
                    game.leaving = Some("Saving your game".into());
                    let (saves, played) = self
                        .view()
                        .map_or((0, false), |v| (v.save_counter, v.game_in_session));
                    Stage::Leaving {
                        game,
                        code,
                        error,
                        forced: false,
                        saves,
                        played,
                    }
                } else {
                    Stage::Playing { game, code }
                }
            }
            Stage::Leaving {
                mut game,
                code,
                error,
                mut forced,
                saves,
                played,
            } => {
                let _ = game.ui(ctx);
                if close {
                    // A second close while saving: stop now (the last in-game save is kept).
                    ctx.send_viewport_cmd(ViewportCommand::CancelClose);
                    forced = true;
                }
                let view = self.view();
                let in_session = view.as_ref().is_some_and(|v| v.game_in_session);
                let saved = view
                    .as_ref()
                    .is_some_and(|v| v.save_counter != saves && v.save_counter != 0);
                if in_session && !saved && !forced {
                    game.leaving = Some(if close {
                        "Saving: close again to quit without waiting".into()
                    } else {
                        "Saving your game: finish any battle or menu".into()
                    });
                    Stage::Leaving {
                        game,
                        code,
                        error,
                        forced,
                        saves,
                        played,
                    }
                } else {
                    game.shutdown();
                    let message = match error {
                        Some(e) => (format!("{e} Your game was saved."), true),
                        None if forced => (
                            format!(
                                "You left world {code} before the game saved: your last in-game save is kept."
                            ),
                            true,
                        ),
                        None if played => (format!("Saved. You left world {code}."), false),
                        None => (format!("You left world {code}."), false),
                    };
                    self.back_to_lobby(ctx, Some(message));
                    Stage::Lobby
                }
            }
            Stage::Restoring {
                runner,
                since,
                loaded,
            } => self.restoring(ctx, runner, since, loaded),
        };
    }

    /// What the launcher asked for.
    fn act(&mut self, ctx: &egui::Context, bridge: &SharedBridge, asked: Action) -> Stage {
        match asked {
            Action::None => Stage::Lobby,
            Action::Quit => {
                ctx.send_viewport_cmd(ViewportCommand::Close);
                Stage::Lobby
            }
            Action::ChangeRom => Stage::Rom,
            Action::Join {
                server,
                code,
                create,
            } => match self.join(&server, &code, create) {
                Ok(s) => {
                    if let Some(l) = self.lobby.as_mut() {
                        l.message = None;
                    }
                    s
                }
                Err(e) => {
                    self.say(e);
                    Stage::Lobby
                }
            },
            Action::Restore(path) => match self.start_emulator(bridge) {
                Ok(runner) => {
                    goldlink_bridge::hook::lock(bridge).restore_backup(path);
                    Stage::Restoring {
                        runner,
                        since: Instant::now(),
                        loaded: None,
                    }
                }
                Err(e) => {
                    self.say(e);
                    Stage::Lobby
                }
            },
        }
    }

    fn connecting(
        &mut self,
        ctx: &egui::Context,
        runner: Runner,
        code: String,
        close: bool,
    ) -> Stage {
        let art = self.art.as_ref().expect("loaded");
        let view = self.view();
        let (message, error) = view.as_ref().map_or((String::new(), false), |v| {
            (v.message.clone(), v.message_is_error)
        });
        let cancel = lobby::waiting_screen(
            ctx,
            art,
            &format!("Joining world {code}"),
            &message,
            error,
            |ui, c, _| {
                theme::big_button(ui, art, c.button, None, "CANCEL", Kind::Neutral, true).clicked()
            },
        );
        ctx.request_repaint_after(Duration::from_millis(50));
        if close {
            ctx.send_viewport_cmd(ViewportCommand::CancelClose);
        }
        let Some(v) = view else {
            return Stage::Connecting { runner, code };
        };
        if cancel || close {
            if let Some(b) = &self.bridge {
                goldlink_bridge::hook::lock(b).leave();
            }
            runner.stop();
            self.back_to_lobby(ctx, None);
            return Stage::Lobby;
        }
        if v.link == LinkState::Offline {
            runner.stop();
            self.back_to_lobby(ctx, Some((v.message, true)));
            return Stage::Lobby;
        }
        if v.save_missing {
            let server = self
                .bridge
                .as_ref()
                .map(|b| goldlink_bridge::hook::lock(b).config.server.clone())
                .unwrap_or_default();
            return Stage::Missing {
                runner,
                code,
                server,
            };
        }
        if v.slot_ready {
            let bridge = self.bridge.clone().expect("connected");
            let game = GameApp::release(
                runner,
                bridge,
                self.config.clone(),
                self.data.clone(),
                std::mem::take(&mut self.notes),
                &code,
            );
            Self::resize(ctx, game.window_size(2.0), game.window_size(1.0));
            return Stage::Playing {
                game: Box::new(game),
                code,
            };
        }
        Stage::Connecting { runner, code }
    }

    fn missing(
        &mut self,
        ctx: &egui::Context,
        runner: Runner,
        code: String,
        server: String,
        close: bool,
    ) -> Stage {
        let art = self.art.as_ref().expect("loaded");
        let text = format!(
            "Your save for world {code} isn't on this computer, so the game won't start a new \
             one over it. If you played this world before (another computer, or an older \
             GoldLink), import its save; otherwise ask the world's host."
        );
        let (import, leave) = lobby::waiting_screen(
            ctx,
            art,
            "Your save isn't here",
            &text,
            true,
            |ui, c, panel| {
                let r = egui::Rect::from_min_size(
                    egui::pos2(panel.left() + 20.0, panel.bottom() - 66.0),
                    vec2(panel.width() - 40.0, 46.0),
                );
                let import = ui
                    .scope_builder(egui::UiBuilder::new().max_rect(r), |ui| {
                        theme::button(ui, "Import a save…", Kind::Primary, r.size(), true).clicked()
                    })
                    .inner;
                let leave = theme::big_button(
                    ui,
                    art,
                    c.button,
                    Some("leave"),
                    "LEAVE",
                    Kind::Neutral,
                    true,
                )
                .clicked();
                (import, leave)
            },
        );
        if close {
            ctx.send_viewport_cmd(ViewportCommand::CancelClose);
        }
        let leave_now = |shell: &mut Self| {
            if let Some(b) = &shell.bridge {
                goldlink_bridge::hook::lock(b).leave();
            }
        };
        if leave || close {
            leave_now(self);
            runner.stop();
            self.back_to_lobby(ctx, None);
            return Stage::Lobby;
        }
        if import {
            match launcher::pick_folder("Your old GoldLink folder") {
                Ok(Some(dir)) => {
                    // Out of the world first: the import may change who this player is.
                    leave_now(self);
                    runner.stop();
                    let result = crate::import::import(&self.data, &dir);
                    if let Some(b) = &self.bridge {
                        goldlink_bridge::hook::lock(b).reload_from_disk();
                    }
                    return match result {
                        Ok(report) if report.worlds.contains(&code) => {
                            match self.join(&server, &code, false) {
                                Ok(s) => s,
                                Err(e) => {
                                    self.back_to_lobby(ctx, Some((e, true)));
                                    Stage::Lobby
                                }
                            }
                        }
                        Ok(report) => {
                            self.back_to_lobby(
                                ctx,
                                Some((
                                    format!(
                                        "{} There was no save for world {code} in it.",
                                        report.summary()
                                    ),
                                    true,
                                )),
                            );
                            Stage::Lobby
                        }
                        Err(e) => {
                            self.back_to_lobby(ctx, Some((e, true)));
                            Stage::Lobby
                        }
                    };
                }
                Ok(None) => {}
                Err(e) => {
                    self.say(e);
                }
            }
        }
        ctx.request_repaint_after(Duration::from_millis(100));
        Stage::Missing {
            runner,
            code,
            server,
        }
    }

    fn restoring(
        &mut self,
        ctx: &egui::Context,
        runner: Runner,
        since: Instant,
        loaded: Option<u32>,
    ) -> Stage {
        let art = self.art.as_ref().expect("loaded");
        lobby::waiting_screen(
            ctx,
            art,
            "Restoring a backup",
            "Your game goes back to the backup's moment and saves it. This takes a few seconds.",
            false,
            |_, _, _| (),
        );
        ctx.request_repaint_after(Duration::from_millis(100));
        let view = self.view();
        let in_session = view.as_ref().is_some_and(|v| v.game_in_session);
        let Some(saves) = loaded else {
            if in_session && !view.as_ref().is_some_and(|v| v.restore_pending) {
                return Stage::Restoring {
                    runner,
                    since: Instant::now(),
                    loaded: Some(view.as_ref().map_or(0, |v| v.save_counter)),
                };
            }
            if since.elapsed() > RESTORE_LOAD || view.as_ref().is_some_and(|v| v.message_is_error) {
                runner.stop();
                let e = view.map(|v| v.message).unwrap_or_default();
                self.back_to_lobby(
                    ctx,
                    Some((format!("The backup couldn't be restored. {e}"), true)),
                );
                return Stage::Lobby;
            }
            return Stage::Restoring {
                runner,
                since,
                loaded,
            };
        };
        // The restored game's disconnect save is written (and kept by the bridge): done. The ROM
        // only waits for A to restart after it.
        let saved = view
            .as_ref()
            .is_some_and(|v| v.save_counter != saves && v.save_counter != 0);
        if saved || !in_session {
            runner.stop();
            self.back_to_lobby(
                ctx,
                Some((
                    "Backup restored and saved. Join your world to continue.".into(),
                    false,
                )),
            );
            return Stage::Lobby;
        }
        if since.elapsed() > RESTORE_SAVE {
            runner.stop();
            self.back_to_lobby(
                ctx,
                Some(("The restored game didn't save in time; nothing was changed. Try another backup.".into(), true)),
            );
            return Stage::Lobby;
        }
        Stage::Restoring {
            runner,
            since,
            loaded,
        }
    }

    fn shutdown(&mut self) {
        match std::mem::replace(&mut self.stage, Stage::Rom) {
            Stage::Playing { mut game, .. } | Stage::Leaving { mut game, .. } => game.shutdown(),
            Stage::Connecting { runner, .. }
            | Stage::Missing { runner, .. }
            | Stage::Restoring { runner, .. } => {
                if let Some(b) = &self.bridge {
                    let mut b = goldlink_bridge::hook::lock(b);
                    if b.link() != LinkState::Offline {
                        b.leave();
                    }
                }
                runner.stop();
            }
            Stage::Rom | Stage::Lobby => {}
        }
    }
}

impl eframe::App for Shell {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.step(ctx);
    }

    #[cfg(not(windows))]
    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        self.shutdown();
    }

    #[cfg(windows)]
    fn on_exit(&mut self) {
        self.shutdown();
    }
}

/// The release program: the ROM screen, the launcher, the game, in one window. `autoplay`
/// presses PLAY at once (`--play`).
pub fn run(autoplay: bool, auto_join: Option<(String, String, bool)>) -> Result<(), String> {
    let paths = launcher::Paths::detect()?;
    let (config, note) = Config::load(&paths.data);
    let data = paths.data.clone();
    let mut rom = Launcher::new(paths);
    rom.autoplay = autoplay;
    let version = rom.version().unwrap_or("?").to_owned();
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size(LAUNCHER_SIZE)
            .with_min_inner_size(LAUNCHER_MIN)
            .with_title("GoldLink"),
        #[cfg(windows)]
        renderer: eframe::Renderer::Wgpu,
        ..Default::default()
    };
    let mut shell = Some(Shell {
        art: None,
        rom,
        lobby: None,
        bridge: None,
        stage: Stage::Rom,
        config,
        data,
        game_rom: None,
        notes: note.into_iter().collect(),
        version,
        auto_join,
    });
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        eframe::run_native(
            "GoldLink",
            options,
            Box::new(|cc| {
                theme::install(&cc.egui_ctx);
                Ok(Box::new(shell.take().expect("one window")))
            }),
        )
    }))
    .unwrap_or_else(|_| {
        Err(eframe::Error::AppCreation(
            "the graphics driver can't draw the window".into(),
        ))
    })
    .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    //! The session's life end to end (UC8 §2.2), in this process: egui without a window, the
    //! real emulator and the real relay. Ignored by default: it needs the development ROM and the
    //! e2e harness's TEAM01 fixture (a save in the overworld and the relay's campaign):
    //!
    //!     cargo build --release --manifest-path coop-relay/Cargo.toml
    //!     cargo test -p goldlink --lib -- --ignored session
    //!
    //! GOLDLINK_RELAY, GOLDLINK_TEST_ROM, GOLDLINK_TEST_INSTANCE and GOLDLINK_TEST_CAMPAIGN
    //! point elsewhere.

    use super::*;
    use eframe::egui::{Event, Key, RawInput, ViewportEvent, ViewportId, ViewportInfo};
    use std::path::Path;

    fn env_or(var: &str, default: &str) -> PathBuf {
        std::env::var_os(var).map(PathBuf::from).unwrap_or_else(|| {
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../..")
                .join(default)
        })
    }

    fn copy_dir(from: &Path, to: &Path) {
        std::fs::create_dir_all(to).unwrap();
        for e in std::fs::read_dir(from).unwrap().flatten() {
            let t = to.join(e.file_name());
            if e.path().is_dir() {
                copy_dir(&e.path(), &t);
            } else {
                std::fs::copy(e.path(), t).unwrap();
            }
        }
    }

    struct Relay(std::process::Child);
    impl Drop for Relay {
        fn drop(&mut self) {
            self.0.kill().ok();
            self.0.wait().ok();
        }
    }

    /// One UI frame with these keys going down (and those of the last frame coming up).
    struct Driver {
        ctx: egui::Context,
        held: Vec<Key>,
        start: Instant,
    }

    impl Driver {
        fn frame(&mut self, shell: &mut Shell, press: &[Key], close: bool) {
            let mut input = RawInput {
                focused: true,
                time: Some(self.start.elapsed().as_secs_f64()),
                ..Default::default()
            };
            let key = |key, pressed| Event::Key {
                key,
                physical_key: None,
                pressed,
                repeat: false,
                modifiers: Default::default(),
            };
            for k in self.held.drain(..) {
                input.events.push(key(k, false));
            }
            for k in press {
                input.events.push(key(*k, true));
                self.held.push(*k);
            }
            if close {
                input.viewports.insert(
                    ViewportId::ROOT,
                    ViewportInfo {
                        events: vec![ViewportEvent::Close],
                        ..Default::default()
                    },
                );
            }
            let _ = self.ctx.run(input, |ctx| shell.step(ctx));
            std::thread::sleep(Duration::from_millis(16));
        }

        /// Frames until `done` (pressing `keys` now and then), or panics after `secs`.
        fn until(
            &mut self,
            shell: &mut Shell,
            secs: u64,
            what: &str,
            keys: &[Key],
            done: impl Fn(&Shell) -> bool,
        ) {
            let end = Instant::now() + Duration::from_secs(secs);
            let mut n = 0u64;
            while !done(shell) {
                assert!(
                    Instant::now() < end,
                    "timed out waiting for {what}; the bridge says: {:?}",
                    shell.view().map(|v| v.message)
                );
                // A key held for 6 frames every 40: the game sees a press.
                let press: Vec<Key> = if !keys.is_empty() && n % 40 == 0 {
                    vec![keys[(n / 40) as usize % keys.len()]]
                } else {
                    vec![]
                };
                self.frame(shell, &press, false);
                n += 1;
            }
        }
    }

    fn message(shell: &Shell) -> String {
        shell
            .lobby
            .as_ref()
            .and_then(|l| l.message.clone())
            .map(|m| m.0)
            .unwrap_or_default()
    }

    #[test]
    #[ignore = "needs the development ROM, the TEAM01 fixture and the relay binary"]
    fn session_leave_saves_and_returns_then_a_backup_restores() {
        let rom = std::fs::read(env_or("GOLDLINK_TEST_ROM", "hg-engine/test.nds"))
            .expect("the development ROM");
        let instance = env_or("GOLDLINK_TEST_INSTANCE", "instances/agent-team-a");
        let campaign = env_or(
            "GOLDLINK_TEST_CAMPAIGN",
            "build/probe-stress/campaigns/TEAM01.json",
        );
        let relay_bin = env_or("GOLDLINK_RELAY", "coop-relay/target/release/coop-relay");

        let dir = std::env::temp_dir().join(format!("goldlink-session-{}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();
        let data = dir.join("data");
        copy_dir(&instance.join("coop_saves"), &data.join("coop_saves"));
        std::fs::copy(
            instance.join("coop_config.json"),
            data.join("coop_config.json"),
        )
        .unwrap();
        // The relay knows the world; its roster takes this install's trainer as it shows up (a
        // legacy roster: no secrets yet).
        let mut c: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&campaign).unwrap()).unwrap();
        for r in c["roster"].as_array_mut().unwrap() {
            r["player_secret"] = 0.into();
        }
        let campaigns = dir.join("campaigns");
        std::fs::create_dir_all(&campaigns).unwrap();
        std::fs::write(campaigns.join("TEAM01.json"), c.to_string()).unwrap();
        let port = 25_000 + (std::process::id() % 500) as u16;
        let _relay = Relay(
            std::process::Command::new(&relay_bin)
                .args(["--bind", &format!("127.0.0.1:{port}"), "--save-dir"])
                .arg(&campaigns)
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .spawn()
                .expect("the relay binary"),
        );
        std::thread::sleep(Duration::from_millis(400));

        let save = data.join("coop_saves/TEAM01-5445414D.sav");
        let before = std::fs::read(&save).unwrap();

        let mut config = Config::default();
        config.audio.muted = true;
        let paths = launcher::Paths {
            install: dir.join("install"),
            data: data.clone(),
        };
        let mut shell = Shell {
            art: None,
            rom: Launcher::new(paths),
            lobby: None,
            bridge: None,
            stage: Stage::Lobby,
            config,
            data: data.clone(),
            game_rom: Some(Arc::new(rom)),
            notes: Vec::new(),
            version: "test".into(),
            auto_join: Some((format!("127.0.0.1:{port}"), "TEAM01".into(), false)),
        };
        shell.bridge().unwrap();
        let mut d = Driver {
            ctx: egui::Context::default(),
            held: Vec::new(),
            start: Instant::now(),
        };
        theme::install(&d.ctx);

        // Join: the game boots hidden, the world's save goes in, then the game takes the window.
        d.until(&mut shell, 60, "the game to be shown", &[], |s| {
            matches!(s.stage, Stage::Playing { .. })
        });
        // Title screen → Continue → the overworld (A and Start now and then).
        d.until(
            &mut shell,
            120,
            "the overworld",
            &[Key::X, Key::Enter, Key::X],
            |s| s.view().is_some_and(|v| v.game_in_session),
        );
        // A backup (a melonDS state), taken at the next safe moment.
        let backups = data.join("coop_backups/TEAM01-5445414D");
        let mlds = |dir: &Path| {
            std::fs::read_dir(dir)
                .map(|d| {
                    d.flatten()
                        .map(|e| e.path())
                        .filter(|p| p.extension().is_some_and(|x| x == "mlds"))
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default()
        };
        goldlink_bridge::hook::lock(shell.bridge.as_ref().unwrap()).backup_soon();
        d.until(&mut shell, 30, "a backup", &[Key::Z], |_| {
            !mlds(&backups).is_empty()
        });
        let backup = mlds(&backups).remove(0);
        // Close whatever the last A opened (B), let it settle, then close the window: Leave.
        for _ in 0..4 {
            d.frame(&mut shell, &[Key::Z], false);
            for _ in 0..30 {
                d.frame(&mut shell, &[], false);
            }
        }
        d.frame(&mut shell, &[], true);
        assert!(
            matches!(shell.stage, Stage::Leaving { .. }),
            "closing the game's window is Leave"
        );
        // While it saves: what the ROM says, every few seconds.
        for i in 0..400 {
            if matches!(shell.stage, Stage::Lobby) {
                break;
            }
            if i % 120 == 0
                && let Some(v) = shell.view()
                && let Some(mb) = &v.mailbox
            {
                use goldlink_bridge::layout::*;
                let lp = &mb[LOCAL_PLAYER..LOCAL_PLAYER + PLAYER_STATE_SIZE];
                eprintln!(
                    "leaving: disconnect_state {} in_session {} connected {} activity {} active {} save_status {}",
                    mb[DISCONNECT_STATE],
                    mb[WAS_IN_ACTIVE_SESSION],
                    mb[IS_CONNECTED_TO_ROOM],
                    lp[PS_ACTIVITY],
                    lp[PS_IS_ACTIVE],
                    lp[PS_SAVE_STATUS]
                );
            }
            // The player closes whatever is open (B), as the bar asks them to.
            let b = if i % 40 == 0 { vec![Key::Z] } else { vec![] };
            d.frame(&mut shell, &b, false);
        }
        d.until(
            &mut shell,
            90,
            "the save and the return to the launcher",
            &[Key::Z],
            |s| matches!(s.stage, Stage::Lobby),
        );
        assert!(
            message(&shell).starts_with("Saved. You left world TEAM01"),
            "{}",
            message(&shell)
        );
        assert_eq!(shell.view().unwrap().link, LinkState::Offline);
        assert_ne!(
            std::fs::read(&save).unwrap(),
            before,
            "the game saved on its way out"
        );

        // A backup, from the launcher: loaded hidden, saved by the game, back to the launcher.
        let saved = std::fs::read(&save).unwrap();
        let bridge = shell.bridge.clone().unwrap();
        let ctx = d.ctx.clone();
        shell.stage = shell.act(&ctx, &bridge, Action::Restore(backup));
        assert!(matches!(shell.stage, Stage::Restoring { .. }));
        d.until(&mut shell, 120, "the restored game to save", &[], |s| {
            matches!(s.stage, Stage::Lobby)
        });
        assert!(
            message(&shell).starts_with("Backup restored and saved"),
            "{}",
            message(&shell)
        );
        assert_ne!(
            std::fs::read(&save).unwrap(),
            saved,
            "the restored game was saved"
        );
        std::fs::remove_dir_all(&dir).ok();
    }
}
