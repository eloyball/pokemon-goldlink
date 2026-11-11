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
    /// Leave was asked: the game shows until the ROM has saved and is back at the title.
    Leaving {
        game: Box<GameApp>,
        code: String,
        error: Option<String>,
        forced: bool,
    },
    /// A backup goes in, hidden; the ROM saves it and returns to the title.
    Restoring {
        runner: Runner,
        since: Instant,
        loaded: bool,
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
        if let Some(l) = self.lobby.as_mut() {
            if message.is_some() {
                l.message = message;
            }
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
                match lobby.ui(
                    ctx,
                    art,
                    &bridge,
                    &mut self.config,
                    &self.data,
                    &self.version,
                ) {
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
                    Action::Restore(path) => match self.start_emulator(&bridge) {
                        Ok(runner) => {
                            goldlink_bridge::hook::lock(&bridge).restore_backup(path);
                            Stage::Restoring {
                                runner,
                                since: Instant::now(),
                                loaded: false,
                            }
                        }
                        Err(e) => {
                            self.say(e);
                            Stage::Lobby
                        }
                    },
                }
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
                    Stage::Leaving {
                        game,
                        code,
                        error,
                        forced: false,
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
            } => {
                let _ = game.ui(ctx);
                if close {
                    // A second close while saving: stop now (the last in-game save is kept).
                    ctx.send_viewport_cmd(ViewportCommand::CancelClose);
                    forced = true;
                }
                let in_session = self.view().is_some_and(|v| v.game_in_session);
                if in_session && !forced {
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
                        None => (format!("Saved. You left world {code}."), false),
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
        loaded: bool,
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
        if !loaded {
            if in_session {
                return Stage::Restoring {
                    runner,
                    since: Instant::now(),
                    loaded: true,
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
        }
        if !in_session {
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
pub fn run(autoplay: bool) -> Result<(), String> {
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
