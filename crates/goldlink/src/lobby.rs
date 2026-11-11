//! The launcher (UC8 §2.1, §2.4): joining or creating a world, between the ROM screen and the
//! game. The left panel is the PLAY menu with YOU PLAY AS below it; the right panel is the chosen
//! item; the server's status at the top right; the tip line and QUIT at the bottom.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, channel};
use std::time::{Duration, Instant};

use eframe::egui::{self, Rect, RichText, TextureHandle, Ui, UiBuilder, pos2, vec2};
use goldlink_bridge::SharedBridge;
use goldlink_bridge::text;
use goldlink_bridge::worlds::is_world_code;
use goldlink_proto as wire;
use goldlink_runtime::config::{Config, Filter, Layout};

use crate::import::{self, Secret};
use crate::theme::{self, Art, Kind};

/// What the player asked for.
pub enum Action {
    None,
    Join {
        server: String,
        code: String,
        create: bool,
    },
    Restore(PathBuf),
    ChangeRom,
    Quit,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Item {
    Continue,
    Join,
    Create,
    Restore,
    Import,
    Settings,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Status {
    NoServer,
    Checking,
    Online,
    /// A relay of another protocol (its version).
    Other(u16),
    Offline,
}

/// Asks the relay for its handshake challenge, as `scripts/relay_ping.py` does: no world is
/// joined, nothing about the player is sent.
fn probe(server: &str) -> Status {
    let Some(target) = goldlink_bridge::net::resolve(server) else {
        return Status::Offline;
    };
    let bind = if target.addr.is_ipv6() {
        "[::]:0"
    } else {
        "0.0.0.0:0"
    };
    let Ok(sock) = std::net::UdpSocket::bind(bind) else {
        return Status::Offline;
    };
    sock.set_read_timeout(Some(Duration::from_millis(1500)))
        .ok();
    let mut pkt = vec![0u8; wire::CLIENT_PACKET_SIZE];
    pkt[0..4].copy_from_slice(&wire::COOP_MAGIC.to_le_bytes());
    pkt[4..6].copy_from_slice(&wire::PROTOCOL_VERSION.to_le_bytes());
    let mut buf = [0u8; 2048];
    for _ in 0..3 {
        if sock.send_to(&pkt, target.addr).is_err() {
            return Status::Offline;
        }
        if let Ok((n, _)) = sock.recv_from(&mut buf) {
            if n < 13 || buf[0..4] != wire::COOP_MAGIC.to_le_bytes() {
                return Status::Offline;
            }
            let theirs = u16::from_le_bytes([buf[4], buf[5]]);
            return if buf[12] == wire::ROOM_STATUS_ERR_PROTOCOL {
                Status::Other(theirs)
            } else {
                Status::Online
            };
        }
    }
    Status::Offline
}

const TIPS: &[&str] = &[
    "Tip: everyone in a world uses the same server and world code.",
    "Tip: your game saves before you leave a world, and the next time you join it picks up there.",
    "Tip: Esc in the game opens the menu; F3 shows the details.",
    "Tip: a backup of your game is taken every few minutes while you play.",
    "Tip: coming from another computer? Import a save brings your worlds over.",
];

pub struct Lobby {
    item: Item,
    code: String,
    create_code: String,
    server: String,
    /// The latest message (error = red), shown on the tip line.
    pub message: Option<(String, bool)>,
    /// An import found a different player identity: the player decides.
    secret_offer: Option<u64>,
    /// Import a save's folder chooser, while it's open.
    choosing: Option<crate::launcher::Choosing>,
    avatar: Option<(u8, Option<TextureHandle>)>,
    status: Status,
    status_of: String,
    status_rx: Option<Receiver<Status>>,
    status_at: Instant,
}

impl Lobby {
    pub fn new(bridge: &SharedBridge) -> Self {
        let b = goldlink_bridge::hook::lock(bridge);
        let last = b.worlds().entries().first().cloned();
        let server = last
            .as_ref()
            .map(|w| w.server.clone())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| b.config.server.clone());
        Self {
            item: if last.is_some() {
                Item::Continue
            } else {
                Item::Join
            },
            code: String::new(),
            create_code: String::new(),
            server,
            message: None,
            secret_offer: None,
            choosing: None,
            avatar: None,
            status: Status::NoServer,
            status_of: String::new(),
            status_rx: None,
            status_at: Instant::now(),
        }
    }

    /// Back from a world: Continue, the world just played, is at the top again.
    pub fn after_session(&mut self, bridge: &SharedBridge) {
        if !goldlink_bridge::hook::lock(bridge)
            .worlds()
            .entries()
            .is_empty()
        {
            self.item = Item::Continue;
        }
    }

    /// The server's status, checked when the address changes and every 20 seconds.
    fn poll_status(&mut self, ctx: &egui::Context) {
        if let Some(rx) = &self.status_rx {
            if let Ok(s) = rx.try_recv() {
                self.status = s;
                self.status_rx = None;
                self.status_at = Instant::now();
            } else {
                ctx.request_repaint_after(Duration::from_millis(200));
                return;
            }
        }
        let server = self.server.trim().to_owned();
        if server.is_empty() {
            self.status = Status::NoServer;
            self.status_of.clear();
            return;
        }
        let changed = server != self.status_of;
        if changed || self.status_at.elapsed() > Duration::from_secs(20) {
            if changed {
                self.status = Status::Checking;
            }
            self.status_of = server.clone();
            let (tx, rx) = channel();
            std::thread::spawn(move || {
                tx.send(probe(&server)).ok();
            });
            self.status_rx = Some(rx);
        }
        ctx.request_repaint_after(Duration::from_secs(1));
    }

    #[allow(clippy::too_many_arguments)]
    pub fn ui(
        &mut self,
        ctx: &egui::Context,
        art: &Art,
        bridge: &SharedBridge,
        config: &mut Config,
        data: &Path,
        version: &str,
    ) -> Action {
        self.poll_status(ctx);
        let mut action = Action::None;
        egui::CentralPanel::default()
            .frame(egui::Frame::NONE)
            .show(ctx, |ui| {
                let c = theme::chrome(ui, art);
                self.status_bar(ui, c.status, version);
                theme::panel(ui, c.left, "PLAY", theme::EDGE, |ui| {
                    self.menu(ui, art, bridge);
                });
                let (title, accent) = match self.item {
                    Item::Continue => ("Continue", theme::GREEN),
                    Item::Join => ("Join a world", theme::GREEN),
                    Item::Create => ("Create a world", theme::GREEN),
                    Item::Restore => ("Restore a backup", theme::EDGE),
                    Item::Import => ("Import a save", theme::EDGE),
                    Item::Settings => ("Settings", theme::EDGE),
                };
                let a = theme::panel(ui, c.right, title, accent, |ui| {
                    egui::ScrollArea::vertical()
                        .auto_shrink([false, false])
                        .show(ui, |ui| self.item_ui(ui, bridge, config, data))
                        .inner
                });
                if !matches!(a, Action::None) {
                    action = a;
                }
                let (tip, error) = match &self.message {
                    Some((m, e)) => (m.clone(), *e),
                    None => {
                        let i = (ui.input(|i| i.time) / 8.0) as usize % TIPS.len();
                        (TIPS[i].to_owned(), false)
                    }
                };
                theme::tip_line(ui, c.tip, &tip, error);
                if theme::big_button(
                    ui,
                    art,
                    c.button,
                    Some("leave"),
                    "QUIT",
                    Kind::Neutral,
                    true,
                )
                .clicked()
                {
                    action = Action::Quit;
                }
            });
        ctx.request_repaint_after(Duration::from_secs(1));
        action
    }

    fn status_bar(&mut self, ui: &mut Ui, rect: Rect, version: &str) {
        ui.scope_builder(UiBuilder::new().max_rect(rect), |ui| {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let (label, colour) = match self.status {
                    Status::NoServer => ("No server set".to_owned(), theme::TEXT_FAINT),
                    Status::Checking => ("Checking…".to_owned(), theme::AMBER),
                    Status::Online => ("Server online".to_owned(), theme::GREEN_DOT),
                    Status::Other(v) if v > wire::PROTOCOL_VERSION => {
                        ("Update GoldLink".to_owned(), theme::RED)
                    }
                    Status::Other(_) => ("Server outdated".to_owned(), theme::RED),
                    Status::Offline => ("Server offline".to_owned(), theme::RED),
                };
                let hover = match self.status {
                    Status::Other(v) => format!(
                        "The server speaks relay protocol {v}; this GoldLink speaks {}.",
                        wire::PROTOCOL_VERSION
                    ),
                    Status::Offline => {
                        "No answer from the server: is it running, and its UDP port open?".into()
                    }
                    _ => self.status_of.clone(),
                };
                theme::pill(ui, &label, colour).on_hover_text(hover);
                ui.add_space(10.0);
                ui.label(
                    RichText::new(format!("GAME VERSION {version}"))
                        .font(theme::label(14.0))
                        .color(theme::TEXT),
                );
            });
        });
    }

    fn menu(&mut self, ui: &mut Ui, art: &Art, bridge: &SharedBridge) {
        let (last, has_backups) = {
            let b = goldlink_bridge::hook::lock(bridge);
            (
                b.worlds().entries().first().cloned(),
                !b.backups().is_empty(),
            )
        };
        let full = ui.max_rect();
        ui.spacing_mut().item_spacing.y = 2.0;
        let mut row = |ui: &mut Ui, item: Item, icon: &str, label: &str, badge: Option<&str>| {
            if theme::menu_row(ui, art, icon, label, badge, self.item == item).clicked() {
                self.item = item;
                self.message = None;
            }
        };
        if let Some(w) = &last {
            row(ui, Item::Continue, "continue", "Continue", Some(&w.code));
        }
        row(ui, Item::Join, "join", "Join a world", None);
        row(ui, Item::Create, "create", "Create a world", None);
        if has_backups {
            row(ui, Item::Restore, "restore", "Restore a backup", None);
        }
        row(ui, Item::Import, "import", "Import a save", None);
        row(ui, Item::Settings, "settings", "Settings", None);

        // YOU PLAY AS, at the bottom of the panel.
        let card = Rect::from_min_max(pos2(full.left(), full.bottom() - 104.0), full.max);
        ui.painter().line_segment(
            [
                card.left_top() - vec2(0.0, 8.0),
                card.right_top() - vec2(0.0, 8.0),
            ],
            egui::Stroke::new(2.0_f32, theme::EDGE.gamma_multiply(0.5)),
        );
        ui.scope_builder(UiBuilder::new().max_rect(card), |ui| {
            self.avatar_card(ui, bridge)
        });
    }

    fn avatar_card(&mut self, ui: &mut Ui, bridge: &SharedBridge) {
        let (avatars, current) = {
            let b = goldlink_bridge::hook::lock(bridge);
            (b.avatars().to_vec(), b.avatar_id())
        };
        if self.avatar.as_ref().is_none_or(|(id, _)| *id != current) {
            let tex = avatars
                .iter()
                .find(|a| a.id == current)
                .and_then(|a| a.preview.as_deref())
                .and_then(crate::world_ui::load_png)
                .map(|img| {
                    ui.ctx()
                        .load_texture("avatar", img, egui::TextureOptions::NEAREST)
                });
            self.avatar = Some((current, tex));
        }
        let index = avatars.iter().position(|a| a.id == current).unwrap_or(0);
        let name = avatars
            .get(index)
            .map_or("Default", |a| a.name.as_str())
            .to_owned();
        ui.horizontal(|ui| {
            let (pic, _) = ui.allocate_exact_size(vec2(88.0, 88.0), egui::Sense::hover());
            ui.painter()
                .rect_filled(pic, egui::CornerRadius::ZERO, theme::INSET);
            theme::border(ui.painter(), pic, 2.0, theme::EDGE.gamma_multiply(0.6));
            if current == 0 {
                // "Default": Ethan or Lyra, by the save.
                ui.painter().text(
                    pic.center(),
                    egui::Align2::CENTER_CENTER,
                    "?",
                    theme::text(40.0),
                    theme::TEXT_DIM,
                );
            }
            if let Some((_, Some(t))) = &self.avatar {
                let size = t.size_vec2();
                let scale = ((80.0 / size.x.max(size.y)).floor()).max(1.0);
                ui.painter().image(
                    t.id(),
                    egui::Rect::from_center_size(pic.center(), size * scale),
                    egui::Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
                    egui::Color32::WHITE,
                );
            }
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 2.0;
                ui.label(
                    RichText::new("YOU PLAY AS")
                        .font(theme::label(12.0))
                        .color(theme::TEXT_DIM),
                );
                ui.label(
                    RichText::new(&name)
                        .font(theme::text(24.0))
                        .color(theme::TEXT),
                );
                ui.label(
                    RichText::new("How friends see you.")
                        .font(theme::text(14.0))
                        .color(theme::TEXT_DIM),
                );
                if avatars.len() > 1 {
                    ui.horizontal(|ui| {
                        let step = |d: isize| {
                            let n = avatars.len() as isize;
                            avatars[((index as isize + d).rem_euclid(n)) as usize].id
                        };
                        if theme::button(ui, "<", Kind::Neutral, vec2(34.0, 26.0), true).clicked() {
                            goldlink_bridge::hook::lock(bridge).set_avatar(step(-1));
                        }
                        if theme::button(ui, ">", Kind::Neutral, vec2(34.0, 26.0), true).clicked() {
                            goldlink_bridge::hook::lock(bridge).set_avatar(step(1));
                        }
                    });
                }
            });
        });
    }

    fn item_ui(
        &mut self,
        ui: &mut Ui,
        bridge: &SharedBridge,
        config: &mut Config,
        data: &Path,
    ) -> Action {
        ui.spacing_mut().item_spacing.y = 6.0;
        match self.item {
            Item::Continue => self.continue_ui(ui, bridge),
            Item::Join => self.join_ui(ui, bridge),
            Item::Create => self.create_ui(ui, bridge),
            Item::Restore => self.restore_ui(ui, bridge),
            Item::Import => {
                self.import_ui(ui, bridge, data);
                Action::None
            }
            Item::Settings => self.settings_ui(ui, config, data),
        }
    }

    fn server_field(&mut self, ui: &mut Ui) {
        theme::caption(ui, "Server");
        theme::field(
            ui,
            &mut self.server,
            "host:port",
            ui.available_width() - 24.0,
            18.0,
        );
        theme::hint(ui, "The address your world's host gave you.");
    }

    fn continue_ui(&mut self, ui: &mut Ui, bridge: &SharedBridge) -> Action {
        let Some(w) = goldlink_bridge::hook::lock(bridge)
            .worlds()
            .entries()
            .first()
            .cloned()
        else {
            theme::hint(ui, "No worlds yet: join or create one.");
            return Action::None;
        };
        theme::hint(ui, "Back to the world you played last.");
        ui.add_space(4.0);
        theme::inset(ui, theme::GREEN, |ui| {
            ui.label(
                RichText::new("WORLD")
                    .font(theme::label(12.0))
                    .color(theme::TEXT_DIM),
            );
            ui.label(
                RichText::new(&w.code)
                    .font(theme::text(38.0))
                    .color(theme::GOLD),
            );
            let server = if w.server.is_empty() {
                "unknown server".to_owned()
            } else {
                w.server.clone()
            };
            let how = if w.created {
                "You created it"
            } else {
                "You joined it"
            };
            ui.label(
                RichText::new(format!("{how} · {server}"))
                    .font(theme::text(15.0))
                    .color(theme::TEXT_DIM),
            );
            if w.last_played > 0
                && let Some(t) = chrono::DateTime::from_timestamp(w.last_played, 0)
            {
                let t: chrono::DateTime<chrono::Local> = t.into();
                ui.label(
                    RichText::new(format!("Last played {}", t.format("%a %-d %b, %H:%M")))
                        .font(theme::text(15.0))
                        .color(theme::TEXT_DIM),
                );
            }
        });
        ui.add_space(10.0);
        let server = if w.server.is_empty() {
            self.server.clone()
        } else {
            w.server.clone()
        };
        let ok = !server.trim().is_empty();
        if theme::button(
            ui,
            "Continue",
            Kind::Primary,
            vec2(ui.available_width(), 46.0),
            ok,
        )
        .clicked()
        {
            return Action::Join {
                server,
                code: w.code,
                create: false,
            };
        }
        if !ok {
            theme::hint(
                ui,
                "This world's server isn't known: join it from Join a world.",
            );
        }
        Action::None
    }

    fn code_field(ui: &mut Ui, code: &mut String) {
        theme::caption(ui, "World code");
        let r = theme::field(ui, code, "ABC123", 150.0, 30.0);
        if r.changed() {
            *code = code
                .chars()
                .filter(char::is_ascii_alphanumeric)
                .take(6)
                .collect::<String>()
                .to_uppercase();
        }
    }

    fn join_ui(&mut self, ui: &mut Ui, bridge: &SharedBridge) -> Action {
        Self::code_field(ui, &mut self.code);
        theme::hint(ui, "6 letters or numbers, shared by everyone in the world.");
        let worlds: Vec<_> = goldlink_bridge::hook::lock(bridge)
            .worlds()
            .entries()
            .to_vec();
        if !worlds.is_empty() {
            ui.add_space(4.0);
            theme::caption(ui, "Played before");
            for w in worlds.iter().take(4) {
                ui.horizontal(|ui| {
                    let label = format!(
                        "{}  ·  {}",
                        w.code,
                        if w.server.is_empty() {
                            "server unknown"
                        } else {
                            &w.server
                        }
                    );
                    let on = self.code == w.code;
                    if ui
                        .selectable_label(on, RichText::new(label).font(theme::text(16.0)))
                        .clicked()
                    {
                        self.code = w.code.clone();
                        if !w.server.is_empty() {
                            self.server = w.server.clone();
                        }
                    }
                    if ui
                        .small_button("Forget")
                        .on_hover_text("Drops it from the list; its save stays")
                        .clicked()
                    {
                        goldlink_bridge::hook::lock(bridge).forget_world(&w.code);
                    }
                });
            }
        }
        ui.add_space(4.0);
        self.server_field(ui);
        ui.add_space(8.0);
        let ok = is_world_code(&self.code) && !self.server.trim().is_empty();
        if theme::button(
            ui,
            "Join world",
            Kind::Primary,
            vec2(ui.available_width(), 46.0),
            ok,
        )
        .clicked()
        {
            return Action::Join {
                server: self.server.trim().to_owned(),
                code: self.code.clone(),
                create: false,
            };
        }
        Action::None
    }

    fn create_ui(&mut self, ui: &mut Ui, bridge: &SharedBridge) -> Action {
        Self::code_field(ui, &mut self.create_code);
        theme::hint(
            ui,
            "Pick any 6 letters or numbers, then share the code with your friends.",
        );
        let (mut party, mut rules) = {
            let b = goldlink_bridge::hook::lock(bridge);
            (b.config.party_size, b.config.challenge_rules)
        };
        let before = (party, rules);
        ui.add_space(4.0);
        theme::caption(ui, "Players");
        let choices: Vec<(u8, String)> = (2..=6u8).map(|n| (n, n.to_string())).collect();
        let refs: Vec<(u8, &str)> = choices.iter().map(|(n, s)| (*n, s.as_str())).collect();
        theme::segments(ui, &mut party, &refs, ui.available_width() - 8.0);
        theme::hint(
            ui,
            "The adventure starts, and the world locks to these players, once everyone is ready in their bedroom.",
        );
        ui.add_space(4.0);
        theme::caption(ui, "Challenge rules");
        let mut random = (rules & wire::CHALLENGE_RULE_RANDOM_MASK) as usize;
        ui.horizontal(|ui| {
            ui.label("Random encounters");
            egui::ComboBox::from_id_salt("random")
                .selected_text(text::RANDOM_MODES[random])
                .show_ui(ui, |ui| {
                    for (i, m) in text::RANDOM_MODES.iter().enumerate() {
                        ui.selectable_value(&mut random, i, *m);
                    }
                });
        });
        rules = (rules & !wire::CHALLENGE_RULE_RANDOM_MASK) | random as u16;
        let random_on = random != 0;
        let mut flag = |ui: &mut Ui, bit: u16, label: &str, enabled: bool| {
            let mut on = rules & bit != 0;
            if ui
                .add_enabled(enabled, egui::Checkbox::new(&mut on, label))
                .changed()
            {
                rules = if on { rules | bit } else { rules & !bit };
            }
        };
        ui.horizontal(|ui| {
            flag(
                ui,
                wire::CHALLENGE_RULE_LEGENDS_WILD,
                "Legends in the wild",
                random_on,
            );
            flag(
                ui,
                wire::CHALLENGE_RULE_RANDOM_ABILITIES,
                "Random abilities",
                random_on,
            );
        });
        ui.horizontal(|ui| {
            flag(ui, wire::CHALLENGE_RULE_NUZLOCKE, "Nuzlocke", true);
            flag(ui, wire::CHALLENGE_RULE_MONOLOCKE, "Monolocke", true);
        });
        ui.horizontal(|ui| {
            flag(ui, wire::CHALLENGE_RULE_LEVEL_CAP, "Level cap", true);
            flag(ui, wire::CHALLENGE_RULE_RANDOM_ITEMS, "Random items", true);
        });
        let mut soul = wire::soul_link_mode(rules) as usize;
        ui.add_enabled_ui(rules & wire::CHALLENGE_RULE_NUZLOCKE != 0, |ui| {
            ui.horizontal(|ui| {
                ui.label("Soul-Link");
                egui::ComboBox::from_id_salt("soul")
                    .selected_text(text::SOUL_LINK_MODES[soul.min(2)])
                    .show_ui(ui, |ui| {
                        for (i, m) in text::SOUL_LINK_MODES.iter().enumerate() {
                            ui.selectable_value(&mut soul, i, *m);
                        }
                    });
            });
        });
        rules = (rules & !wire::CHALLENGE_RULE_SOUL_LINK_MASK)
            | ((soul as u16) << wire::CHALLENGE_RULE_SOUL_LINK_SHIFT);
        if (party, rules) != before {
            goldlink_bridge::hook::lock(bridge).set_host_options(party, rules);
        }
        theme::hint(
            ui,
            "Everyone in the world plays with these rules. They can't change once the adventure starts.",
        );
        ui.add_space(4.0);
        self.server_field(ui);
        ui.add_space(8.0);
        let ok = is_world_code(&self.create_code) && !self.server.trim().is_empty();
        if theme::button(
            ui,
            "Create world",
            Kind::Primary,
            vec2(ui.available_width(), 46.0),
            ok,
        )
        .clicked()
        {
            return Action::Join {
                server: self.server.trim().to_owned(),
                code: self.create_code.clone(),
                create: true,
            };
        }
        Action::None
    }

    fn restore_ui(&mut self, ui: &mut Ui, bridge: &SharedBridge) -> Action {
        let (list, world) = {
            let b = goldlink_bridge::hook::lock(bridge);
            (b.backups(), b.view().save_line)
        };
        theme::hint(
            ui,
            "Only after a crash or a game-breaking bug your save can't fix. Your game goes back to \
             that moment: everything since is lost, and a trade made since can leave the traded \
             Pokémon in both games. GoldLink then saves it; join your world again to continue.",
        );
        ui.label(
            RichText::new(world)
                .font(theme::text(15.0))
                .color(theme::TEXT_DIM),
        );
        ui.add_space(4.0);
        if list.is_empty() {
            theme::hint(ui, "No backups of your last world yet.");
        }
        for b in list.iter().take(10) {
            let when: chrono::DateTime<chrono::Local> = b.taken.into();
            let label = format!("Restore {}", when.format("%a %-d %b, %H:%M:%S"));
            if theme::button(
                ui,
                &label,
                Kind::Neutral,
                vec2(ui.available_width(), 34.0),
                true,
            )
            .clicked()
            {
                return Action::Restore(b.path.clone());
            }
        }
        Action::None
    }

    fn import_ui(&mut self, ui: &mut Ui, bridge: &SharedBridge, data: &Path) {
        theme::hint(
            ui,
            "Bring your worlds over from another GoldLink: pick its folder (the BizHawk folder of an \
             older GoldLink, or another computer's GoldLink folder). Nothing here is replaced.",
        );
        ui.add_space(6.0);
        if theme::button(
            ui,
            "Choose a folder…",
            Kind::Neutral,
            vec2(ui.available_width(), 40.0),
            self.choosing.is_none(),
        )
        .clicked()
        {
            self.message = None;
            self.choosing = Some(crate::launcher::choose_folder("Your old GoldLink folder"));
        }
        let answer = self.choosing.as_ref().and_then(|c| c.poll());
        if self.choosing.is_some() && answer.is_none() {
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label("Choose the folder in the window that opened…");
            });
            ui.ctx().request_repaint_after(Duration::from_millis(100));
        }
        if let Some(answer) = answer {
            self.choosing = None;
            match answer {
                Ok(Some(dir)) => match import::import(data, &dir) {
                    Ok(report) => {
                        goldlink_bridge::hook::lock(bridge).reload_from_disk();
                        self.secret_offer = match report.secret {
                            Secret::Differs(s) => Some(s),
                            _ => None,
                        };
                        if report.listed > 0 || !report.worlds.is_empty() {
                            self.code = report.worlds.first().cloned().unwrap_or_default();
                        }
                        self.message = Some((report.summary(), false));
                    }
                    Err(e) => self.message = Some((e, true)),
                },
                Ok(None) => {}
                Err(e) => self.message = Some((e, true)),
            }
        }
        if let Some(secret) = self.secret_offer {
            ui.add_space(6.0);
            theme::inset(ui, theme::AMBER, |ui| {
                ui.label(
                    "That folder plays under another player identity. Worlds you played there \
                     only let you back in with it; worlds you played here need this one.",
                );
                ui.horizontal(|ui| {
                    if theme::button(
                        ui,
                        "Use that identity",
                        Kind::Primary,
                        vec2(170.0, 34.0),
                        true,
                    )
                    .clicked()
                    {
                        match import::adopt_secret(data, secret) {
                            Ok(()) => {
                                goldlink_bridge::hook::lock(bridge).reload_from_disk();
                                self.message = Some((
                                    "You now play with that folder's identity.".into(),
                                    false,
                                ));
                            }
                            Err(e) => self.message = Some((e, true)),
                        }
                        self.secret_offer = None;
                    }
                    if theme::button(ui, "Keep mine", Kind::Neutral, vec2(120.0, 34.0), true)
                        .clicked()
                    {
                        self.secret_offer = None;
                    }
                });
            });
        }
    }

    fn settings_ui(&mut self, ui: &mut Ui, config: &mut Config, data: &Path) -> Action {
        let mut changed = false;
        let v = &mut config.video;
        theme::caption(ui, "Screens");
        ui.horizontal(|ui| {
            changed |= ui
                .radio_value(&mut v.layout, Layout::Vertical, "Top and bottom")
                .changed();
            changed |= ui
                .radio_value(&mut v.layout, Layout::Horizontal, "Side by side")
                .changed();
            changed |= ui
                .radio_value(&mut v.layout, Layout::OneScreen, "One screen")
                .on_hover_text(
                    "The other screen shows beside the game when it's needed (Tab: show or hide it)",
                )
                .changed();
        });
        changed |= ui
            .add(egui::Slider::new(&mut v.gap, 0..=96).text("Gap"))
            .changed();
        ui.horizontal(|ui| {
            changed |= ui
                .radio_value(&mut v.filter, Filter::Nearest, "Sharp pixels")
                .changed();
            changed |= ui
                .radio_value(&mut v.filter, Filter::Linear, "Smooth")
                .changed();
        });
        changed |= ui
            .checkbox(&mut v.integer_scale, "Whole-number scaling only")
            .changed();
        theme::caption(ui, "Sound");
        let a = &mut config.audio;
        changed |= ui
            .add(egui::Slider::new(&mut a.volume, 0..=100).text("Volume"))
            .changed();
        changed |= ui.checkbox(&mut a.muted, "Mute").changed();
        theme::caption(ui, "Controls");
        theme::hint(
            ui,
            "A = X or K, B = Z or J, X = V, Y = C, L = Q, R = E, Start = Enter, Select = Space, \
             arrows or WASD, the mouse on the bottom screen. Gamepads work too.",
        );
        if changed && let Err(e) = config.save(data) {
            self.message = Some((format!("Settings not saved: {e}"), true));
        }
        ui.add_space(6.0);
        let w = ui.available_width();
        if theme::button(
            ui,
            "Open your GoldLink folder",
            Kind::Neutral,
            vec2(w, 34.0),
            true,
        )
        .on_hover_text("Saves, settings, backups")
        .clicked()
        {
            crate::launcher::open_folder(data);
        }
        if theme::button(
            ui,
            "Choose another ROM…",
            Kind::Neutral,
            vec2(w, 34.0),
            true,
        )
        .clicked()
        {
            return Action::ChangeRom;
        }
        Action::None
    }
}

/// A screen with one panel in the middle (connecting, a missing save, restoring): the title,
/// a line that moves, what the bridge says, and buttons the caller adds.
pub fn waiting_screen<R>(
    ctx: &egui::Context,
    art: &Art,
    title: &str,
    message: &str,
    error: bool,
    add: impl FnOnce(&mut Ui, &theme::Chrome, Rect) -> R,
) -> R {
    egui::CentralPanel::default()
        .frame(egui::Frame::NONE)
        .show(ctx, |ui| {
            let c = theme::chrome(ui, art);
            let w = (c.main.width() * 0.62).clamp(380.0, 620.0);
            let rect = Rect::from_center_size(
                c.main.center(),
                vec2(w, (c.main.height() * 0.7).max(220.0)),
            );
            theme::panel(
                ui,
                rect,
                title,
                if error { theme::RED } else { theme::EDGE },
                |ui| {
                    ui.label(
                        RichText::new(message)
                            .font(theme::text(19.0))
                            .color(if error { theme::RED } else { theme::TEXT }),
                    );
                },
            );
            theme::tip_line(ui, c.tip, TIPS[0], false);
            add(ui, &c, rect)
        })
        .inner
}
