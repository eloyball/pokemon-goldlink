//! The bridge's per-frame work (the C# bridge's `UpdateValues` and the connection lifecycle),
//! ported step for step. It runs on the emulation thread after every frame; the window calls
//! [`Bridge::join`] / [`Bridge::leave`] and reads [`Bridge::view`].

use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, TryRecvError};
use std::time::{Duration, Instant};

use goldlink_proto as wire;

use crate::avatars::{self, AvatarChoice};
use crate::config::{NetConfig, random_u64};
use crate::layout::*;
use crate::net::{self, RelayClient};
use crate::sanitize::{self, Limits};
use crate::slots::{self, SaveSlots};
use crate::text;
use crate::worlds::{WorldList, is_world_code};
use goldlink_runtime::backups::{self, Backups};

/// Receives every message the bridge shows: `(text, is_error)`.
pub type StatusListener = Box<dyn FnMut(&str, bool) + Send>;

/// What the bridge needs from the emulator, between frames.
pub trait Machine {
    fn main_ram(&mut self) -> &mut [u8];
    /// The cart's save memory.
    fn cart(&mut self) -> &mut [u8];
    fn savestate(&mut self) -> Option<Vec<u8>>;
    fn load_savestate(&mut self, state: &[u8]) -> Result<(), String>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LinkState {
    Offline,
    Resolving,
    Connecting,
    Connected,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SlotState {
    None,
    Swapping,
    Ready,
}

/// Things the emulator side must do after a frame.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Actions {
    /// The bridge put another world's save into the cart: write the cart's save file now, or a
    /// restart before the next in-game save would bring the old one back.
    pub cart_replaced: bool,
    pub backup_taken: Option<PathBuf>,
    pub backup_failed: Option<String>,
}

/// A snapshot for the window's world panel.
#[derive(Debug, Clone)]
pub struct View {
    pub link: LinkState,
    pub slot: Option<u8>,
    pub message: String,
    pub message_is_error: bool,
    /// The mailbox, when the ROM has published it.
    pub mailbox: Option<Vec<u8>>,
    pub local_player: [u8; PLAYER_STATE_SIZE],
    pub room: String,
    pub room_rules: Option<(u16, u8)>,
    pub save_line: String,
    pub has_backups: bool,
}

const SERVER_TIMEOUT: Duration = Duration::from_secs(3);
/// Challenges prove the relay is reachable, but must lead to a real OK packet within this long
/// (a NAT that changes our port per packet would otherwise keep us "connecting" forever).
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);
const SWAP_RETRY: Duration = Duration::from_secs(10);

pub struct Bridge {
    dir: PathBuf,
    pub config: NetConfig,
    avatars: Vec<AvatarChoice>,
    avatar_id: u8,
    net: RelayClient,
    resolving: Option<Receiver<Option<SocketAddr>>>,
    pending_join: Option<(String, String, bool)>,

    link: LinkState,
    host_mode: bool,
    room_code: [u8; 6],
    active_room: String,
    active_server: String,
    active_action: u8,
    host_token: u32,
    assigned_slot: u8,
    sequence: u32,
    bridge_heartbeat: u32,
    last_server_packet: Instant,
    clear_payload_pending: bool,
    /// Gym gate: teammates may challenge bosses while this player is behind on badges.
    pub allow_ahead: bool,

    local_player: [u8; PLAYER_STATE_SIZE],
    session_out: [u8; SESSION_OUT_SIZE],
    trade_out: Option<[u8; TRADE_OUT_SIZE]>,
    challenge_up: Option<[u8; wire::CHALLENGE_UP_SIZE]>,
    badge_mask_local: u16,
    heartbeat_rom: u32,
    limits: Limits,
    room_rules: Option<(u16, u8)>,

    slots: SaveSlots,
    slot_state: SlotState,
    slot_key: Option<String>,
    last_save_counter: Option<u32>,
    last_rom_heartbeat: u32,
    swap_requested: Instant,
    backups: Backups,
    next_backup: Option<Instant>,
    pending_restore: Option<PathBuf>,
    worlds: WorldList,

    message: String,
    message_is_error: bool,
    mailbox_copy: Option<Vec<u8>>,
    /// Called with every message the bridge shows (dev tools log them).
    pub status_listener: Option<StatusListener>,
}

impl Bridge {
    /// `dir` holds `coop_config.json`, `coop_worlds.json`, `coop_saves/` and `coop_backups/`
    /// (the BizHawk bridge's config folder). `avatar_files` are where `coop_avatars.json` may be.
    pub fn new(dir: &Path, avatar_files: &[PathBuf]) -> std::io::Result<Self> {
        let config = NetConfig::load_or_create(dir);
        let avatars = avatars::load(avatar_files);
        let avatar_id = avatars
            .iter()
            .find(|a| a.key == config.avatar)
            .map_or(0, |a| a.id);
        let saves = dir.join("coop_saves");
        Ok(Self {
            dir: dir.to_owned(),
            avatars,
            avatar_id,
            net: RelayClient::new()?,
            resolving: None,
            pending_join: None,
            link: LinkState::Offline,
            host_mode: false,
            room_code: [0; 6],
            active_room: String::new(),
            active_server: String::new(),
            active_action: 0,
            host_token: 0,
            assigned_slot: PLAYER_ID_NONE,
            sequence: 0,
            bridge_heartbeat: 0,
            last_server_packet: Instant::now(),
            clear_payload_pending: true,
            allow_ahead: false,
            local_player: [0; PLAYER_STATE_SIZE],
            session_out: [0; SESSION_OUT_SIZE],
            trade_out: None,
            challenge_up: None,
            badge_mask_local: 0,
            heartbeat_rom: 0,
            limits: Limits {
                map_id: 0,
                species: 0,
            },
            room_rules: None,
            slots: SaveSlots::new(&saves),
            slot_state: SlotState::None,
            slot_key: None,
            last_save_counter: None,
            last_rom_heartbeat: 0,
            swap_requested: Instant::now(),
            backups: Backups::new(dir.join("coop_backups")),
            next_backup: None,
            pending_restore: None,
            worlds: WorldList::new(dir, &saves),
            message: text::DEFAULT_MESSAGE.into(),
            message_is_error: false,
            mailbox_copy: None,
            status_listener: None,
            config,
        })
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    pub fn link(&self) -> LinkState {
        self.link
    }

    pub fn worlds(&self) -> &WorldList {
        &self.worlds
    }

    pub fn forget_world(&mut self, code: &str) {
        self.worlds.forget(code);
    }

    pub fn avatars(&self) -> &[AvatarChoice] {
        &self.avatars
    }

    pub fn avatar_id(&self) -> u8 {
        self.avatar_id
    }

    /// Picks the avatar (offline only); remembered by key.
    pub fn set_avatar(&mut self, id: u8) {
        if self.link != LinkState::Offline {
            return;
        }
        if let Some(a) = self.avatars.iter().find(|a| a.id == id) {
            self.avatar_id = a.id;
            self.config.avatar = a.key.clone();
            self.config.save(&self.dir).ok();
        }
    }

    /// The host's world settings (offline only).
    pub fn set_host_options(&mut self, party_size: u8, rules: u16) {
        if self.link != LinkState::Offline {
            return;
        }
        self.config.party_size = party_size.clamp(2, MAX_PLAYERS as u8);
        self.config.challenge_rules = wire::sanitize_challenge_rules(rules);
    }

    /// The local player's current activity (CoopActivity), from the last snapshot.
    pub fn local_activity(&self) -> u8 {
        self.local_player[PS_ACTIVITY]
    }

    fn set_status(&mut self, message: impl Into<String>, is_error: bool) {
        self.message = message.into();
        self.message_is_error = is_error;
        if let Some(l) = self.status_listener.as_mut() {
            l(&self.message, is_error);
        }
    }

    /// Join (`create` false) or create a world on `server`. Looked up off the emulation thread.
    pub fn join(&mut self, server: &str, code: &str, create: bool) {
        if self.link != LinkState::Offline || !is_world_code(code) {
            return;
        }
        let server = server.trim().to_owned();
        if server.is_empty() {
            // No address is built in (Milestone 6 §4.3): the world's host shares theirs.
            self.set_status(
                "Enter the server address (host:port) your world's host gave you.",
                true,
            );
            return;
        }
        self.link = LinkState::Resolving;
        self.host_mode = create;
        self.set_status(format!("Looking up {server}…"), false);
        let (tx, rx) = std::sync::mpsc::channel();
        let s = server.clone();
        std::thread::spawn(move || {
            tx.send(net::resolve(&s)).ok();
        });
        self.resolving = Some(rx);
        self.pending_join = Some((server, code.to_owned(), create));
    }

    fn poll_resolve(&mut self) {
        let Some(rx) = &self.resolving else { return };
        let result = match rx.try_recv() {
            Ok(r) => r,
            Err(TryRecvError::Empty) => return,
            Err(TryRecvError::Disconnected) => None,
        };
        self.resolving = None;
        let Some((server, code, create)) = self.pending_join.take() else {
            return;
        };
        if self.link != LinkState::Resolving {
            return; // left meanwhile
        }
        let Some(addr) = result else {
            self.link = LinkState::Offline;
            self.set_status(
                format!(
                    "Couldn't find the server \"{server}\". Use host:port, e.g. 127.0.0.1:7777."
                ),
                true,
            );
            return;
        };
        self.net.set_server(addr);
        self.room_code.copy_from_slice(code.as_bytes());
        self.active_room = code.clone();
        self.active_server = server.clone();
        self.active_action = if create {
            wire::ROOM_ACTION_CREATE
        } else {
            wire::ROOM_ACTION_JOIN
        };
        self.assigned_slot = PLAYER_ID_NONE;
        self.last_server_packet = Instant::now();
        self.begin_new_session();
        // A fresh host token on every create attempt.
        self.host_token = if create {
            (random_u64() as u32).max(1)
        } else {
            0
        };
        self.link = LinkState::Connecting;
        self.set_status(
            if create {
                format!("Creating world {code}…")
            } else {
                format!("Joining world {code}…")
            },
            false,
        );
        self.config.server = server;
        self.config.save(&self.dir).ok();
    }

    /// Leaves the world (the Leave button).
    pub fn leave(&mut self) {
        self.disconnect("Left the world.", false);
    }

    /// Leaves with another message (dev tools say why).
    pub fn leave_with(&mut self, message: &str) {
        self.disconnect(message, false);
    }

    fn disconnect(&mut self, reason: &str, is_error: bool) {
        if matches!(self.link, LinkState::Connecting | LinkState::Connected) {
            // One-shot LEAVE so the relay frees the slot at once.
            let pkt = self.build_client_packet(wire::ROOM_ACTION_LEAVE);
            self.net.send(&pkt);
        }
        self.link = LinkState::Offline;
        self.resolving = None;
        self.pending_join = None;
        self.next_backup = None;
        self.assigned_slot = PLAYER_ID_NONE;
        self.active_room.clear();
        self.clear_payload_pending = true;
        self.begin_new_session();
        self.set_status(reason.to_owned(), is_error);
    }

    fn begin_new_session(&mut self) {
        self.net.begin_session();
        self.slot_key = None;
        self.slot_state = SlotState::None;
    }

    /// The emulator powered on again: the mailbox comes back empty.
    pub fn on_reboot(&mut self) {
        self.clear_payload_pending = true;
    }

    /// The world's backups (offline: the last world's), newest first.
    pub fn backups(&self) -> Vec<backups::Backup> {
        let key = self
            .slot_key
            .clone()
            .unwrap_or_else(|| self.slots.owner().to_owned());
        if key.is_empty() {
            Vec::new()
        } else {
            self.backups.list(&key)
        }
    }

    /// Restores a backup on the next frame (offline only: the restored game still thinks it's in
    /// the world, so the ROM's disconnect flow saves it and returns to the title).
    pub fn restore_backup(&mut self, path: PathBuf) {
        if self.link == LinkState::Offline {
            self.pending_restore = Some(path);
        }
    }

    // ------------------------------------------------------------------
    // After every frame (the emulation thread)
    // ------------------------------------------------------------------

    pub fn post_frame(&mut self, m: &mut dyn Machine, mailbox: usize) -> Actions {
        let mut actions = Actions::default();
        self.poll_resolve();

        if let Some(path) = self.pending_restore.take() {
            let loaded = std::fs::read(&path)
                .map_err(|e| e.to_string())
                .and_then(|s| m.load_savestate(&s));
            match loaded {
                Ok(()) => self.set_status(
                    "Backup restored. The game saves it and returns to the title screen; then join your world again.",
                    false,
                ),
                Err(_) => self.set_status("Could not load that backup.", true),
            }
            return actions;
        }

        let ram_len = m.main_ram().len();
        if mailbox + MAILBOX_SIZE > ram_len {
            self.mailbox_copy = None;
            return actions;
        }
        {
            let ram = m.main_ram();
            let mb = &ram[mailbox..mailbox + MAILBOX_SIZE];
            if u32_at(mb, MAGIC) != goldlink_abi::COOP_MAGIC
                || u32::from(u16_at(mb, VERSION)) != goldlink_abi::COOP_MAILBOX_VERSION
            {
                self.mailbox_copy = None;
                return actions;
            }
            self.limits = Limits {
                map_id: u16_at(mb, LIMIT_MAP_ID),
                species: u16_at(mb, LIMIT_SPECIES),
            };
        }

        // Tell the ROM we're alive (its disconnect watchdog), and our own look.
        self.bridge_heartbeat = self.bridge_heartbeat.wrapping_add(1);
        {
            let ram = m.main_ram();
            put_u32(ram, mailbox + HEARTBEAT_BRIDGE, self.bridge_heartbeat);
            ram[mailbox + LOCAL_AVATAR] = self.avatar_id;
        }
        self.read_local_player(m.main_ram(), mailbox);
        if self.update_save_slot(m, mailbox) {
            actions.cart_replaced = true;
        }

        if !matches!(self.link, LinkState::Connecting | LinkState::Connected) {
            self.clear_rom_connection_state(m.main_ram(), mailbox, self.clear_payload_pending);
            self.copy_mailbox(m.main_ram(), mailbox);
            return actions;
        }

        self.process_latest_server_packet(m.main_ram(), mailbox);
        if self.link == LinkState::Offline {
            self.copy_mailbox(m.main_ram(), mailbox);
            return actions; // the server turned us down
        }
        self.try_take_backup(m, mailbox, &mut actions);

        let now = Instant::now();
        let handshaking = self
            .net
            .last_challenge()
            .is_some_and(|t| now.duration_since(t) <= SERVER_TIMEOUT)
            && now.duration_since(self.last_server_packet) <= HANDSHAKE_TIMEOUT;
        if !handshaking && now.duration_since(self.last_server_packet) > SERVER_TIMEOUT {
            // Never connected: no answer at all. Besides a wrong address or a closed port, an old
            // relay drops another version's packets without a word.
            let reason = if self.link == LinkState::Connecting {
                text::no_answer()
            } else {
                "Lost the connection to the server.".into()
            };
            self.disconnect(&reason, true);
            self.clear_rom_connection_state(m.main_ram(), mailbox, true);
            self.copy_mailbox(m.main_ram(), mailbox);
            return actions;
        }

        let pkt = self.build_client_packet(self.active_action);
        self.net.send(&pkt);
        self.copy_mailbox(m.main_ram(), mailbox);
        actions
    }

    fn copy_mailbox(&mut self, ram: &[u8], mailbox: usize) {
        let mb = &ram[mailbox..mailbox + MAILBOX_SIZE];
        match &mut self.mailbox_copy {
            Some(v) => v.copy_from_slice(mb),
            None => self.mailbox_copy = Some(mb.to_vec()),
        }
    }

    fn read_local_player(&mut self, ram: &[u8], mb: usize) {
        // Sequence lock: keep the previous snapshot if the ROM was mid-write when the frame
        // ended (seq odd). RunFrame returns at a frame boundary, not where the ROM's writer is
        // quiescent (RFC §4.2).
        let seq = u32_at(ram, mb + LOCAL_PLAYER_SEQ);
        if seq & 1 == 0 {
            self.local_player
                .copy_from_slice(&ram[mb + LOCAL_PLAYER..mb + LOCAL_PLAYER + PLAYER_STATE_SIZE]);
            self.badge_mask_local = u16_at(ram, mb + BADGE_MASK_LOCAL);
        }
        self.heartbeat_rom = u32_at(ram, mb + HEARTBEAT_ROM);
        // The ROM fills the session block before bumping its seq byte, between frames.
        self.session_out
            .copy_from_slice(&ram[mb + SESSION_OUT..mb + SESSION_OUT + SESSION_OUT_SIZE]);
        self.read_challenge_out(ram);
        self.trade_out = find_exchange(ram, mb).map(|ex| {
            let mut t = [0u8; TRADE_OUT_SIZE];
            t.copy_from_slice(&ram[ex + EXCHANGE_OUT..ex + EXCHANGE_OUT + TRADE_OUT_SIZE]);
            t
        });
    }

    /// The ROM's challenge report, re-framed (magic, version, checksum) so a stale or half-written
    /// block never reaches the relay looking valid. The previous one stays in use outside the
    /// field.
    fn read_challenge_out(&mut self, ram: &[u8]) {
        let Some(sync) = find_challenge_sync(ram, &self.local_player) else {
            return;
        };
        let mut up = [0u8; wire::CHALLENGE_UP_SIZE];
        up.copy_from_slice(
            &ram[sync + CHALLENGE_SYNC_OUT..sync + CHALLENGE_SYNC_OUT + wire::CHALLENGE_UP_SIZE],
        );
        up[0..4].copy_from_slice(&wire::CHALLENGE_MAGIC.to_le_bytes());
        up[4] = wire::CHALLENGE_VERSION;
        let sum = sanitize::checksum(&up[..44]);
        up[44..46].copy_from_slice(&sum.to_le_bytes());
        up[46] = 0;
        up[47] = 0;
        self.challenge_up = Some(up);
    }

    /// The client packet, byte for byte the C# bridge's `BuildClientPacket`: the 120-byte
    /// header, then the trade (while the ROM has one open), the link trailer (UC4) and the
    /// challenge trailer, always last (the relay finds it from the end).
    pub fn build_client_packet(&mut self, action: u8) -> Vec<u8> {
        let mut tx = vec![0u8; wire::CLIENT_PACKET_SIZE];
        tx[0..4].copy_from_slice(&wire::COOP_MAGIC.to_le_bytes());
        tx[4..6].copy_from_slice(&wire::PROTOCOL_VERSION.to_le_bytes());
        tx[6..12].copy_from_slice(&self.room_code);
        tx[12] = action;
        tx[13] = self.assigned_slot;
        tx[14] = u8::from(self.allow_ahead);
        tx[15] = self.config.party_size; // the relay honours it only from the host
        tx[16..20].copy_from_slice(&self.host_token.to_le_bytes());
        tx[20..22].copy_from_slice(&self.badge_mask_local.to_le_bytes());
        self.sequence = self.sequence.wrapping_add(1);
        tx[22..26].copy_from_slice(&self.sequence.to_le_bytes());
        tx[26..30].copy_from_slice(&self.heartbeat_rom.to_le_bytes());
        tx[30..32].copy_from_slice(&self.config.challenge_rules.to_le_bytes()); // host only
        tx[32..32 + PLAYER_STATE_SIZE].copy_from_slice(&self.local_player);
        tx[32 + PS_AVATAR_ID] = self.avatar_id; // the ROM leaves it 0
        tx[96..104].copy_from_slice(&self.net.cookie().to_le_bytes());
        tx[104..112].copy_from_slice(&self.config.player_secret.to_le_bytes());
        tx[112..112 + SESSION_OUT_SIZE].copy_from_slice(&self.session_out);
        if let Some(t) = &self.trade_out {
            tx.extend_from_slice(t);
        }
        if let Some(c) = &self.challenge_up {
            tx.extend_from_slice(c);
        }
        tx
    }

    fn process_latest_server_packet(&mut self, ram: &mut [u8], mb: usize) {
        let Some(pkt) = self.net.take_latest() else {
            return;
        };
        let status = pkt[12];
        if status != wire::ROOM_STATUS_OK {
            // A rejected save also tells the ROM why: it shows the reason and returns to the
            // title without saving (written in the same frame as the disconnect, so it wins).
            if matches!(
                status,
                wire::ROOM_STATUS_ERR_MIDGAME_SAVE
                    | wire::ROOM_STATUS_ERR_SAVE_OTHER_ROOM
                    | wire::ROOM_STATUS_ERR_RUN_LOST
            ) {
                ram[mb + ROOM_REJECTION] = status;
            }
            let reason = text::server_status(status, &self.active_room, u16_at(&pkt, 4));
            self.disconnect(&reason, true);
            self.clear_rom_connection_state(ram, mb, true);
            return;
        }
        let assigned = pkt[6];
        if assigned as usize >= MAX_PLAYERS {
            return; // malformed: ignore the packet
        }

        let (pkt, down) = sanitize::take_challenge_down(&pkt);
        if let Some(down) = down {
            self.room_rules = Some((u16_at(&down, 8), down[5]));
            if let Some(sync) = find_challenge_sync(ram, &self.local_player) {
                ram[sync + CHALLENGE_SYNC_IN..sync + CHALLENGE_SYNC_IN + down.len()]
                    .copy_from_slice(&down);
            }
        }

        let payload_at = 16;
        let room_uid = u32_at(pkt, payload_at + PAYLOAD_GATES + GATE_ROOM_UID);
        if room_uid != 0 && self.slot_key.is_none() {
            self.slot_key = Some(slots::room_key(&self.active_room, room_uid));
        }

        let first_ok = self.link != LinkState::Connected || self.assigned_slot != assigned;
        let created = self.active_action == wire::ROOM_ACTION_CREATE;
        self.assigned_slot = assigned;
        self.active_action = wire::ROOM_ACTION_SYNC; // handshake done: steady-state SYNC
        self.last_server_packet = Instant::now();
        if first_ok {
            self.link = LinkState::Connected;
            let (room, server) = (self.active_room.clone(), self.active_server.clone());
            self.worlds.record(&room, &server, created); // Milestone 6 §4.2
            self.set_status(
                format!("In world {room}. Start or continue your game."),
                false,
            );
        }

        let payload = sanitize::server_payload(&pkt[payload_at..], self.limits);
        ram[mb + SERVER_PAYLOAD..mb + SERVER_PAYLOAD + SERVER_PAYLOAD_SIZE]
            .copy_from_slice(&payload);
        ram[mb + ASSIGNED_PLAYER_ID] = self.assigned_slot;
        ram[mb + ACTIVE_PLAYERS_MASK] = pkt[7] & 0x3F;
        put_u32(ram, mb + SERVER_TICK, u32_at(pkt, 8));
        ram[mb + IS_CONNECTED_TO_ROOM] = 1;
        self.write_trade_in(ram, mb, pkt);
        self.clear_payload_pending = true; // clear the remote state once we disconnect
    }

    /// Copies the relay's trade trailer (or "nothing": zeros) into the ROM's open exchange.
    fn write_trade_in(&self, ram: &mut [u8], mb: usize, pkt: &[u8]) {
        let Some(ex) = find_exchange(ram, mb) else {
            return;
        };
        let server = wire::SERVER_PACKET_SIZE;
        let mut trade = [0u8; TRADE_IN_SIZE];
        if pkt.len() >= server + TRADE_IN_SIZE && !has_link_magic(pkt, server) {
            trade = sanitize::trade_in(&pkt[server..]);
        }
        ram[ex + EXCHANGE_IN..ex + EXCHANGE_IN + TRADE_IN_SIZE].copy_from_slice(&trade);
    }

    /// Writes the "not connected" state. Every disconnected frame, so a savestate can never
    /// resurrect a stale connection.
    fn clear_rom_connection_state(&mut self, ram: &mut [u8], mb: usize, clear_payload: bool) {
        ram[mb + IS_CONNECTED_TO_ROOM] = 0;
        ram[mb + ASSIGNED_PLAYER_ID] = PLAYER_ID_NONE;
        ram[mb + ACTIVE_PLAYERS_MASK] = 0;
        if clear_payload {
            ram[mb + SERVER_PAYLOAD..mb + SERVER_PAYLOAD + SERVER_PAYLOAD_SIZE].fill(0);
            self.clear_payload_pending = false;
        }
    }

    /// One save per world (the C# `UpdateSaveSlot`). Copies the cart to the owning world's file
    /// after every completed in-game save (also while disconnected: the disconnect flow saves
    /// after the link drops). Once connected at the title, swaps the world's save (or a blank
    /// one) into the cart and asks the ROM to restart so the game loads it. `save_slot_ready`
    /// lets the startup gate pass only with the right save loaded. True: the cart was replaced.
    fn update_save_slot(&mut self, m: &mut dyn Machine, mb: usize) -> bool {
        let (rom_heartbeat, counter, in_session) = {
            let ram = m.main_ram();
            (
                u32_at(ram, mb + HEARTBEAT_ROM),
                u32_at(ram, mb + SAVE_COUNTER),
                ram[mb + WAS_IN_ACTIVE_SESSION] == 1,
            )
        };
        let rom_restarted = rom_heartbeat < self.last_rom_heartbeat;
        self.last_rom_heartbeat = rom_heartbeat;

        // save_counter restarts at 0 with the game; only a change to non-zero is a new save.
        if self.last_save_counter.is_some_and(|c| c != counter)
            && counter != 0
            && self.slot_state != SlotState::Swapping
        {
            let cart = m.cart().to_vec();
            self.slots.capture(&cart);
        }
        self.last_save_counter = Some(counter);

        let mut replaced = false;
        if self.link == LinkState::Connected
            && let Some(key) = self.slot_key.clone()
        {
            match self.slot_state {
                SlotState::None => {
                    if in_session {
                        // A save is already in play: never swap under it.
                        self.slot_state = SlotState::Ready;
                    } else {
                        let cart = m.cart().to_vec();
                        match self.slots.prepare_join(&key, &cart) {
                            None => self.slot_state = SlotState::Ready,
                            Some(image) => {
                                let c = m.cart();
                                let n = c.len().min(image.len());
                                c[..n].copy_from_slice(&image[..n]);
                                m.main_ram()[mb + RESET_REQUEST] = 1;
                                self.swap_requested = Instant::now();
                                self.slot_state = SlotState::Swapping;
                                replaced = true;
                            }
                        }
                    }
                }
                SlotState::Swapping => {
                    // The restart re-initializes the mailbox (heartbeat_rom back near 0).
                    if rom_restarted {
                        self.slot_state = SlotState::Ready;
                    } else if self.swap_requested.elapsed() > SWAP_RETRY {
                        m.main_ram()[mb + RESET_REQUEST] = 1;
                        self.swap_requested = Instant::now();
                    }
                }
                SlotState::Ready => {}
            }
        }
        let ready = self.link == LinkState::Connected && self.slot_state == SlotState::Ready;
        m.main_ram()[mb + SAVE_SLOT_READY] = u8::from(ready);
        replaced
    }

    /// Every few minutes in a world, only while the player stands in the overworld with nothing
    /// running (the C# `TryTakeBackup`).
    fn try_take_backup(&mut self, m: &mut dyn Machine, mb: usize, actions: &mut Actions) {
        let Some(key) = self.slot_key.clone() else {
            return;
        };
        if self.slot_state != SlotState::Ready {
            return;
        }
        let now = Instant::now();
        let due = *self.next_backup.get_or_insert(now + backups::INTERVAL);
        if now < due {
            return;
        }
        let safe = {
            let ram = m.main_ram();
            backups::safe_moment(&ram[mb..mb + MAILBOX_SIZE])
        };
        if !safe {
            return; // try again next frame
        }
        self.next_backup = Some(now + backups::INTERVAL);
        match m.savestate() {
            Some(state) => match self.backups.write(&key, &state) {
                Ok(p) => actions.backup_taken = Some(p),
                Err(e) => actions.backup_failed = Some(e),
            },
            None => actions.backup_failed = Some("the emulator couldn't make a savestate".into()),
        }
    }

    /// What the window shows.
    pub fn view(&self) -> View {
        let save_line = if !self.slots.last_error.is_empty() {
            format!("Error: {}", self.slots.last_error)
        } else if self.link != LinkState::Connected || self.slot_key.is_none() {
            match self.slots.owner().split('-').next() {
                Some(w) if !w.is_empty() => format!("Last world: {w}"),
                _ => "—".into(),
            }
        } else {
            match self.slot_state {
                SlotState::Swapping => "Loading this world's save (the game restarts)…".into(),
                SlotState::Ready => {
                    if self
                        .slots
                        .has_room_save(self.slot_key.as_deref().unwrap_or(""))
                    {
                        "This world's save is loaded".into()
                    } else {
                        "New world: start a New Game".into()
                    }
                }
                SlotState::None => "Preparing…".into(),
            }
        };
        View {
            link: self.link,
            slot: (self.link == LinkState::Connected
                && (self.assigned_slot as usize) < MAX_PLAYERS)
                .then_some(self.assigned_slot),
            message: self.message.clone(),
            message_is_error: self.message_is_error,
            mailbox: self.mailbox_copy.clone(),
            local_player: self.local_player,
            room: self.active_room.clone(),
            room_rules: self.room_rules,
            save_line,
            has_backups: !self.backups().is_empty(),
        }
    }
}

/// Main-RAM offset of the ROM's open trade exchange, or None. The pointer comes from the game, so
/// it's range-checked and the block's header verified before anything is read or written.
fn find_exchange(ram: &[u8], mb: usize) -> Option<usize> {
    let ptr = u32_at(ram, mb + EXCHANGE_PTR);
    if ptr < MAIN_RAM_BASE
        || ptr >= MAIN_RAM_BASE + MAIN_RAM_SIZE - EXCHANGE_SIZE as u32
        || ptr & 3 != 0
    {
        return None;
    }
    let at = (ptr - MAIN_RAM_BASE) as usize;
    (u32_at(ram, at) == EXCHANGE_MAGIC
        && u16_at(ram, at + 4) == EXCHANGE_VERSION
        && u16_at(ram, at + 6) as usize == EXCHANGE_SIZE)
        .then_some(at)
}

/// Main-RAM offset of the ROM's challenge sync block, or None outside the field. Overlay 131's
/// pinned header holds the pointer; header magic, range and the block's own header are all
/// checked, since that memory belongs to other overlays in battles and menus.
fn find_challenge_sync(ram: &[u8], local_player: &[u8]) -> Option<usize> {
    if local_player[PS_IS_ACTIVE] != 1 || u32_at(ram, FIELD_ENTRY_HEADER) != FIELD_ENTRY_MAGIC {
        return None;
    }
    let ptr = u32_at(ram, FIELD_ENTRY_HEADER + FIELD_ENTRY_CHALLENGE);
    if ptr < OVERLAY131_START || ptr > OVERLAY131_END - CHALLENGE_SYNC_SIZE as u32 || ptr & 3 != 0 {
        return None;
    }
    let at = (ptr - MAIN_RAM_BASE) as usize;
    (u32_at(ram, at) == CHALLENGE_SYNC_MAGIC
        && u16_at(ram, at + 4) == CHALLENGE_SYNC_VERSION
        && u16_at(ram, at + 6) as usize == CHALLENGE_SYNC_SIZE)
        .then_some(at)
}

/// A link trailer ("LNK1") at `at` (CoopLink.HasMagic): then there's no trade trailer.
fn has_link_magic(pkt: &[u8], at: usize) -> bool {
    pkt.len() >= at + 4 && u32_at(pkt, at) == wire::LINK_MAGIC
}
