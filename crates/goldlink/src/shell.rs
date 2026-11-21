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

const LAUNCHER_SIZE: egui::Vec2 = egui::vec2(1040.0, 680.0);
/// Tall enough for the whole Play list (Continue at its top) above the avatar, and wide enough
/// for the avatar's name on one line.
const LAUNCHER_MIN: egui::Vec2 = egui::vec2(1000.0, 640.0);
/// A restored backup must be loaded by then, and saved by the ROM after that.
const RESTORE_LOAD: Duration = Duration::from_secs(20);
const RESTORE_SAVE: Duration = Duration::from_secs(90);
/// A join shows the game after this even if it never reached the overworld (UC9 §2).
const REVEAL_ANYWAY: Duration = Duration::from_secs(30);

enum Stage {
    /// The ROM screen.
    Rom,
    Lobby,
    /// The emulator boots hidden and muted while the bridge joins, loads the world's save and
    /// boots straight into the game (UC9 §2). `since`: when the join started.
    Connecting {
        runner: Runner,
        code: String,
        since: Instant,
    },
    /// A started world whose save isn't here (UC8 §2.3): nothing is shown of the game.
    Missing {
        runner: Runner,
        code: String,
        server: String,
        /// Import a save's folder chooser, while it's open.
        choosing: Option<launcher::Choosing>,
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
        /// When the restore began (shown as it goes).
        started: Instant,
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
    /// The window as it was last seen, for opening it again (KNOWN_RISKS #138).
    window: WindowState,
    /// The window's GPU device was lost: this window closes and a new one opens, on a new device.
    reopen: bool,
    /// The window opened once (a panic before is the graphics driver failing to draw it).
    created: bool,
    /// The screen kept on while a game shows (a controller isn't input to the idle timer).
    awake: Option<crate::awake::KeepAwake>,
}

/// What a window opened again keeps of the one before.
#[derive(Clone, Copy)]
struct WindowState {
    size: egui::Vec2,
    fullscreen: bool,
    maximized: bool,
    minimized: bool,
    /// Minimize the new window on its first frame (the old one was).
    minimize_next: bool,
}

impl Default for WindowState {
    fn default() -> Self {
        Self {
            size: LAUNCHER_SIZE,
            fullscreen: false,
            maximized: false,
            minimized: false,
            minimize_next: false,
        }
    }
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
            vec![Box::new(BridgeHook::new(bridge.clone()))],
        ))
    }

    /// A window of this size (the launcher is never fullscreen; the game turns it on after).
    fn resize(ctx: &egui::Context, size: egui::Vec2, min: egui::Vec2) {
        ctx.send_viewport_cmd(ViewportCommand::Fullscreen(false));
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
            since: Instant::now(),
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
            Stage::Connecting {
                runner,
                code,
                since,
            } => self.connecting(ctx, runner, code, since, close),
            Stage::Missing {
                runner,
                code,
                server,
                choosing,
            } => self.missing(ctx, runner, code, server, choosing, close),
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
                // A game that stopped responding can't save: no waiting for it.
                let hung = view.as_ref().is_some_and(|v| v.game_hung);
                forced |= hung;
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
                    // The settings as the player left them in the game.
                    self.config = game.config().clone();
                    game.shutdown();
                    let message = match error {
                        None if hung => (
                            format!(
                                "The game in world {code} had stopped responding, so it couldn't save: restore the newest backup (\"Restore a backup\") to go on."
                            ),
                            true,
                        ),
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
                started,
            } => self.restoring(ctx, runner, since, loaded, started),
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
                        started: Instant::now(),
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
        since: Instant,
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
            return Stage::Connecting {
                runner,
                code,
                since,
            };
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
                choosing: None,
            };
        }
        // The game shows once it's in the overworld (or Oak's speech, for a new game), never
        // during the boot's black screens; after REVEAL_ANYWAY whatever it shows, so a game
        // that never gets there is still in front of the player (UC9 §2).
        if v.slot_ready && (v.game_started || since.elapsed() > REVEAL_ANYWAY) {
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
            // The game fills the screen (the player's setting; F11 switches).
            if self.config.video.fullscreen {
                ctx.send_viewport_cmd(ViewportCommand::Fullscreen(true));
            }
            return Stage::Playing {
                game: Box::new(game),
                code,
            };
        }
        Stage::Connecting {
            runner,
            code,
            since,
        }
    }

    fn missing(
        &mut self,
        ctx: &egui::Context,
        runner: Runner,
        code: String,
        server: String,
        mut choosing: Option<launcher::Choosing>,
        close: bool,
    ) -> Stage {
        let art = self.art.as_ref().expect("loaded");
        let open = choosing.is_some();
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
                        let label = if open {
                            "Choose the folder in the window that opened…"
                        } else {
                            "Import a save…"
                        };
                        theme::button(ui, label, Kind::Primary, r.size(), !open).clicked()
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
            choosing = Some(launcher::choose_folder("Your old GoldLink folder"));
        }
        if let Some(answer) = choosing.as_ref().and_then(|c| c.poll()) {
            choosing = None;
            match answer {
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
            choosing,
        }
    }

    fn restoring(
        &mut self,
        ctx: &egui::Context,
        runner: Runner,
        since: Instant,
        loaded: Option<u32>,
        started: Instant,
    ) -> Stage {
        let art = self.art.as_ref().expect("loaded");
        // Something moves while it works (the user, 2026-11-20: a still screen looked frozen).
        let step = if loaded.is_none() {
            "Loading the backup"
        } else {
            "Saving your game"
        };
        let secs = started.elapsed().as_secs();
        lobby::waiting_screen(
            ctx,
            art,
            "Restoring a backup",
            "Your game goes back to the backup's moment and saves it. This can take up to a minute: please don't close GoldLink.",
            false,
            |ui, _, rect| {
                // A spinner and the step, centred low in the panel.
                let text = format!("{step}{} ({secs} s)", theme::dots(ui));
                let galley = ui.painter().layout_no_wrap(
                    // The dots' room kept, so the line doesn't shift as they come and go.
                    format!("{step}... ({secs} s)"),
                    theme::text(19.0),
                    theme::TEXT,
                );
                let (spin, gap) = (26.0, 12.0);
                let w = spin + gap + galley.size().x;
                let y = rect.bottom() - 70.0;
                let x = rect.center().x - w / 2.0;
                egui::Spinner::new().size(spin).color(theme::GOLD).paint_at(
                    ui,
                    egui::Rect::from_center_size(egui::pos2(x + spin / 2.0, y), vec2(spin, spin)),
                );
                ui.painter().text(
                    egui::pos2(x + spin + gap, y),
                    egui::Align2::LEFT_CENTER,
                    text,
                    theme::text(19.0),
                    theme::TEXT,
                );
            },
        );
        ctx.request_repaint_after(Duration::from_millis(50));
        let view = self.view();
        let in_session = view.as_ref().is_some_and(|v| v.game_in_session);
        let Some(saves) = loaded else {
            if in_session && !view.as_ref().is_some_and(|v| v.restore_pending) {
                return Stage::Restoring {
                    runner,
                    since: Instant::now(),
                    loaded: Some(view.as_ref().map_or(0, |v| v.save_counter)),
                    started,
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
                started,
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
            started,
        }
    }

    /// After a crash, the window gone: a last emergency backup if the game allows one in a
    /// moment (in the overworld, nothing open), then the usual shutdown (the game's save).
    fn crash_exit(&mut self) {
        if let (Stage::Playing { .. } | Stage::Leaving { .. }, Some(b)) =
            (&self.stage, &self.bridge)
        {
            goldlink_bridge::hook::lock(b).backup_soon();
            let until = Instant::now() + Duration::from_secs(3);
            let mut pending = true;
            while pending && Instant::now() < until {
                std::thread::sleep(Duration::from_millis(50));
                pending = goldlink_bridge::hook::lock(b).backup_pending();
            }
            if pending {
                log::warn!("no backup after the crash: the game wasn't at a safe moment");
            } else {
                log::info!("a backup was taken after the crash");
            }
        }
        self.shutdown();
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

impl Shell {
    /// Notes the window's size and state (not while minimized: its size is nothing then).
    fn remember_window(&mut self, ctx: &egui::Context) {
        let w = &mut self.window;
        ctx.input(|i| {
            let v = i.viewport();
            w.minimized = v.minimized == Some(true);
            if w.minimized {
                return;
            }
            w.fullscreen = v.fullscreen == Some(true);
            w.maximized = v.maximized == Some(true);
            if let Some(r) = v
                .inner_rect
                .filter(|r| r.width() >= 64.0 && r.height() >= 64.0)
            {
                w.size = r.size();
            }
        });
    }

    /// The window's least size for what it shows.
    fn min_size(&self) -> egui::Vec2 {
        match &self.stage {
            Stage::Playing { game, .. } | Stage::Leaving { game, .. } => game.window_size(1.0),
            _ => LAUNCHER_MIN,
        }
    }

    /// A new window (on a new GPU device) is opening: what was drawn for the old one is gone.
    fn new_window(&mut self) {
        self.art = None;
        if let Some(l) = self.lobby.as_mut() {
            l.forget_textures();
        }
        if let Stage::Playing { game, .. } | Stage::Leaving { game, .. } = &mut self.stage {
            game.forget_textures();
        }
    }

    fn update(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        let playing = matches!(self.stage, Stage::Playing { .. } | Stage::Leaving { .. });
        if playing != self.awake.is_some() {
            self.awake = playing.then(crate::awake::KeepAwake::start);
        }
        if let Some(rs) = frame.wgpu_render_state() {
            crate::gpu_watch::watch(rs);
            crate::composite::install(rs);
            #[cfg(feature = "dev-hooks")]
            crate::dev_hooks::lose_gpu_tick(rs);
        }
        if crate::gpu_watch::lost() {
            // Nothing more for this window: it closes and run() opens another on a new device.
            if !self.reopen {
                log::warn!("the window's GPU device was lost: opening the window again");
                self.reopen = true;
                ctx.send_viewport_cmd(ViewportCommand::Close);
            }
            return;
        }
        if std::mem::take(&mut self.window.minimize_next) {
            ctx.send_viewport_cmd(ViewportCommand::Minimized(true));
        }
        self.remember_window(ctx);
        self.step(ctx);
        #[cfg(feature = "dev-hooks")]
        {
            crate::dev_hooks::pending(ctx);
            crate::dev_hooks::backup_tick(self.bridge.as_ref());
        }
    }
}

/// The window's view of the shell, which outlives it (a lost GPU device's window is closed and
/// another opened, the shell and its game kept).
struct Window<'s>(&'s mut Shell);

impl eframe::App for Window<'_> {
    fn update(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        self.0.update(ctx, frame);
    }

    fn raw_input_hook(&mut self, _ctx: &egui::Context, raw: &mut egui::RawInput) {
        #[cfg(feature = "dev-hooks")]
        crate::dev_hooks::input(raw);
        // The game's Tab (its touch screen), before egui moves its focus with it.
        if let Stage::Playing { game, .. } | Stage::Leaving { game, .. } = &mut self.0.stage {
            game.take_tab(raw);
        }
    }

    fn on_exit(&mut self) {
        if !self.0.reopen {
            self.0.shutdown();
        }
    }
}

/// How often a window may be opened again before GoldLink gives up on the graphics driver.
const REOPEN_MAX: u32 = 5;

/// run()'s errors after the window opened: said as they are (main.rs), not as a failed start.
pub const CRASHED: &str = "GoldLink closed after an error. If you lost progress since your last \
in-game save, the launcher's \"Restore a backup\" has the newest emergency backups. A crash \
report is in the crashes folder of GoldLink's data folder.";
pub const GPU_GONE: &str = "GoldLink closed: the graphics driver kept losing the window. If you \
lost progress since your last in-game save, the launcher's \"Restore a backup\" has the newest \
emergency backups. Updating the graphics driver may help.";

/// The release program: the ROM screen, the launcher, the game, in one window. `autoplay`
/// presses PLAY at once (`--play`).
pub fn run(autoplay: bool, auto_join: Option<(String, String, bool)>) -> Result<(), String> {
    let paths = launcher::Paths::detect()?;
    let (config, note) = Config::load(&paths.data);
    let data = paths.data.clone();
    let mut rom = Launcher::new(paths);
    rom.autoplay = autoplay;
    let version = rom.version().unwrap_or("?").to_owned();
    // Whether the revamped lighting can run, known before a game for the settings (BACKLOG #12).
    goldlink_light::check_in_background();
    let mut shell = Shell {
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
        window: WindowState::default(),
        reopen: false,
        created: false,
        awake: None,
    };
    let mut opened = 0;
    loop {
        let w = shell.window;
        let mut viewport = egui::ViewportBuilder::default()
            .with_inner_size(w.size)
            .with_min_inner_size(shell.min_size())
            .with_title("GoldLink")
            // matches goldlink.desktop, so desktops show the window under GoldLink's own icon
            .with_app_id("goldlink")
            .with_icon(
                eframe::icon_data::from_png_bytes(include_bytes!("../assets/goldlink.png"))
                    .expect("the app icon is a PNG"),
            );
        if opened > 0 {
            viewport = viewport
                .with_fullscreen(w.fullscreen)
                .with_maximized(w.maximized)
                // Opened again behind the player's back: don't take the focus from what they do.
                .with_active(!w.minimized);
            shell.window.minimize_next = w.minimized;
        }
        let options = eframe::NativeOptions {
            viewport,
            renderer: eframe::Renderer::Wgpu,
            ..Default::default()
        };
        let first = opened == 0;
        let shell_ref = &mut shell;
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            eframe::run_native(
                "GoldLink",
                options,
                Box::new(move |cc| {
                    theme::install(&cc.egui_ctx);
                    shell_ref.created = true;
                    if !first {
                        shell_ref.new_window();
                    }
                    Ok(Box::new(Window(shell_ref)))
                }),
            )
        }));
        opened += 1;
        match result {
            Ok(_) if shell.reopen && opened <= REOPEN_MAX => {
                shell.reopen = false;
                crate::gpu_watch::clear();
                log::warn!("opening the window again ({} of {REOPEN_MAX})", opened);
            }
            Ok(r) => {
                if shell.reopen {
                    shell.reopen = false;
                    shell.shutdown();
                    return Err(GPU_GONE.into());
                }
                return r.map_err(|e| e.to_string());
            }
            Err(_) if !shell.created => {
                return Err("the graphics driver can't draw the window".into());
            }
            Err(_) => {
                // A crash: the window is gone (eframe unwound), the game isn't yet: a last
                // backup and its save before quitting.
                shell.crash_exit();
                return Err(CRASHED.into());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    //! The session's life end to end (UC8 §2.2), in this process: egui without a window, the
    //! real emulator and the real relay. Ignored by default: it needs the development ROM and the
    //! TEAM01 fixture in build/fixtures/session (dev/setup_fixtures.sh: a save in the overworld
    //! and the relay's campaign):
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
        let instance = env_or("GOLDLINK_TEST_INSTANCE", "build/fixtures/session");
        let campaign = env_or(
            "GOLDLINK_TEST_CAMPAIGN",
            "build/fixtures/session/TEAM01.json",
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
            window: WindowState::default(),
            reopen: false,
            created: true,
            awake: None,
        };
        shell.bridge().unwrap();
        let mut d = Driver {
            ctx: egui::Context::default(),
            held: Vec::new(),
            start: Instant::now(),
        };
        theme::install(&d.ctx);

        // Join: the game boots hidden, the world's save goes in, the game boots straight into it
        // (UC9 §2: no intro, title or menu, and no key pressed), and only then takes the window.
        let joined = Instant::now();
        d.until(&mut shell, 60, "the game to be shown", &[], |s| {
            matches!(s.stage, Stage::Playing { .. })
        });
        assert!(
            shell.view().is_some_and(|v| v.game_in_session),
            "the game was shown before it reached the overworld"
        );
        eprintln!(
            "join → overworld, shown: {:.1} s",
            joined.elapsed().as_secs_f64()
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
