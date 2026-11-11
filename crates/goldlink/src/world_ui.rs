//! The world panel: the C# bridge's window, inside the game window (join or create a world,
//! the host's options, the avatar, what's happening in the world, backups).

use eframe::egui::{self, Color32, RichText};
use goldlink_bridge::layout::*;
use goldlink_bridge::text;
use goldlink_bridge::worlds::is_world_code;
use goldlink_bridge::{LinkState, SharedBridge, View};
use goldlink_proto as wire;

const OFFLINE: Color32 = Color32::from_rgb(0x9C, 0xA3, 0xAF);
const CONNECTING: Color32 = Color32::from_rgb(0xF5, 0x9E, 0x0B);
const CONNECTED: Color32 = Color32::from_rgb(0x22, 0xC5, 0x5E);
const ERROR: Color32 = Color32::from_rgb(0xF8, 0x71, 0x71);
const SECONDARY: Color32 = Color32::from_rgb(0xA0, 0xA6, 0xB0);

pub struct WorldPanel {
    pub open: bool,
    host: bool,
    code: String,
    server: String,
    restore_open: bool,
    /// The chosen avatar's preview picture, by avatar ID (None: it has none).
    preview: Option<(u8, Option<egui::TextureHandle>)>,
}

/// A PNG file as an egui image.
fn load_png(path: &std::path::Path) -> Option<egui::ColorImage> {
    let decoder = png::Decoder::new(std::io::BufReader::new(std::fs::File::open(path).ok()?));
    let mut reader = decoder.read_info().ok()?;
    let mut buf = vec![0; reader.output_buffer_size()];
    let info = reader.next_frame(&mut buf).ok()?;
    let (w, h) = (info.width as usize, info.height as usize);
    let px = &buf[..info.buffer_size()];
    let pixels: Vec<egui::Color32> = match (info.color_type, info.bit_depth) {
        (png::ColorType::Rgba, png::BitDepth::Eight) => px
            .chunks_exact(4)
            .map(|c| egui::Color32::from_rgba_unmultiplied(c[0], c[1], c[2], c[3]))
            .collect(),
        (png::ColorType::Rgb, png::BitDepth::Eight) => px
            .chunks_exact(3)
            .map(|c| egui::Color32::from_rgb(c[0], c[1], c[2]))
            .collect(),
        _ => return None,
    };
    (pixels.len() == w * h).then_some(egui::ColorImage {
        size: [w, h],
        pixels,
    })
}

impl WorldPanel {
    pub fn new(bridge: &SharedBridge) -> Self {
        let b = goldlink_bridge::hook::lock(bridge);
        Self {
            open: true,
            host: false,
            code: String::new(),
            server: b.config.server.clone(),
            restore_open: false,
            preview: None,
        }
    }

    pub fn show(&mut self, ctx: &egui::Context, bridge: &SharedBridge) {
        if !self.open {
            return;
        }
        let view = goldlink_bridge::hook::lock(bridge).view();
        let mut open = self.open;
        egui::Window::new("World")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .default_width(380.0)
            .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
            .show(ctx, |ui| {
                ui.set_max_width(380.0);
                status_line(ui, &view);
                ui.add_space(6.0);
                if view.link == LinkState::Connected {
                    self.connected(ui, &view, bridge);
                } else {
                    self.offline(ui, &view, bridge);
                }
            });
        self.open &= open;
        if self.restore_open {
            self.restore_dialog(ctx, bridge);
        }
    }

    fn offline(&mut self, ui: &mut egui::Ui, view: &View, bridge: &SharedBridge) {
        let busy = view.link != LinkState::Offline;
        // Protocol 9 (QUIC by default): the world's relay presented another certificate than
        // the one pinned when this world was first played. Never re-pinned silently.
        if let Some(fp) = &view.identity_change {
            ui.label("The server now presents this certificate (SHA-256):");
            let grouped: Vec<&str> = (0..fp.len())
                .step_by(8)
                .map(|i| &fp[i..(i + 8).min(fp.len())])
                .collect();
            ui.monospace(grouped.join(" "));
            if ui
                .button("Trust the new identity")
                .on_hover_text(
                    "Only if the world's host says they replaced the relay's certificate",
                )
                .clicked()
            {
                goldlink_bridge::hook::lock(bridge).trust_new_identity();
            }
            ui.add_space(6.0);
        }
        ui.add_enabled_ui(!busy, |ui| {
            ui.horizontal(|ui| {
                ui.selectable_value(&mut self.host, false, "Join a world");
                ui.selectable_value(&mut self.host, true, "Create a world");
            });
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.label("World code");
                let edit = ui.add(
                    egui::TextEdit::singleline(&mut self.code)
                        .desired_width(90.0)
                        .char_limit(6),
                );
                if edit.changed() {
                    self.code = self
                        .code
                        .chars()
                        .filter(char::is_ascii_alphanumeric)
                        .collect::<String>()
                        .to_uppercase();
                }
                let worlds: Vec<_> = goldlink_bridge::hook::lock(bridge)
                    .worlds()
                    .entries()
                    .to_vec();
                egui::ComboBox::from_id_salt("worlds")
                    .selected_text("Played before")
                    .width(140.0)
                    .show_ui(ui, |ui| {
                        if worlds.is_empty() {
                            ui.label("None yet");
                        }
                        for w in &worlds {
                            if ui.selectable_label(false, w.label()).clicked() {
                                self.code = w.code.clone();
                                if !w.server.is_empty() {
                                    self.server = w.server.clone();
                                }
                                self.host = false; // a world that exists can't be created again
                            }
                        }
                    });
                let known = worlds.iter().any(|w| w.code == self.code);
                if known
                    && ui
                        .small_button("Forget")
                        .on_hover_text("Drops it from the list; its save stays")
                        .clicked()
                {
                    goldlink_bridge::hook::lock(bridge).forget_world(&self.code);
                }
            });
            ui.horizontal(|ui| {
                ui.label("Server");
                ui.add(
                    egui::TextEdit::singleline(&mut self.server)
                        .hint_text("host:port")
                        .desired_width(220.0),
                );
            });
            self.avatar_picker(ui, bridge);
            if self.host {
                self.host_options(ui, bridge);
            }
        });
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            let label = match view.link {
                LinkState::Resolving | LinkState::Connecting => "Connecting…",
                _ if self.host => "Create world",
                _ => "Join world",
            };
            let ok = !busy && is_world_code(&self.code) && !self.server.trim().is_empty();
            if ui
                .add_enabled(ok, egui::Button::new(RichText::new(label).strong()))
                .clicked()
            {
                goldlink_bridge::hook::lock(bridge).join(&self.server, &self.code, self.host);
            }
            if busy && ui.button("Cancel").clicked() {
                goldlink_bridge::hook::lock(bridge).leave();
            }
            if !busy && view.has_backups && ui.button("Restore a backup…").clicked() {
                self.restore_open = true;
            }
        });
        if let Some(mb) = &view.mailbox {
            ui.separator();
            game_lines(ui, view, mb);
        }
    }

    fn avatar_picker(&mut self, ui: &mut egui::Ui, bridge: &SharedBridge) {
        let (avatars, current) = {
            let b = goldlink_bridge::hook::lock(bridge);
            (b.avatars().to_vec(), b.avatar_id())
        };
        if self.preview.as_ref().is_none_or(|(id, _)| *id != current) {
            let tex = avatars
                .iter()
                .find(|a| a.id == current)
                .and_then(|a| a.preview.as_deref())
                .and_then(load_png)
                .map(|img| {
                    ui.ctx()
                        .load_texture("avatar", img, egui::TextureOptions::NEAREST)
                });
            self.preview = Some((current, tex));
        }
        let name = avatars
            .iter()
            .find(|a| a.id == current)
            .map_or("Default", |a| a.name.as_str())
            .to_owned();
        ui.horizontal(|ui| {
            ui.label("Avatar");
            egui::ComboBox::from_id_salt("avatar")
                .selected_text(name)
                .show_ui(ui, |ui| {
                    for a in &avatars {
                        if ui.selectable_label(a.id == current, &a.name).clicked() {
                            goldlink_bridge::hook::lock(bridge).set_avatar(a.id);
                        }
                    }
                });
            if let Some((_, Some(tex))) = &self.preview {
                let size = tex.size_vec2();
                let scale = 48.0 / size.y.max(1.0);
                ui.image((tex.id(), size * scale));
            }
        });
    }

    fn host_options(&mut self, ui: &mut egui::Ui, bridge: &SharedBridge) {
        let (mut party, mut rules) = {
            let b = goldlink_bridge::hook::lock(bridge);
            (b.config.party_size, b.config.challenge_rules)
        };
        let before = (party, rules);
        ui.group(|ui| {
            ui.horizontal(|ui| {
                ui.label("Players");
                for n in 2..=6u8 {
                    ui.selectable_value(&mut party, n, n.to_string());
                }
            });
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
            let mut flag = |ui: &mut egui::Ui, bit: u16, label: &str, enabled: bool| {
                let mut on = rules & bit != 0;
                if ui
                    .add_enabled(enabled, egui::Checkbox::new(&mut on, label))
                    .changed()
                {
                    rules = if on { rules | bit } else { rules & !bit };
                }
            };
            let random_on = random != 0;
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
        });
        if (party, rules) != before {
            goldlink_bridge::hook::lock(bridge).set_host_options(party, rules);
        }
    }

    fn connected(&mut self, ui: &mut egui::Ui, view: &View, bridge: &SharedBridge) {
        if let Some(mb) = &view.mailbox {
            game_lines(ui, view, mb);
            let g = GATES;
            let started = mb[g + GATE_CAMPAIGN_STARTED] == 1;
            let (online, expected, ready) = (
                mb[g + GATE_ONLINE_PARTY],
                mb[g + GATE_EXPECTED_PARTY],
                mb[g + GATE_READY_PARTY],
            );
            row(
                ui,
                "Players",
                &if started {
                    format!("{online} of {expected}")
                } else {
                    format!("{online} of {expected} · {ready} ready in bedroom")
                },
                None,
            );
            row(
                ui,
                "Adventure",
                if started {
                    "Started · world locked"
                } else {
                    "Waiting for everyone in the bedroom"
                },
                Some(if started { CONNECTED } else { CONNECTING }),
            );
            match view.room_rules {
                Some((rules, mono)) => row(ui, "Rules", &text::describe_rules(rules, mono), None),
                None => row(ui, "Rules", "Waiting for the server…", Some(SECONDARY)),
            }
            let open = mb[g + GATE_SYNC_READY] == 1;
            ui.horizontal(|ui| {
                row(
                    ui,
                    "Gym gate",
                    if open {
                        "Open for you"
                    } else {
                        "Closed · a teammate is behind on badges"
                    },
                    Some(if open { CONNECTED } else { SECONDARY }),
                );
            });
            let mut ahead = goldlink_bridge::hook::lock(bridge).allow_ahead;
            if ui
                .checkbox(
                    &mut ahead,
                    "Let teammates challenge bosses while I'm behind",
                )
                .changed()
            {
                goldlink_bridge::hook::lock(bridge).allow_ahead = ahead;
            }
            let partner = mb[g + GATE_TEAM_PARTNER];
            let mut lines = Vec::new();
            for i in 0..REMOTE_SLOTS {
                let o = SERVER_PAYLOAD + i * PLAYER_STATE_SIZE;
                let p = &mb[o..o + PLAYER_STATE_SIZE];
                if u32_at(p, PS_TRAINER_ID) == 0 {
                    continue;
                }
                let tag = if i as u8 == partner {
                    " (teammate)"
                } else {
                    ""
                };
                lines.push(format!(
                    "{}{tag}: {}",
                    text::decode_name(p),
                    text::describe_player(p)
                ));
            }
            if lines.is_empty() {
                row(ui, "Others", "Nobody else yet", Some(SECONDARY));
            } else {
                row(ui, "Others", &lines.join("\n"), None);
            }
        }
        ui.add_space(6.0);
        if ui.button("Leave the world").clicked() {
            goldlink_bridge::hook::lock(bridge).leave();
        }
    }

    fn restore_dialog(&mut self, ctx: &egui::Context, bridge: &SharedBridge) {
        let list = goldlink_bridge::hook::lock(bridge).backups();
        let mut open = true;
        egui::Window::new("Restore a backup").open(&mut open).collapsible(false).show(ctx, |ui| {
            ui.label("Use this only after a crash or a game-breaking bug that your save can't fix. Your game goes back to that moment: everything since then is lost, and a trade made since then can leave the traded Pokémon in both games. The game then saves and returns to the title screen; join your world again to continue.");
            ui.separator();
            if list.is_empty() {
                ui.label("There are no backups of your last world yet. One is taken every few minutes while you play in a world.");
            }
            for b in list.iter().take(12) {
                let when: chrono::DateTime<chrono::Local> = b.taken.into();
                if ui.button(format!("Restore {}", when.format("%a %-d %b, %H:%M:%S"))).clicked() {
                    goldlink_bridge::hook::lock(bridge).restore_backup(b.path.clone());
                    self.restore_open = false;
                }
            }
        });
        self.restore_open &= open;
    }
}

fn status_line(ui: &mut egui::Ui, view: &View) {
    let (text, color) = match view.link {
        LinkState::Connected => (
            match view.slot {
                Some(s) => format!("Connected · Player {} · {}", s + 1, view.room),
                None => "Connected".into(),
            },
            CONNECTED,
        ),
        LinkState::Resolving | LinkState::Connecting => ("Connecting".into(), CONNECTING),
        LinkState::Offline => ("Offline".into(), OFFLINE),
    };
    ui.horizontal(|ui| {
        // The default fonts have no "●": draw the dot.
        let (rect, _) = ui.allocate_exact_size(egui::vec2(10.0, 14.0), egui::Sense::hover());
        ui.painter().circle_filled(rect.center(), 4.5, color);
        ui.label(RichText::new(text).strong().color(color));
    });
    ui.label(
        RichText::new(&view.message).color(if view.message_is_error {
            ERROR
        } else {
            SECONDARY
        }),
    );
}

fn row(ui: &mut egui::Ui, label: &str, value: &str, color: Option<Color32>) {
    ui.horizontal_top(|ui| {
        ui.add_sized(
            [86.0, 16.0],
            egui::Label::new(RichText::new(label).color(SECONDARY)),
        );
        let mut t = RichText::new(value);
        if let Some(c) = color {
            t = t.color(c);
        }
        ui.label(t);
    });
}

/// The game's own lines: running, trainer, location, save.
fn game_lines(ui: &mut egui::Ui, view: &View, mb: &[u8]) {
    let (game, color) = match mb[DISCONNECT_STATE] {
        1 => (
            "Disconnected · will save at the next safe moment",
            CONNECTING,
        ),
        2 => ("Saving and returning to the title screen…", CONNECTING),
        _ => ("Running", CONNECTED),
    };
    row(ui, "Game", game, Some(color));
    let lp = &view.local_player;
    let otid = u32_at(lp, PS_TRAINER_ID);
    if otid != 0 {
        row(
            ui,
            "Trainer",
            &format!("{} · {:05}", text::decode_name(lp), otid & 0xFFFF),
            None,
        );
        row(ui, "Location", &text::describe_player(lp), None);
    } else {
        row(ui, "Trainer", "No trainer yet", Some(SECONDARY));
    }
    let save_color = if view.save_line.starts_with("Error") {
        ERROR
    } else {
        SECONDARY
    };
    row(ui, "Save", &view.save_line, Some(save_color));
}
