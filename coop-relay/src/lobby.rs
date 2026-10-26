use std::collections::HashMap;
use std::collections::hash_map::RandomState;
use std::fs;
use std::hash::BuildHasher;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use tracing::{error, info, warn};

use crate::admission::{CookieJar, FailureLimiter};
use crate::protocol::{
    AUTO_PLAYER_ID, COOP_MAGIC, ClientToServerPacket, MAX_PLAYERS, NO_SLOT, PROTOCOL_VERSION,
    PlayerState, REMOTE_SLOTS, ROOM_ACTION_CREATE, ROOM_ACTION_JOIN, ROOM_ACTION_LEAVE,
    ROOM_ACTION_SYNC, ROOM_STATUS_ERR_ALREADY_EXISTS, ROOM_STATUS_ERR_FULL, ROOM_STATUS_ERR_LOCKED,
    ROOM_STATUS_ERR_MIDGAME_SAVE, ROOM_STATUS_ERR_NOT_FOUND, ROOM_STATUS_ERR_RATE_LIMITED,
    ROOM_STATUS_ERR_SAVE_OTHER_ROOM, ROOM_STATUS_OK, SAVE_STATUS_IN_PROGRESS, SERVER_PACKET_SIZE,
    SESSION_KIND_BATTLE, SESSION_KIND_TEAM_UP, SESSION_KIND_TRADE, SESSION_OP_ACCEPT,
    SESSION_OP_CANCEL, SESSION_OP_DECLINE, SESSION_OP_REQUEST, SESSION_REASON_ALREADY_TEAMED,
    SESSION_REASON_BUSY, SESSION_REASON_INVALID, SESSION_REASON_NONE, SESSION_REASON_NOT_HERE,
    SESSION_REASON_TEAM_RULE, SESSION_STATE_ACCEPTED, SESSION_STATE_CANCELLED,
    SESSION_STATE_DECLINED, SESSION_STATE_INCOMING, SESSION_STATE_OUTGOING, SESSION_STATE_REFUSED,
    SESSION_STATE_TIMEOUT, ServerToClientPacket, SessionOut, TEAMED_SELF_BIT, room_code_to_string,
    validate_room_code,
};

/// A request nobody answers expires after this long.
pub const SESSION_TIMEOUT: Duration = Duration::from_secs(20);

/// A campaign file exists on disk but could not be read or parsed. The room
/// is refused rather than silently reset, so a corrupt file can never unlock
/// a roster.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CampaignLoadError(pub String);

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegisteredTrainer {
    pub slot_id: u8,
    pub trainer_id: u32,
    pub last_badge_mask: u16,
    pub assigned_monolocke: u8,
    /// Per-install secret of the bridge that registered this trainer. OTIDs
    /// are broadcast to every room member, so the OTID alone must never be
    /// enough to take over an offline teammate's slot. 0 = legacy campaign
    /// file: the first claimant binds it. Reset to 0 by hand to let a player
    /// rejoin from a new install.
    #[serde(default)]
    pub player_secret: u64,
    /// "Let others go ahead" (bridge toggle), remembered while offline so an
    /// absent teammate can still let the others through the gym gate.
    #[serde(default)]
    pub allow_ahead: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CampaignSaveFile {
    pub room_code: String,
    /// Random per room, never reused; keys the bridge's per-room saves.
    /// 0 in files written before it existed: a new one is assigned on load.
    #[serde(default)]
    pub room_uid: u32,
    pub host_token: u32,
    pub campaign_started: bool,
    pub expected_party_size: u8,
    pub roster: Vec<RegisteredTrainer>,
    /// Permanent pairs of trainer IDs (Milestone 3 "team up"); never undone.
    #[serde(default)]
    pub teams: Vec<[u32; 2]>,
}

/// What one player currently sees of their session, latched until the next
/// change. `seq` moves on every change so the ROM reacts once.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SessionView {
    pub state: u8,
    pub kind: u8,
    pub peer: Option<usize>, // absolute slot of the other side
    pub reason: u8,
    pub seq: u8,
}

/// A pending request: `from` asked `to` (absolute slots).
#[derive(Debug, Clone)]
struct Session {
    kind: u8,
    from: usize,
    to: usize,
    created: Instant,
}

#[derive(Debug, Clone)]
pub struct ConnectedPlayer {
    pub slot_id: u8,
    pub addr: SocketAddr,
    pub last_seen: Instant,
    pub last_sequence: u32,
    pub heartbeat_rom: u32,
    pub in_bedroom_ready: bool,
    pub state: PlayerState,
    pub badge_mask: u16,
    pub assigned_monolocke: u8,
    pub player_secret: u64,
    pub allow_ahead: bool,
    /// Last session operation sequence applied (the client repeats it).
    pub last_session_seq: u8,
    pub session_view: SessionView,
}

pub struct LobbyState {
    room_code: [u8; 6],
    room_uid: u32,
    host_token: u32,
    host_addr: Option<SocketAddr>,
    slots: [Option<ConnectedPlayer>; MAX_PLAYERS],
    campaign_started: bool,
    roster: Vec<RegisteredTrainer>,
    target_party_size: usize,
    save_dir: Option<PathBuf>,
    client_timeout: Duration,
    server_tick: u32,
    inspect_all_packets: bool,
    last_activity: Instant,
    teams: Vec<[u32; 2]>,
    sessions: Vec<Session>,
}

impl LobbyState {
    pub fn new(
        client_timeout: Duration,
        default_party_size: usize,
        inspect_all_packets: bool,
    ) -> Self {
        Self::with_room(
            *b"TEST01",
            0,
            None,
            client_timeout,
            default_party_size,
            inspect_all_packets,
            None,
        )
        .expect("a room without a save directory never touches disk")
    }

    pub fn with_room(
        room_code: [u8; 6],
        host_token: u32,
        host_addr: Option<SocketAddr>,
        client_timeout: Duration,
        default_party_size: usize,
        inspect_all_packets: bool,
        save_dir: Option<PathBuf>,
    ) -> Result<Self, CampaignLoadError> {
        let mut lobby = Self {
            room_code,
            room_uid: new_room_uid(room_code),
            host_token,
            host_addr,
            slots: Default::default(),
            campaign_started: false,
            roster: Vec::new(),
            target_party_size: default_party_size.clamp(2, MAX_PLAYERS),
            save_dir,
            client_timeout,
            server_tick: 0,
            inspect_all_packets,
            last_activity: Instant::now(),
            teams: Vec::new(),
            sessions: Vec::new(),
        };
        lobby.load_campaign_from_disk()?;
        Ok(lobby)
    }

    pub fn host_token(&self) -> u32 {
        self.host_token
    }

    pub fn room_uid(&self) -> u32 {
        self.room_uid
    }

    /// Trainer IDs of the locked roster (empty while the room is forming).
    pub fn roster_trainer_ids(&self) -> impl Iterator<Item = u32> + '_ {
        self.roster.iter().map(|r| r.trainer_id)
    }

    pub fn host_addr(&self) -> Option<SocketAddr> {
        self.host_addr
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

    /// True when nobody is connected and no packet arrived for `idle_timeout`.
    pub fn is_idle(&self, now: Instant, idle_timeout: Duration) -> bool {
        self.slots.iter().all(Option::is_none)
            && now.duration_since(self.last_activity) > idle_timeout
    }

    pub fn remove_client_by_addr(&mut self, addr: SocketAddr) {
        let room_str = room_code_to_string(&self.room_code);
        for (idx, slot) in self.slots.iter_mut().enumerate() {
            if let Some(p) = slot
                && p.addr == addr
            {
                info!("[Room {room_str} | LEAVE] Slot P{idx} ({addr}) disconnected");
                *slot = None;
            }
        }
        self.drop_sessions_of_missing_players();
    }

    pub fn handle_client_packet(
        &mut self,
        addr: SocketAddr,
        pkt: ClientToServerPacket,
    ) -> Result<u8, u8> {
        let now = Instant::now();
        self.last_activity = now;
        let trainer_id = pkt.local_player.trainer_id;
        let room_str = room_code_to_string(&self.room_code);

        // Game-agnostic admission: the relay never inspects map numbers or badge
        // masks during lobby formation. Readiness is computed by the ROM and
        // carried in local_player.client_ready.
        // Only the room creator can configure target_party_size while forming
        if !self.campaign_started
            && pkt.target_party_size >= 2
            && pkt.host_token != 0
            && pkt.host_token == self.host_token
        {
            self.target_party_size = (pkt.target_party_size as usize).clamp(2, MAX_PLAYERS);
        }

        // Save admission (docs/MILESTONE2_PLAN.md 7.1): a room that has not
        // started only takes fresh saves. The ROM decides what "fresh" means
        // (bedroom, empty party); the relay compares a number.
        if !self.campaign_started
            && pkt.local_player.is_active == 1
            && pkt.local_player.save_status == SAVE_STATUS_IN_PROGRESS
        {
            warn!(
                "[Room {room_str}] Rejected {addr} (OTID: 0x{trainer_id:08X}): mid-game save in a room that has not started"
            );
            self.remove_client_by_addr(addr);
            return Err(ROOM_STATUS_ERR_MIDGAME_SAVE);
        }

        let slot_idx = match self.resolve_or_assign_slot(
            addr,
            pkt.requested_player_id,
            trainer_id,
            pkt.player_secret,
        ) {
            Ok(idx) => idx,
            Err(err_code) => {
                self.remove_client_by_addr(addr);
                return Err(err_code);
            }
        };

        let assigned_monolocke = self
            .roster
            .iter()
            .find(|r| r.slot_id as usize == slot_idx)
            .map(|r| r.assigned_monolocke)
            .unwrap_or(18);

        // Trust the ROM-computed flag directly: the client decides readiness.
        let in_bedroom_ready = pkt.local_player.client_ready == 1;

        if let Some(existing) = &mut self.slots[slot_idx] {
            // Drop reordered packets from the same socket. A new socket (the
            // owner reconnecting from a restarted bridge) restarts its sequence.
            if existing.addr == addr
                && pkt.sequence < existing.last_sequence
                && existing.last_sequence.wrapping_sub(pkt.sequence) < 10_000
            {
                return Ok(slot_idx as u8);
            }

            let old = &existing.state;
            let new = &pkt.local_player;
            let state_changed = old.map_id != new.map_id
                || old.x != new.x
                || old.z != new.z
                || old.direction != new.direction
                || old.avatar_state != new.avatar_state
                || old.activity != new.activity
                || old.follower_species != new.follower_species
                || old.trainer_id != new.trainer_id
                || existing.in_bedroom_ready != in_bedroom_ready
                || existing.badge_mask != pkt.badge_mask_local;

            if state_changed || self.inspect_all_packets {
                info!(
                    "[Room {room_str} | P{slot_idx} @ {addr}] OTID: 0x{:08X} | Ready: {} | Map: {} (matrix {}) | Pos: ({}, {}) | State: {} | Activity: {} | Follower: #{} | Badges: 0b{:08b}",
                    new.trainer_id,
                    in_bedroom_ready,
                    new.map_id,
                    new.matrix_id,
                    new.x,
                    new.z,
                    new.avatar_state,
                    new.activity,
                    new.follower_species,
                    pkt.badge_mask_local
                );
            }

            let badge_changed = existing.badge_mask != pkt.badge_mask_local;
            let allow_ahead = pkt.allow_ahead == 1;
            let allow_changed = existing.allow_ahead != allow_ahead;
            existing.allow_ahead = allow_ahead;
            existing.addr = addr;
            existing.player_secret = pkt.player_secret;
            existing.last_seen = now;
            existing.last_sequence = pkt.sequence;
            existing.heartbeat_rom = pkt.heartbeat_rom;
            existing.in_bedroom_ready = in_bedroom_ready;
            existing.state = pkt.local_player;
            existing.badge_mask = pkt.badge_mask_local;
            let session_out = pkt.session;
            let fresh_op = session_out.seq != existing.last_session_seq;
            existing.last_session_seq = session_out.seq;
            if fresh_op {
                self.apply_session_op(slot_idx, session_out, now);
            }

            if (badge_changed || allow_changed)
                && self.campaign_started
                && let Some(r) = self
                    .roster
                    .iter_mut()
                    .find(|r| r.slot_id as usize == slot_idx)
            {
                r.last_badge_mask = pkt.badge_mask_local;
                r.allow_ahead = allow_ahead;
                self.save_campaign_to_disk();
            }
        } else {
            let initial_state = pkt.local_player;

            info!(
                "[Room {room_str} | JOIN] Slot P{slot_idx} assigned to {addr} (OTID: 0x{:08X} | Map: {} | Ready: {})",
                initial_state.trainer_id, initial_state.map_id, in_bedroom_ready
            );

            self.slots[slot_idx] = Some(ConnectedPlayer {
                slot_id: slot_idx as u8,
                addr,
                last_seen: now,
                last_sequence: pkt.sequence,
                heartbeat_rom: pkt.heartbeat_rom,
                in_bedroom_ready,
                state: initial_state,
                badge_mask: pkt.badge_mask_local,
                assigned_monolocke,
                player_secret: pkt.player_secret,
                allow_ahead: pkt.allow_ahead == 1,
                // An operation latched before (re)joining is stale: ignore it.
                last_session_seq: pkt.session.seq,
                session_view: SessionView::default(),
            });
        }

        self.check_and_lock_start_gate();
        Ok(slot_idx as u8)
    }

    fn resolve_or_assign_slot(
        &mut self,
        addr: SocketAddr,
        requested_id: u8,
        trainer_id: u32,
        player_secret: u64,
    ) -> Result<usize, u8> {
        if self.campaign_started {
            return self.resolve_campaign_slot(addr, trainer_id, player_secret);
        }

        // Existing active socket match
        for (idx, slot) in self.slots.iter().enumerate() {
            if let Some(player) = slot
                && player.addr == addr
            {
                return Ok(idx);
            }
        }

        // Forming campaign: honor requested slot if free
        if (requested_id as usize) < self.target_party_size && requested_id != AUTO_PLAYER_ID {
            let idx = requested_id as usize;
            if self.slots[idx].is_none() {
                return Ok(idx);
            }
        }

        // Auto-assign lowest free slot up to target_party_size
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

    /// Slot resolution once the roster is locked. A player is identified by
    /// OTID (after loading their save) plus their bridge's player secret, or
    /// by the secret alone while still at the title screen. Unrecognized
    /// title-screen guests may borrow a free roster slot but are displaced
    /// as soon as that slot's owner shows up.
    fn resolve_campaign_slot(
        &mut self,
        addr: SocketAddr,
        trainer_id: u32,
        player_secret: u64,
    ) -> Result<usize, u8> {
        let room_str = room_code_to_string(&self.room_code);

        let reg_idx = if trainer_id != 0 {
            let Some(idx) = self.roster.iter().position(|r| r.trainer_id == trainer_id) else {
                warn!(
                    "[Room {room_str}] Rejected {addr} (OTID: 0x{trainer_id:08X}) — TrainerID is not in locked campaign roster!"
                );
                return Err(ROOM_STATUS_ERR_LOCKED);
            };
            let reg = &mut self.roster[idx];
            if reg.player_secret == 0 && player_secret != 0 {
                // Legacy campaign file: the first claimant binds the slot.
                reg.player_secret = player_secret;
                info!("[Room {room_str}] Bound OTID 0x{trainer_id:08X} to {addr}'s bridge");
                self.save_campaign_to_disk();
            } else if reg.player_secret != player_secret {
                warn!(
                    "[Room {room_str}] Rejected {addr}: OTID 0x{trainer_id:08X} is registered to a different bridge"
                );
                return Err(ROOM_STATUS_ERR_LOCKED);
            }
            Some(idx)
        } else {
            self.roster
                .iter()
                .position(|r| r.player_secret != 0 && r.player_secret == player_secret)
        };

        let current = self
            .slots
            .iter()
            .position(|s| s.as_ref().is_some_and(|p| p.addr == addr));

        let Some(reg_idx) = reg_idx else {
            // Unrecognized title-screen guest: keep its slot or borrow a free one.
            if let Some(cur) = current {
                return Ok(cur);
            }
            if let Some(idx) = self.free_roster_slot() {
                return Ok(idx);
            }
            warn!(
                "[Room {room_str}] Rejected {addr} — All {} registered roster slots are currently occupied",
                self.roster.len()
            );
            return Err(ROOM_STATUS_ERR_LOCKED);
        };

        let reg_slot = self.roster[reg_idx].slot_id as usize;
        let owner_secret = self.roster[reg_idx].player_secret;
        if current == Some(reg_slot) {
            return Ok(reg_slot);
        }

        // Lift our own entry out of any borrowed slot before rearranging.
        let mine = current.and_then(|cur| self.slots[cur].take());

        let occupied_by_guest = self.slots[reg_slot]
            .as_ref()
            .is_some_and(|p| owner_secret == 0 || p.player_secret != owner_secret);
        if occupied_by_guest {
            // Someone else borrowed our slot: move them to a free roster slot,
            // or drop them (their next packet is resolved again).
            let guest = self.slots[reg_slot].take();
            if let (Some(mut guest), Some(free)) = (guest, self.free_roster_slot()) {
                guest.slot_id = free as u8;
                self.slots[free] = Some(guest);
            }
        }
        // else: the slot holds our own previous socket (bridge restarted);
        // the caller updates it in place, taking it over.

        if let Some(mut mine) = mine {
            mine.slot_id = reg_slot as u8;
            self.slots[reg_slot] = Some(mine);
        }
        Ok(reg_slot)
    }

    fn free_roster_slot(&self) -> Option<usize> {
        self.roster
            .iter()
            .map(|r| r.slot_id as usize)
            .find(|&idx| self.slots[idx].is_none())
    }

    fn check_and_lock_start_gate(&mut self) {
        if self.campaign_started {
            return;
        }

        let active: Vec<&ConnectedPlayer> = self.slots.iter().filter_map(|s| s.as_ref()).collect();
        if active.len() < self.target_party_size {
            return;
        }

        // Automatically lock and start once every active player reports
        // client_ready == 1 with a valid TrainerID. The relay never inspects
        // map numbers or badge masks.
        let all_ready = active
            .iter()
            .all(|p| p.state.client_ready == 1 && p.state.trainer_id != 0);
        if !all_ready {
            return;
        }

        self.campaign_started = true;
        self.roster.clear();

        for p in &active {
            self.roster.push(RegisteredTrainer {
                slot_id: p.slot_id,
                trainer_id: p.state.trainer_id,
                last_badge_mask: p.badge_mask,
                assigned_monolocke: p.assigned_monolocke,
                player_secret: p.player_secret,
                allow_ahead: p.allow_ahead,
            });
        }

        info!(
            "=== [Room {} | START GATE UNLOCKED] Roster Locked with {}/{} Players! Campaign Started! ===",
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
            if let Some(player) = slot
                && now.duration_since(player.last_seen) > self.client_timeout
            {
                info!(
                    "[Room {room_str} | TIMEOUT] Slot P{idx} ({}) went offline",
                    player.addr
                );
                *slot = None;
            }
        }
        self.drop_sessions_of_missing_players();
    }

    pub fn build_broadcast_batch(&mut self) -> Vec<(SocketAddr, [u8; SERVER_PACKET_SIZE])> {
        self.server_tick = self.server_tick.wrapping_add(1);
        self.expire_sessions(Instant::now());

        let active_mask = self.compute_active_bitmask();
        let online_count = active_mask.count_ones() as u8;
        let ready_count = self
            .slots
            .iter()
            .filter_map(|s| s.as_ref())
            .filter(|p| p.in_bedroom_ready)
            .count() as u8;
        let expected_size = self.expected_party_size() as u8;

        let synced_badges = self.compute_synced_badges();

        let mut batch = Vec::with_capacity(MAX_PLAYERS);

        for recipient_idx in 0..MAX_PLAYERS {
            let Some(recipient) = &self.slots[recipient_idx] else {
                continue;
            };

            // Every other member's state, untouched: the receiving ROM decides
            // what to show (same matrix and map, distance, renderer budget).
            // Players outside the field keep their last position with
            // is_active = 0, so their ghost can stay frozen in place.
            let mut remote_players = [PlayerState::default(); REMOTE_SLOTS];
            let mut remote_cursor = 0;
            for other_idx in 0..MAX_PLAYERS {
                if other_idx == recipient_idx {
                    continue;
                }
                if let Some(other) = &self.slots[other_idx] {
                    remote_players[remote_cursor] = other.state;
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
                badge_mask_local_echo: recipient.badge_mask,
                badge_mask_synced: synced_badges,
                flag_coop_sync_ready: u8::from(self.gym_gate_open_for(recipient)),
                current_checkpoint: synced_badges.count_ones() as u8,
                assigned_monolocke: recipient.assigned_monolocke,
                nuzlocke_failed: 0,
                flag_campaign_started: u8::from(self.campaign_started),
                expected_party_size: expected_size,
                online_party_count: online_count,
                ready_party_count: ready_count,
                room_uid: self.room_uid,
                team_partner_slot: self.team_partner_remote_slot(recipient_idx),
                teamed_mask: self.teamed_mask_for(recipient_idx),
                session_state: recipient.session_view.state,
                session_kind: recipient.session_view.kind,
                session_peer_slot: recipient
                    .session_view
                    .peer
                    .filter(|&p| self.slots[p].is_some())
                    .map_or(NO_SLOT, |p| remote_slot_of(recipient_idx, p)),
                session_seq: recipient.session_view.seq,
                session_reason: recipient.session_view.reason,
                gates_pad: 0,
            };

            batch.push((recipient.addr, pkt.encode_le()));
        }

        batch
    }

    /// Badges every roster member has (offline members count with their
    /// last known badges).
    fn compute_synced_badges(&self) -> u16 {
        let mut common = 0xFFFFu16;
        let mut any = false;
        for p in self.slots.iter().flatten() {
            common &= p.badge_mask;
            any = true;
        }
        if self.campaign_started {
            for reg in &self.roster {
                if self.slots[reg.slot_id as usize].is_none() {
                    common &= reg.last_badge_mask;
                    any = true;
                }
            }
        }
        if any { common } else { 0 }
    }

    /// Gym / boss gate (docs/MILESTONE2_PLAN.md 7.2), per player and
    /// game-agnostic: `recipient` may challenge a gated boss unless a roster
    /// teammate has fewer badges and is neither in the same map nor letting
    /// others go ahead. Everyone can be at most one badge ahead. Offline
    /// teammates count with their last known badges and choice.
    fn gym_gate_open_for(&self, recipient: &ConnectedPlayer) -> bool {
        if !self.campaign_started {
            return false;
        }
        let mine = recipient.badge_mask.count_ones();
        self.roster
            .iter()
            .filter(|reg| reg.slot_id != recipient.slot_id)
            .all(|reg| match &self.slots[reg.slot_id as usize] {
                Some(p) => {
                    p.badge_mask.count_ones() >= mine
                        || p.allow_ahead
                        || (p.state.map_id != 0 && p.state.map_id == recipient.state.map_id)
                }
                None => reg.last_badge_mask.count_ones() >= mine || reg.allow_ahead,
            })
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
        let connected: Vec<&ConnectedPlayer> =
            self.slots.iter().filter_map(|s| s.as_ref()).collect();
        if connected.is_empty() {
            return;
        }
        let synced_badges = self.compute_synced_badges();
        let room_str = room_code_to_string(&self.room_code);
        info!(
            "--- [Room {room_str} | Online: {}/{} | Started: {} | Synced Badges: 0b{:016b}] ---",
            connected.len(),
            self.expected_party_size(),
            self.campaign_started,
            synced_badges
        );
        for p in connected {
            info!(
                "    Slot P{} ({}) -> OTID: 0x{:08X} | InBedroom: {} | Gym Gate: {} | Map {:3} | XZ: ({:3}, {:3}) | Activity: {} | Follower: #{:4}",
                p.slot_id,
                p.addr,
                p.state.trainer_id,
                p.in_bedroom_ready,
                if self.gym_gate_open_for(p) {
                    "open"
                } else {
                    "closed"
                },
                p.state.map_id,
                p.state.x,
                p.state.z,
                p.state.activity,
                p.state.follower_species
            );
        }
    }

    // -----------------------------------------------------------------
    // Player-to-player sessions and teams (docs/MILESTONE3_PLAN.md, Phase 1)
    // -----------------------------------------------------------------

    /// The teammate's trainer ID, if this trainer is in a team.
    pub fn team_partner_of(&self, trainer_id: u32) -> Option<u32> {
        self.teams.iter().find_map(|t| {
            if t[0] == trainer_id {
                Some(t[1])
            } else if t[1] == trainer_id {
                Some(t[0])
            } else {
                None
            }
        })
    }

    pub fn teams(&self) -> &[[u32; 2]] {
        &self.teams
    }

    fn trainer_at(&self, slot: usize) -> Option<u32> {
        self.slots[slot].as_ref().map(|p| p.state.trainer_id)
    }

    fn is_teamed(&self, slot: usize) -> bool {
        self.trainer_at(slot)
            .is_some_and(|t| t != 0 && self.team_partner_of(t).is_some())
    }

    fn in_session(&self, slot: usize) -> bool {
        self.sessions.iter().any(|s| s.from == slot || s.to == slot)
    }

    fn set_view(&mut self, slot: usize, state: u8, kind: u8, peer: Option<usize>, reason: u8) {
        if let Some(p) = self.slots[slot].as_mut() {
            let seq = p.session_view.seq.wrapping_add(1).max(1);
            p.session_view = SessionView {
                state,
                kind,
                peer,
                reason,
                seq,
            };
        }
    }

    /// Applies one new session operation from `slot` (already deduplicated).
    fn apply_session_op(&mut self, slot: usize, out: SessionOut, now: Instant) {
        let room_str = room_code_to_string(&self.room_code);
        match out.op {
            SESSION_OP_REQUEST => {
                let kind = out.kind;
                let target = (0..MAX_PLAYERS).find(|&j| {
                    j != slot
                        && out.target_trainer != 0
                        && self.trainer_at(j) == Some(out.target_trainer)
                });
                let refuse = |lobby: &mut Self, reason: u8| {
                    lobby.set_view(slot, SESSION_STATE_REFUSED, kind, target, reason);
                };
                if !matches!(
                    kind,
                    SESSION_KIND_TRADE | SESSION_KIND_BATTLE | SESSION_KIND_TEAM_UP
                ) || self.trainer_at(slot) == Some(out.target_trainer)
                {
                    return refuse(self, SESSION_REASON_INVALID);
                }
                let Some(target) = target else {
                    return refuse(self, SESSION_REASON_NOT_HERE);
                };
                if self.in_session(slot) || self.in_session(target) {
                    return refuse(self, SESSION_REASON_BUSY);
                }
                if kind == SESSION_KIND_TEAM_UP {
                    if !self.campaign_started {
                        return refuse(self, SESSION_REASON_INVALID);
                    }
                    if self.is_teamed(slot) || self.is_teamed(target) {
                        return refuse(self, SESSION_REASON_ALREADY_TEAMED);
                    }
                }
                if kind == SESSION_KIND_BATTLE {
                    // Teams only battle teams (team vs solo is in the backlog),
                    // and never their own teammate.
                    let me = self.trainer_at(slot).unwrap_or(0);
                    let same_team = self.team_partner_of(me) == self.trainer_at(target);
                    if self.is_teamed(slot) != self.is_teamed(target) || same_team {
                        return refuse(self, SESSION_REASON_TEAM_RULE);
                    }
                }
                info!("[Room {room_str} | SESSION] P{slot} asks P{target} (kind {kind})");
                self.sessions.push(Session {
                    kind,
                    from: slot,
                    to: target,
                    created: now,
                });
                self.set_view(
                    slot,
                    SESSION_STATE_OUTGOING,
                    kind,
                    Some(target),
                    SESSION_REASON_NONE,
                );
                self.set_view(
                    target,
                    SESSION_STATE_INCOMING,
                    kind,
                    Some(slot),
                    SESSION_REASON_NONE,
                );
            }
            SESSION_OP_ACCEPT | SESSION_OP_DECLINE => {
                let Some(i) = self.sessions.iter().position(|s| s.to == slot) else {
                    return;
                };
                let session = self.sessions.remove(i);
                let accepted = out.op == SESSION_OP_ACCEPT;
                let state = if accepted {
                    SESSION_STATE_ACCEPTED
                } else {
                    SESSION_STATE_DECLINED
                };
                info!(
                    "[Room {room_str} | SESSION] P{} {} P{}'s request (kind {})",
                    session.to,
                    if accepted { "accepted" } else { "declined" },
                    session.from,
                    session.kind
                );
                if accepted
                    && session.kind == SESSION_KIND_TEAM_UP
                    && let (Some(a), Some(b)) =
                        (self.trainer_at(session.from), self.trainer_at(session.to))
                    && self.team_partner_of(a).is_none()
                    && self.team_partner_of(b).is_none()
                {
                    self.teams.push([a, b]);
                    info!("[Room {room_str} | TEAM] 0x{a:08X} and 0x{b:08X} teamed up");
                    self.save_campaign_to_disk();
                }
                self.set_view(
                    session.from,
                    state,
                    session.kind,
                    Some(session.to),
                    SESSION_REASON_NONE,
                );
                self.set_view(
                    session.to,
                    state,
                    session.kind,
                    Some(session.from),
                    SESSION_REASON_NONE,
                );
            }
            SESSION_OP_CANCEL => {
                let Some(i) = self.sessions.iter().position(|s| s.from == slot) else {
                    return;
                };
                let session = self.sessions.remove(i);
                self.set_view(
                    session.from,
                    SESSION_STATE_CANCELLED,
                    session.kind,
                    Some(session.to),
                    SESSION_REASON_NONE,
                );
                self.set_view(
                    session.to,
                    SESSION_STATE_CANCELLED,
                    session.kind,
                    Some(session.from),
                    SESSION_REASON_NONE,
                );
            }
            _ => {}
        }
    }

    /// Requests nobody answered in time.
    pub fn expire_sessions(&mut self, now: Instant) {
        let (expired, kept): (Vec<Session>, Vec<Session>) = self
            .sessions
            .drain(..)
            .partition(|s| now.duration_since(s.created) >= SESSION_TIMEOUT);
        self.sessions = kept;
        for s in expired {
            self.set_view(
                s.from,
                SESSION_STATE_TIMEOUT,
                s.kind,
                Some(s.to),
                SESSION_REASON_NONE,
            );
            self.set_view(
                s.to,
                SESSION_STATE_TIMEOUT,
                s.kind,
                Some(s.from),
                SESSION_REASON_NONE,
            );
        }
    }

    /// A side went offline: the other side's request is cancelled.
    fn drop_sessions_of_missing_players(&mut self) {
        let (gone, kept): (Vec<Session>, Vec<Session>) = self
            .sessions
            .drain(..)
            .partition(|s| self.slots[s.from].is_none() || self.slots[s.to].is_none());
        self.sessions = kept;
        for s in gone {
            self.set_view(
                s.from,
                SESSION_STATE_CANCELLED,
                s.kind,
                Some(s.to),
                SESSION_REASON_NONE,
            );
            self.set_view(
                s.to,
                SESSION_STATE_CANCELLED,
                s.kind,
                Some(s.from),
                SESSION_REASON_NONE,
            );
        }
    }

    fn team_partner_remote_slot(&self, recipient: usize) -> u8 {
        let Some(partner) = self
            .trainer_at(recipient)
            .and_then(|t| self.team_partner_of(t))
        else {
            return NO_SLOT;
        };
        (0..MAX_PLAYERS)
            .find(|&j| j != recipient && self.trainer_at(j) == Some(partner))
            .map_or(NO_SLOT, |j| remote_slot_of(recipient, j))
    }

    fn teamed_mask_for(&self, recipient: usize) -> u8 {
        let mut mask = if self.is_teamed(recipient) {
            TEAMED_SELF_BIT
        } else {
            0
        };
        for j in (0..MAX_PLAYERS).filter(|&j| j != recipient) {
            if self.is_teamed(j) {
                mask |= 1 << remote_slot_of(recipient, j);
            }
        }
        mask
    }

    fn save_file_path(&self) -> Option<PathBuf> {
        let dir = self.save_dir.as_ref()?;
        let room_str = room_code_to_string(&self.room_code);
        Some(dir.join(format!("{room_str}.json")))
    }

    /// Persists the campaign atomically (write to a temp file, then rename),
    /// so a crash mid-write can never leave a truncated roster behind.
    fn save_campaign_to_disk(&self) {
        let Some(path) = self.save_file_path() else {
            return;
        };
        if let Some(parent) = path.parent()
            && let Err(err) = fs::create_dir_all(parent)
        {
            error!(
                "[Room {}] Cannot create save dir {parent:?}: {err}",
                room_code_to_string(&self.room_code)
            );
            return;
        }
        let payload = CampaignSaveFile {
            room_code: room_code_to_string(&self.room_code),
            room_uid: self.room_uid,
            host_token: self.host_token,
            campaign_started: self.campaign_started,
            expected_party_size: self.roster.len() as u8,
            roster: self.roster.clone(),
            teams: self.teams.clone(),
        };
        let json = match serde_json::to_string_pretty(&payload) {
            Ok(json) => json,
            Err(err) => {
                error!(
                    "[Room {}] Cannot serialize campaign: {err}",
                    payload.room_code
                );
                return;
            }
        };
        let tmp_path = path.with_extension("json.tmp");
        if let Err(err) =
            write_private(&tmp_path, json.as_bytes()).and_then(|()| fs::rename(&tmp_path, &path))
        {
            error!(
                "[Room {}] Failed to save campaign to {path:?}: {err}",
                payload.room_code
            );
        }
    }

    fn load_campaign_from_disk(&mut self) -> Result<(), CampaignLoadError> {
        let Some(path) = self.save_file_path() else {
            return Ok(());
        };
        if !path.exists() {
            return Ok(());
        }
        let raw = fs::read_to_string(&path)
            .map_err(|err| CampaignLoadError(format!("cannot read {path:?}: {err}")))?;
        let saved = serde_json::from_str::<CampaignSaveFile>(&raw)
            .map_err(|err| CampaignLoadError(format!("cannot parse {path:?}: {err}")))?;

        self.host_token = saved.host_token;
        self.campaign_started = saved.campaign_started;
        self.roster = saved.roster;
        self.teams = saved.teams;
        self.target_party_size = (saved.expected_party_size as usize).clamp(2, MAX_PLAYERS);
        info!(
            "[Room {}] Restored locked campaign from disk ({} registered trainers)",
            saved.room_code,
            self.roster.len()
        );
        if saved.room_uid != 0 {
            self.room_uid = saved.room_uid;
        } else {
            // Written before room UIDs existed: keep the one generated above.
            self.save_campaign_to_disk();
        }
        Ok(())
    }
}

/// Where absolute slot `other` appears in `recipient`'s remote_players
/// (the broadcast lists every other slot in order, skipping the recipient).
fn remote_slot_of(recipient: usize, other: usize) -> u8 {
    if other < recipient {
        other as u8
    } else {
        (other - 1) as u8
    }
}

/// A random, non-zero room identity. It only has to differ between rooms
/// that reuse a code (the per-room save files are keyed by it), not resist
/// guessing, so a per-process random hasher over the code and time is enough.
fn new_room_uid(room_code: [u8; 6]) -> u32 {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let uid = RandomState::new().hash_one((room_code, nanos)) as u32;
    uid.max(1)
}

/// Writes a file readable only by the relay's user on Unix: campaign files
/// contain each player's bridge secret.
fn write_private(path: &Path, contents: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    let mut options = fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options.open(path)?.write_all(contents)
}

/// Rooms with nobody connected are dropped from memory after this long.
/// Started campaigns live on disk and are reloaded on the next JOIN/SYNC.
pub const DEFAULT_ROOM_IDLE_TIMEOUT: Duration = Duration::from_secs(120);
/// Upper bound on rooms held in memory at once.
pub const DEFAULT_MAX_ROOMS: usize = 256;
/// Rooms one IP may host at once, so a single client cannot exhaust the cap.
pub const MAX_ROOMS_PER_HOST_IP: usize = 4;
/// Rejected admissions (wrong code, locked roster, ...) allowed per IP in a
/// burst, refilled at `FAILURE_REFILL_PER_SEC`: ~30 guesses/minute at most.
pub const FAILURE_BURST: u32 = 10;
pub const FAILURE_REFILL_PER_SEC: f64 = 0.5;

/// Why a packet was not accepted; `main` turns this into the reply.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Rejection {
    /// A `ROOM_STATUS_*` error code.
    Status(u8),
    /// No valid cookie yet: reply with this cookie and let the client retry.
    Challenge(u64),
}

pub struct RoomManager {
    rooms: HashMap<[u8; 6], LobbyState>,
    client_timeout: Duration,
    default_party_size: usize,
    inspect_all_packets: bool,
    save_dir: PathBuf,
    room_idle_timeout: Duration,
    max_rooms: usize,
    cookies: CookieJar,
    failures: FailureLimiter,
    /// Trainer ID -> rooms whose locked roster holds it (every campaign file
    /// in `save_dir`, plus rooms that lock while running). A save registered
    /// in one campaign cannot join another room.
    trainer_rooms: HashMap<u32, Vec<[u8; 6]>>,
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
            room_idle_timeout: DEFAULT_ROOM_IDLE_TIMEOUT,
            max_rooms: DEFAULT_MAX_ROOMS,
            cookies: CookieJar::new(),
            failures: FailureLimiter::new(FAILURE_BURST, FAILURE_REFILL_PER_SEC),
            trainer_rooms: index_campaign_trainers(save_dir.as_ref()),
        }
    }

    fn register_trainer(&mut self, trainer_id: u32, code: [u8; 6]) {
        let rooms = self.trainer_rooms.entry(trainer_id).or_default();
        if !rooms.contains(&code) {
            rooms.push(code);
        }
    }

    /// A trainer locked in another campaign may not use this room.
    fn trainer_belongs_elsewhere(&self, trainer_id: u32, code: [u8; 6]) -> bool {
        trainer_id != 0
            && self
                .trainer_rooms
                .get(&trainer_id)
                .is_some_and(|rooms| !rooms.contains(&code))
    }

    pub fn with_limits(mut self, room_idle_timeout: Duration, max_rooms: usize) -> Self {
        self.room_idle_timeout = room_idle_timeout;
        self.max_rooms = max_rooms.max(1);
        self
    }

    pub fn room_count(&self) -> usize {
        self.rooms.len()
    }

    /// The cookie a challenge reply would hand to `addr` right now.
    pub fn cookie_for(&self, addr: SocketAddr) -> u64 {
        self.cookies.issue(addr, Instant::now())
    }

    /// Admission gate, then room dispatch. Returns the assigned slot, or how
    /// to reply (see `ServerToClientPacket::create_error_response` /
    /// `create_challenge_response`).
    pub fn handle_client_packet(
        &mut self,
        addr: SocketAddr,
        pkt: ClientToServerPacket,
    ) -> Result<u8, Rejection> {
        let now = Instant::now();

        // Cookie first: nothing below (rooms, slots, broadcasts, LEAVE, even
        // the failure limiter's memory) is reachable from a spoofed address.
        if !self.cookies.verify(addr, pkt.cookie, now) {
            return Err(Rejection::Challenge(self.cookies.issue(addr, now)));
        }
        if !self.failures.allows(addr.ip(), now) {
            return Err(Rejection::Status(ROOM_STATUS_ERR_RATE_LIMITED));
        }
        self.dispatch(addr, pkt).map_err(|status| {
            self.failures.record_failure(addr.ip(), now);
            Rejection::Status(status)
        })
    }

    fn dispatch(&mut self, addr: SocketAddr, pkt: ClientToServerPacket) -> Result<u8, u8> {
        // 1. Strictly enforce 6-character alphanumeric Room Code
        let code = validate_room_code(pkt.room_code).inspect_err(|_| {
            warn!("[RoomManager] Rejected invalid room code from {addr}");
        })?;

        let room_str = room_code_to_string(&code);
        let save_path = self.save_dir.join(format!("{room_str}.json"));
        let room_in_memory = self.rooms.contains_key(&code);
        let room_on_disk = save_path.exists();

        if pkt.room_action != ROOM_ACTION_LEAVE
            && self.trainer_belongs_elsewhere(pkt.local_player.trainer_id, code)
        {
            warn!(
                "[RoomManager] Rejected {addr} for room {room_str}: OTID 0x{:08X} belongs to another campaign",
                pkt.local_player.trainer_id
            );
            if let Some(room) = self.rooms.get_mut(&code) {
                room.remove_client_by_addr(addr);
            }
            return Err(ROOM_STATUS_ERR_SAVE_OTHER_ROOM);
        }

        match pkt.room_action {
            ROOM_ACTION_LEAVE => {
                if let Some(room) = self.rooms.get_mut(&code) {
                    room.remove_client_by_addr(addr);
                }
                Ok(0)
            }

            ROOM_ACTION_CREATE => {
                // Reject if room already exists on disk
                if room_on_disk {
                    warn!(
                        "[RoomManager] Rejected CREATE for room {room_str} from {addr}: campaign already exists on disk"
                    );
                    return Err(ROOM_STATUS_ERR_ALREADY_EXISTS);
                }

                // Reject if room already exists in memory (unless it is an in-flight 60Hz handshake packet from the exact creator socket & token)
                if let Some(existing_room) = self.rooms.get(&code) {
                    let is_same_creator_handshake = pkt.host_token != 0
                        && existing_room.host_token() == pkt.host_token
                        && existing_room.host_addr() == Some(addr);

                    if !is_same_creator_handshake {
                        warn!(
                            "[RoomManager] Rejected CREATE for room {room_str} from {addr}: room already exists"
                        );
                        return Err(ROOM_STATUS_ERR_ALREADY_EXISTS);
                    }
                }

                let party_size = (pkt.target_party_size as usize)
                    .clamp(2, MAX_PLAYERS)
                    .max(self.default_party_size);
                let room = self.get_or_open_room(code, pkt.host_token, Some(addr), party_size)?;
                let result = room.handle_client_packet(addr, pkt);
                self.register_if_locked(code);
                result
            }

            ROOM_ACTION_JOIN | ROOM_ACTION_SYNC => {
                if !room_in_memory && !room_on_disk {
                    warn!(
                        "[RoomManager] Rejected {} from {addr}: room {room_str} does not exist",
                        if pkt.room_action == ROOM_ACTION_JOIN {
                            "JOIN"
                        } else {
                            "SYNC"
                        }
                    );
                    return Err(ROOM_STATUS_ERR_NOT_FOUND);
                }

                let party_size = self.default_party_size;
                let room = self.get_or_open_room(code, 0, None, party_size)?;
                let result = room.handle_client_packet(addr, pkt);
                self.register_if_locked(code);
                result
            }

            _ => Err(ROOM_STATUS_ERR_NOT_FOUND),
        }
    }

    /// Adds a room's roster to the trainer index once it is locked.
    fn register_if_locked(&mut self, code: [u8; 6]) {
        let ids: Vec<u32> = match self.rooms.get(&code) {
            Some(room) if room.is_campaign_started() => room.roster_trainer_ids().collect(),
            _ => return,
        };
        for id in ids {
            self.register_trainer(id, code);
        }
    }

    /// Returns the in-memory room, or opens it (fresh, or restored from its
    /// campaign file). Refuses when the room cap is reached or the campaign
    /// file is unreadable.
    fn get_or_open_room(
        &mut self,
        code: [u8; 6],
        host_token: u32,
        host_addr: Option<SocketAddr>,
        party_size: usize,
    ) -> Result<&mut LobbyState, u8> {
        if !self.rooms.contains_key(&code) {
            let room_str = room_code_to_string(&code);
            if self.rooms.len() >= self.max_rooms {
                warn!(
                    "[RoomManager] Refused room {room_str}: room limit ({}) reached",
                    self.max_rooms
                );
                return Err(ROOM_STATUS_ERR_FULL);
            }
            if let Some(host) = host_addr {
                let hosted = self
                    .rooms
                    .values()
                    .filter(|r| r.host_addr().is_some_and(|a| a.ip() == host.ip()))
                    .count();
                if hosted >= MAX_ROOMS_PER_HOST_IP {
                    warn!(
                        "[RoomManager] Refused room {room_str}: {} already hosts {hosted} rooms",
                        host.ip()
                    );
                    return Err(ROOM_STATUS_ERR_FULL);
                }
            }
            let room = LobbyState::with_room(
                code,
                host_token,
                host_addr,
                self.client_timeout,
                party_size,
                self.inspect_all_packets,
                Some(self.save_dir.clone()),
            )
            .map_err(|CampaignLoadError(reason)| {
                error!("[RoomManager] Refused room {room_str}: {reason}");
                ROOM_STATUS_ERR_LOCKED
            })?;
            if let Some(addr) = host_addr {
                info!(
                    "[RoomManager] Created room {room_str} (Host: {addr}, Token: 0x{host_token:08X}, Party Size: {party_size})"
                );
            }
            self.rooms.insert(code, room);
        }
        Ok(self.rooms.get_mut(&code).expect("room inserted above"))
    }

    pub fn tick_all_rooms(&mut self) -> Vec<(SocketAddr, [u8; SERVER_PACKET_SIZE])> {
        let now = Instant::now();
        let idle_timeout = self.room_idle_timeout;
        let mut all_packets = Vec::new();
        for room in self.rooms.values_mut() {
            room.prune_stale_clients();
            all_packets.extend(room.build_broadcast_batch());
        }
        self.failures.prune(now);
        self.rooms.retain(|code, room| {
            let keep = !room.is_idle(now, idle_timeout);
            if !keep {
                info!(
                    "[RoomManager] Closed idle room {}",
                    room_code_to_string(code)
                );
            }
            keep
        });
        all_packets
    }

    pub fn print_all_summaries(&self) {
        for room in self.rooms.values() {
            room.print_periodic_summary();
        }
    }
}

/// Trainer ID -> rooms, from every readable campaign file in `dir`.
/// Unreadable files are skipped here; opening that room reports them.
fn index_campaign_trainers(dir: &Path) -> HashMap<u32, Vec<[u8; 6]>> {
    let mut index: HashMap<u32, Vec<[u8; 6]>> = HashMap::new();
    let Ok(entries) = fs::read_dir(dir) else {
        return index;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        let Some(saved) = fs::read_to_string(&path)
            .ok()
            .and_then(|raw| serde_json::from_str::<CampaignSaveFile>(&raw).ok())
        else {
            continue;
        };
        let Ok(code) = <[u8; 6]>::try_from(saved.room_code.as_bytes()) else {
            continue;
        };
        if !saved.campaign_started {
            continue;
        }
        for reg in &saved.roster {
            let rooms = index.entry(reg.trainer_id).or_default();
            if !rooms.contains(&code) {
                rooms.push(code);
            }
        }
    }
    index
}
