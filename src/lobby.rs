use std::collections::HashMap;
use std::fs;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use tracing::{debug, info, warn};

use crate::protocol::{
    room_code_to_string, sanitize_room_code, ClientToServerPacket, FollowerState, PlayerState,
    ServerToClientPacket, AUTO_PLAYER_ID, COOP_MAGIC, DEFAULT_ROOM_CODE, MAX_PLAYERS,
    PROTOCOL_VERSION, REMOTE_SLOTS, ROOM_ACTION_CREATE, ROOM_ACTION_JOIN,
    ROOM_STATUS_ERR_ALREADY_EXISTS, ROOM_STATUS_ERR_FULL, ROOM_STATUS_ERR_LOCKED,
    ROOM_STATUS_ERR_NOT_FOUND, ROOM_STATUS_OK, SERVER_PACKET_SIZE,
};

const MAX_NPCS_BEFORE_FOLLOWER_CULL: u8 = 26;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegisteredTrainer {
    pub slot_id: u8,
    pub trainer_id: u32,
    pub last_badge_mask: u16,
    pub assigned_monolocke: u8,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CampaignSaveFile {
    pub room_code: String,
    pub host_token: u32,
    pub campaign_started: bool,
    pub expected_party_size: u8,
    pub roster: Vec<RegisteredTrainer>,
}

#[derive(Debug, Clone)]
pub struct ConnectedPlayer {
    pub slot_id: u8,
    pub addr: SocketAddr,
    pub last_seen: Instant,
    pub last_sequence: u32,
    pub heartbeat_rom: u32,
    pub client_ready: bool,
    pub state: PlayerState,
    pub badge_mask: u16,
    pub assigned_monolocke: u8,
}

pub struct LobbyState {
    room_code: [u8; 6],
    host_token: u32,
    slots: [Option<ConnectedPlayer>; MAX_PLAYERS],
    campaign_started: bool,
    roster: Vec<RegisteredTrainer>,
    target_party_size: usize,
    save_dir: Option<PathBuf>,
    client_timeout: Duration,
    server_tick: u32,
    inspect_all_packets: bool,
}

impl LobbyState {
    pub fn new(
        client_timeout: Duration,
        default_party_size: usize,
        inspect_all_packets: bool,
    ) -> Self {
        Self::with_room(
            DEFAULT_ROOM_CODE,
            0,
            client_timeout,
            default_party_size,
            inspect_all_packets,
            None,
        )
    }

    pub fn with_room(
        room_code: [u8; 6],
        host_token: u32,
        client_timeout: Duration,
        default_party_size: usize,
        inspect_all_packets: bool,
        save_dir: Option<PathBuf>,
    ) -> Self {
        let clean_code = sanitize_room_code(room_code);
        let mut lobby = Self {
            room_code: clean_code,
            host_token,
            slots: Default::default(),
            campaign_started: false,
            roster: Vec::new(),
            target_party_size: default_party_size.clamp(2, MAX_PLAYERS),
            save_dir,
            client_timeout,
            server_tick: 0,
            inspect_all_packets,
        };
        lobby.try_load_campaign_from_disk();
        lobby
    }

    pub fn host_token(&self) -> u32 {
        self.host_token
    }

    pub fn is_campaign_started(&self) -> bool {
        self.campaign_started
    }

    pub fn expected_party_size(&self) -> usize {
        if self.campaign_started {
            self.roster.len()
        } else {
            self.target_party_size
        }
    }

    pub fn handle_client_packet(&mut self, addr: SocketAddr, pkt: ClientToServerPacket) -> Result<u8, u8> {
        let now = Instant::now();
        let trainer_id = pkt.local_player.trainer_id;

        if !self.campaign_started
            && pkt.local_player.target_party_size >= 2
            && pkt.host_token == self.host_token
        {
            self.target_party_size =
                (pkt.local_player.target_party_size as usize).clamp(2, MAX_PLAYERS);
        }

        let slot_idx = self.resolve_or_assign_slot(addr, pkt.requested_player_id, trainer_id)?;
        let room_str = room_code_to_string(&self.room_code);

        let assigned_monolocke = self
            .roster
            .iter()
            .find(|r| r.slot_id as usize == slot_idx)
            .map(|r| r.assigned_monolocke)
            .unwrap_or(18);

        if let Some(existing) = &mut self.slots[slot_idx] {
            if pkt.sequence < existing.last_sequence
                && existing.last_sequence.wrapping_sub(pkt.sequence) < 10_000
            {
                return Ok(slot_idx as u8);
            }

            let old = &existing.state;
            let new = &pkt.local_player;
            let ready_bool = pkt.client_ready != 0;
            let state_changed = old.map_id != new.map_id
                || old.x != new.x
                || old.y != new.y
                || old.direction != new.direction
                || old.lead_species != new.lead_species
                || old.trainer_id != new.trainer_id
                || existing.client_ready != ready_bool
                || existing.badge_mask != pkt.badge_mask_local;

            if state_changed || self.inspect_all_packets {
                info!(
                    "[Room {room_str} | P{slot_idx} @ {addr}] OTID: 0x{:08X} | Ready: {} | Map: {} | Pos: ({}, {}) | Lead: #{} | Badges: 0b{:08b}",
                    new.trainer_id,
                    ready_bool,
                    new.map_id,
                    new.x,
                    new.y,
                    new.lead_species,
                    pkt.badge_mask_local
                );
            }

            let mut updated_state = pkt.local_player;
            updated_state.is_active = 1;
            if (updated_state.x != old.x || updated_state.y != old.y)
                && updated_state.prev_x == updated_state.x
                && updated_state.prev_y == updated_state.y
            {
                updated_state.prev_x = old.x;
                updated_state.prev_y = old.y;
            }

            let badge_changed = existing.badge_mask != pkt.badge_mask_local;
            existing.addr = addr;
            existing.last_seen = now;
            existing.last_sequence = pkt.sequence;
            existing.heartbeat_rom = pkt.heartbeat_rom;
            existing.client_ready = ready_bool;
            existing.state = updated_state;
            existing.badge_mask = pkt.badge_mask_local;

            if badge_changed && self.campaign_started {
                if let Some(r) = self.roster.iter_mut().find(|r| r.slot_id as usize == slot_idx) {
                    r.last_badge_mask = pkt.badge_mask_local;
                    self.save_campaign_to_disk();
                }
            }
        } else {
            let mut initial_state = pkt.local_player;
            initial_state.is_active = 1;
            if initial_state.prev_x == 0 && initial_state.prev_y == 0 {
                initial_state.prev_x = initial_state.x;
                initial_state.prev_y = initial_state.y.saturating_sub(1);
            }

            info!(
                "[Room {room_str} | JOIN] Slot P{slot_idx} assigned to {addr} (OTID: 0x{:08X} | Map: {} | Ready: {})",
                initial_state.trainer_id,
                initial_state.map_id,
                pkt.client_ready != 0
            );

            self.slots[slot_idx] = Some(ConnectedPlayer {
                slot_id: slot_idx as u8,
                addr,
                last_seen: now,
                last_sequence: pkt.sequence,
                heartbeat_rom: pkt.heartbeat_rom,
                client_ready: pkt.client_ready != 0,
                state: initial_state,
                badge_mask: pkt.badge_mask_local,
                assigned_monolocke,
            });
        }

        self.check_and_lock_bedroom_gate();
        Ok(slot_idx as u8)
    }

    fn resolve_or_assign_slot(
        &self,
        addr: SocketAddr,
        requested_id: u8,
        trainer_id: u32,
    ) -> Result<usize, u8> {
        // 1. Existing socket match
        for (idx, slot) in self.slots.iter().enumerate() {
            if let Some(player) = slot {
                if player.addr == addr {
                    return Ok(idx);
                }
            }
        }

        // 2. If campaign is locked, authenticate strictly against registered roster
        if self.campaign_started {
            if trainer_id != 0 {
                for reg in &self.roster {
                    let idx = reg.slot_id as usize;
                    if reg.trainer_id == trainer_id && self.slots[idx].is_none() {
                        return Ok(idx);
                    }
                }
            }
            if (requested_id as usize) < self.roster.len()
                && self.slots[requested_id as usize].is_none()
            {
                return Ok(requested_id as usize);
            }

            warn!(
                "[Room {}] Rejected connection from {addr} (OTID: 0x{trainer_id:08X}) — Campaign roster is locked ({} players)",
                room_code_to_string(&self.room_code),
                self.roster.len()
            );
            return Err(ROOM_STATUS_ERR_LOCKED);
        }

        // 3. Campaign is Forming: honor requested slot if within target_party_size and free
        if (requested_id as usize) < self.target_party_size && requested_id != AUTO_PLAYER_ID {
            let idx = requested_id as usize;
            if self.slots[idx].is_none() {
                return Ok(idx);
            }
        }

        // 4. Auto-assign lowest free slot up to target_party_size
        for idx in 0..self.target_party_size {
            if self.slots[idx].is_none() {
                return Ok(idx);
            }
        }

        warn!(
            "[Room {}] Room full ({}/{}) — rejected {addr}",
            room_code_to_string(&self.room_code),
            self.target_party_size,
            self.target_party_size
        );
        Err(ROOM_STATUS_ERR_FULL)
    }

    fn check_and_lock_bedroom_gate(&mut self) {
        if self.campaign_started {
            return;
        }

        let active: Vec<&ConnectedPlayer> = self.slots.iter().filter_map(|s| s.as_ref()).collect();
        if active.len() < self.target_party_size {
            return;
        }

        let all_ready = active.iter().all(|p| p.client_ready && p.state.map_id != 0);
        if !all_ready {
            return;
        }

        self.campaign_started = true;
        self.roster.clear();

        for p in &active {
            let effective_otid = if p.state.trainer_id != 0 {
                p.state.trainer_id
            } else {
                0xCAFE_0000 | u32::from(p.slot_id)
            };

            self.roster.push(RegisteredTrainer {
                slot_id: p.slot_id,
                trainer_id: effective_otid,
                last_badge_mask: p.badge_mask,
                assigned_monolocke: p.assigned_monolocke,
            });
        }

        info!(
            "=== [Room {} | BEDROOM GATE UNLOCKED] Roster Locked with {}/{} Players! Adventure Started! ===",
            room_code_to_string(&self.room_code),
            self.roster.len(),
            self.target_party_size
        );
        self.save_campaign_to_disk();
    }

    pub fn prune_stale_clients(&mut self) {
        let now = Instant::now();
        let room_str = room_code_to_string(&self.room_code);
        for (idx, slot) in self.slots.iter_mut().enumerate() {
            if let Some(player) = slot {
                if now.duration_since(player.last_seen) > self.client_timeout {
                    info!(
                        "[Room {room_str} | TIMEOUT] Slot P{idx} ({}) went offline",
                        player.addr
                    );
                    *slot = None;
                }
            }
        }
    }

    pub fn build_broadcast_batch(&mut self) -> Vec<(SocketAddr, [u8; SERVER_PACKET_SIZE])> {
        self.server_tick = self.server_tick.wrapping_add(1);

        let active_mask = self.compute_active_bitmask();
        let online_count = active_mask.count_ones() as u8;
        let ready_count = self
            .slots
            .iter()
            .filter_map(|s| s.as_ref())
            .filter(|p| p.client_ready)
            .count() as u8;
        let expected_size = self.expected_party_size() as u8;

        let (synced_badges, sync_ready) = self.compute_progression_gates();

        let mut batch = Vec::with_capacity(MAX_PLAYERS);

        for recipient_idx in 0..MAX_PLAYERS {
            let Some(recipient) = &self.slots[recipient_idx] else {
                continue;
            };

            let mut remote_players = [PlayerState::default(); REMOTE_SLOTS];
            let mut remote_followers = [FollowerState::default(); REMOTE_SLOTS];

            let mut remote_cursor = 0;
            for other_idx in 0..MAX_PLAYERS {
                if other_idx == recipient_idx {
                    continue;
                }

                if let Some(other) = &self.slots[other_idx] {
                    let same_map =
                        other.state.map_id == recipient.state.map_id && other.state.map_id != 0;

                    let mut r_player = other.state;
                    r_player.is_active = u8::from(same_map);
                    remote_players[remote_cursor] = r_player;

                    let show_follower = same_map
                        && other.state.lead_species > 0
                        && recipient.state.map_npc_count <= MAX_NPCS_BEFORE_FOLLOWER_CULL;

                    remote_followers[remote_cursor] = FollowerState {
                        species: other.state.lead_species,
                        form: other.state.lead_form,
                        is_shiny: other.state.lead_is_shiny,
                        gender: other.state.lead_gender,
                        x: other.state.prev_x,
                        y: other.state.prev_y,
                        direction: other.state.direction,
                        is_visible: u8::from(show_follower),
                        pad: [0u8; 12],
                    };
                }

                remote_cursor += 1;
            }

            let pkt = ServerToClientPacket {
                magic: COOP_MAGIC,
                version: PROTOCOL_VERSION,
                assigned_player_id: recipient.slot_id,
                active_players_mask: active_mask,
                server_tick: self.server_tick,
                status_code: ROOM_STATUS_OK,
                reserved_header: [0u8; 3],
                remote_players,
                remote_followers,
                badge_mask_local_echo: recipient.badge_mask,
                badge_mask_synced: synced_badges,
                flag_coop_sync_ready: u8::from(sync_ready),
                current_checkpoint: synced_badges.count_ones() as u8,
                assigned_monolocke: recipient.assigned_monolocke,
                nuzlocke_failed: 0,
                flag_campaign_started: u8::from(self.campaign_started),
                expected_party_size: expected_size,
                online_party_count: online_count,
                ready_party_count: ready_count,
                gates_pad: [0u8; 12],
            };

            batch.push((recipient.addr, pkt.encode_le()));
        }

        batch
    }

    fn compute_progression_gates(&self) -> (u16, bool) {
        let active: Vec<&ConnectedPlayer> = self.slots.iter().filter_map(|s| s.as_ref()).collect();
        if active.is_empty() {
            return (0, false);
        }

        let required_party_size = self.expected_party_size();
        let all_registered_online = active.len() >= required_party_size;

        let mut common_badges = 0xFFFFu16;
        let first_map = active[0].state.map_id;
        let first_badges = active[0].badge_mask;
        let mut all_same_map = true;
        let mut all_equal_badges = true;

        for p in &active {
            common_badges &= p.badge_mask;
            if p.state.map_id != first_map {
                all_same_map = false;
            }
            if p.badge_mask != first_badges {
                all_equal_badges = false;
            }
        }

        if self.campaign_started {
            for reg in &self.roster {
                if self.slots[reg.slot_id as usize].is_none() {
                    common_badges &= reg.last_badge_mask;
                }
            }
        }

        let sync_ready =
            all_registered_online && all_equal_badges && all_same_map && first_map != 0;

        (common_badges, sync_ready)
    }

    fn compute_active_bitmask(&self) -> u8 {
        let mut mask = 0u8;
        for (idx, slot) in self.slots.iter().enumerate() {
            if slot.is_some() {
                mask |= 1 << idx;
            }
        }
        mask
    }

    pub fn print_periodic_summary(&self) {
        let connected: Vec<&ConnectedPlayer> = self.slots.iter().filter_map(|s| s.as_ref()).collect();
        if connected.is_empty() {
            return;
        }
        let (synced_badges, ready) = self.compute_progression_gates();
        let room_str = room_code_to_string(&self.room_code);
        info!(
            "--- [Room {room_str} | Online: {}/{} | Started: {} | Synced Badges: 0b{:08b} | Gym Gate: {}] ---",
            connected.len(),
            self.expected_party_size(),
            self.campaign_started,
            synced_badges,
            if ready { "UNLOCKED" } else { "LOCKED" }
        );
        for p in connected {
            info!(
                "    Slot P{} ({}) -> OTID: 0x{:08X} | Ready: {} | Map {:3} | XY: ({:3}, {:3}) | Lead: #{:4}",
                p.slot_id,
                p.addr,
                p.state.trainer_id,
                p.client_ready,
                p.state.map_id,
                p.state.x,
                p.state.y,
                p.state.lead_species
            );
        }
    }

    fn save_file_path(&self) -> Option<PathBuf> {
        let dir = self.save_dir.as_ref()?;
        let room_str = room_code_to_string(&self.room_code);
        Some(dir.join(format!("{room_str}.json")))
    }

    fn save_campaign_to_disk(&self) {
        let Some(path) = self.save_file_path() else {
            return;
        };
        if let Some(parent) = path.parent() {
            let _ = fs::create_dir_all(parent);
        }
        let payload = CampaignSaveFile {
            room_code: room_code_to_string(&self.room_code),
            host_token: self.host_token,
            campaign_started: self.campaign_started,
            expected_party_size: self.roster.len() as u8,
            roster: self.roster.clone(),
        };
        if let Ok(json) = serde_json::to_string_pretty(&payload) {
            let _ = fs::write(&path, json);
        }
    }

    fn try_load_campaign_from_disk(&mut self) {
        let Some(path) = self.save_file_path() else {
            return;
        };
        if !path.exists() {
            return;
        }
        if let Ok(raw) = fs::read_to_string(&path) {
            if let Ok(saved) = serde_json::from_str::<CampaignSaveFile>(&raw) {
                self.host_token = saved.host_token;
                self.campaign_started = saved.campaign_started;
                self.roster = saved.roster;
                self.target_party_size = (saved.expected_party_size as usize).clamp(2, MAX_PLAYERS);
                info!(
                    "[Room {}] Restored locked campaign from disk (Host Token: 0x{:08X}, {} trainers)",
                    saved.room_code,
                    saved.host_token,
                    self.roster.len()
                );
            }
        }
    }
}

pub struct RoomManager {
    rooms: HashMap<[u8; 6], LobbyState>,
    client_timeout: Duration,
    default_party_size: usize,
    inspect_all_packets: bool,
    save_dir: PathBuf,
}

impl RoomManager {
    pub fn new(
        client_timeout: Duration,
        default_party_size: usize,
        inspect_all_packets: bool,
        save_dir: impl AsRef<Path>,
    ) -> Self {
        Self {
            rooms: HashMap::new(),
            client_timeout,
            default_party_size: default_party_size.clamp(2, MAX_PLAYERS),
            inspect_all_packets,
            save_dir: save_dir.as_ref().to_path_buf(),
        }
    }

    pub fn handle_client_packet(
        &mut self,
        addr: SocketAddr,
        pkt: ClientToServerPacket,
    ) -> Result<u8, [u8; SERVER_PACKET_SIZE]> {
        let code = sanitize_room_code(pkt.room_code);
        let room_str = room_code_to_string(&code);
        let save_path = self.save_dir.join(format!("{room_str}.json"));
        let room_in_memory = self.rooms.contains_key(&code);
        let room_on_disk = save_path.exists();

        match pkt.room_action {
            ROOM_ACTION_CREATE => {
                if let Some(existing_room) = self.rooms.get(&code) {
                    if existing_room.host_token() != pkt.host_token {
                        warn!(
                            "[RoomManager] Rejected CREATE for room {room_str} from {addr}: room already exists with different host_token"
                        );
                        return Err(ServerToClientPacket::create_error_response(
                            ROOM_STATUS_ERR_ALREADY_EXISTS,
                        ));
                    }
                } else if room_on_disk {
                    if let Ok(raw) = fs::read_to_string(&save_path) {
                        if let Ok(saved) = serde_json::from_str::<CampaignSaveFile>(&raw) {
                            if saved.host_token != pkt.host_token {
                                warn!(
                                    "[RoomManager] Rejected CREATE for room {room_str} from {addr}: saved campaign exists on disk"
                                );
                                return Err(ServerToClientPacket::create_error_response(
                                    ROOM_STATUS_ERR_ALREADY_EXISTS,
                                ));
                            }
                        }
                    }
                }

                let timeout = self.client_timeout;
                let def_party = self.default_party_size;
                let inspect = self.inspect_all_packets;
                let save_dir = self.save_dir.clone();

                let room = self.rooms.entry(code).or_insert_with(|| {
                    info!(
                        "[RoomManager] Created room {room_str} (Host Token: 0x{:08X})",
                        pkt.host_token
                    );
                    LobbyState::with_room(
                        code,
                        pkt.host_token,
                        timeout,
                        def_party,
                        inspect,
                        Some(save_dir),
                    )
                });

                room.handle_client_packet(addr, pkt)
                    .map_err(ServerToClientPacket::create_error_response)
            }

            ROOM_ACTION_JOIN => {
                if !room_in_memory && !room_on_disk {
                    debug!(
                        "[RoomManager] Rejected JOIN from {addr}: room {room_str} does not exist"
                    );
                    return Err(ServerToClientPacket::create_error_response(
                        ROOM_STATUS_ERR_NOT_FOUND,
                    ));
                }

                let timeout = self.client_timeout;
                let def_party = self.default_party_size;
                let inspect = self.inspect_all_packets;
                let save_dir = self.save_dir.clone();

                let room = self.rooms.entry(code).or_insert_with(|| {
                    LobbyState::with_room(code, 0, timeout, def_party, inspect, Some(save_dir))
                });

                room.handle_client_packet(addr, pkt)
                    .map_err(ServerToClientPacket::create_error_response)
            }

            _ => Err(ServerToClientPacket::create_error_response(
                ROOM_STATUS_ERR_NOT_FOUND,
            )),
        }
    }

    pub fn tick_all_rooms(&mut self) -> Vec<(SocketAddr, [u8; SERVER_PACKET_SIZE])> {
        let mut all_packets = Vec::new();
        for room in self.rooms.values_mut() {
            room.prune_stale_clients();
            all_packets.extend(room.build_broadcast_batch());
        }
        all_packets
    }

    pub fn print_all_summaries(&self) {
        for room in self.rooms.values() {
            room.print_periodic_summary();
        }
    }
}
