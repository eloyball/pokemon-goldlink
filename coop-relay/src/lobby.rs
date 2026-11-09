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
    ACTIVITY_BATTLE, AUTO_PLAYER_ID, COOP_MAGIC, ClientToServerPacket, LinkDown, MAX_PLAYERS,
    TradeIn, TradeOut, encode_trade_down,
    NO_SLOT, PROTOCOL_VERSION, PlayerState, REMOTE_SLOTS, ROOM_ACTION_CREATE, ROOM_ACTION_JOIN,
    ROOM_ACTION_LEAVE, ROOM_ACTION_SYNC, ROOM_STATUS_ERR_ALREADY_EXISTS, ROOM_STATUS_ERR_FULL,
    ROOM_STATUS_ERR_LOCKED, ROOM_STATUS_ERR_MIDGAME_SAVE, ROOM_STATUS_ERR_NOT_FOUND,
    ROOM_STATUS_ERR_RATE_LIMITED, ROOM_STATUS_ERR_RUN_LOST, ROOM_STATUS_ERR_SAVE_OTHER_ROOM,
    ROOM_STATUS_OK, SAVE_STATUS_IN_PROGRESS, SESSION_KIND_BATTLE, SESSION_KIND_LEAVE_TEAM,
    SESSION_KIND_REVIVE, SESSION_KIND_TEAM_BATTLE, SESSION_KIND_TEAM_UP, SESSION_KIND_TRADE,
    SESSION_OP_ACCEPT, SESSION_OP_CANCEL, SESSION_OP_DECLINE, SESSION_OP_REQUEST,
    SESSION_REASON_ALREADY_TEAMED, SESSION_REASON_BUSY, SESSION_REASON_INVALID,
    SESSION_REASON_NONE, SESSION_REASON_NOT_HERE, SESSION_REASON_TEAM_RULE,
    SESSION_REASON_TEAMMATE_AWAY, SESSION_STATE_ACCEPTED, SESSION_STATE_CANCELLED,
    SESSION_STATE_DECLINED, SESSION_STATE_INCOMING, SESSION_STATE_NOTICE, SESSION_STATE_OUTGOING,
    SESSION_STATE_REFUSED, SESSION_STATE_TIMEOUT, ServerToClientPacket, SessionOut,
    TEAMED_SELF_BIT, room_code_to_string, validate_room_code,
};
use crate::protocol::{
    CHALLENGE_FLAG_SPIRIT, CHALLENGE_LOCATION_BYTES, CHALLENGE_RULE_MONOLOCKE,
    CHALLENGE_RULE_NUZLOCKE, CHALLENGE_STATE_RUN_LOST, CHALLENGE_STATE_SPIRIT, ChallengeDown,
    ChallengeUp, MONO_TYPE_COUNT, MONO_TYPE_NONE, ROOM_STATE_RUN_LOST, SOUL_LINK_DUO,
    SOUL_LINK_ROOM_WIDE, sanitize_challenge_rules, soul_link_mode,
};

mod link;
mod trade;
pub use link::{
    LINK_DEADLINE, LINK_RESEND, LINK_SILENCE, LINK_STALL, LinkStreamEvent, LinkStreamOut,
};

/// What a QUIC player's trade stream did (protocol 9).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TradeStreamEvent {
    /// Opened: its ROM opened a trade exchange.
    Open,
    /// The ROM's TradeOut changed.
    Up(Box<TradeOut>),
    /// Finished: the exchange closed.
    Closed,
}

/// A trade frame for a QUIC player's trade stream (`encode_trade_down`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TradeStreamOut {
    pub addr: SocketAddr,
    pub frame: Vec<u8>,
}

/// A QUIC player's open trade stream: its latest TradeOut, and the TradeIn last sent on it.
#[derive(Debug, Clone)]
struct TradeStream {
    addr: SocketAddr,
    latest: Option<TradeOut>,
    sent: Option<Option<TradeIn>>,
}
pub use trade::{PendingTrade, TRADE_IDLE_TIMEOUT, TRADE_NOTICE_TTL, TRADE_SILENCE_TIMEOUT};

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
    /// Milestone 5 challenge ledger (docs/MILESTONE5_PLAN.md §4.4).
    #[serde(default)]
    pub challenge: TrainerChallenge,
}

/// What the relay keeps of a trainer's challenge run. The location bitmaps
/// only ever gain bits (OR-merged from the ROM's reports), so a restored
/// backup or a re-sent packet can never reopen a route or undo a failure.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct TrainerChallenge {
    #[serde(default)]
    pub spent: [u8; CHALLENGE_LOCATION_BYTES],
    #[serde(default)]
    pub failed: [u8; CHALLENGE_LOCATION_BYTES],
    #[serde(default)]
    pub spirit: bool,
    #[serde(default)]
    pub living: u8,
    #[serde(default)]
    pub deaths: u16,
    /// This trainer's game showed the "Run lost" summary and saved it: from
    /// then on its bridge is turned away (ROOM_STATUS_ERR_RUN_LOST).
    #[serde(default)]
    pub run_lost_seen: bool,
    /// Milestone 5 Duo Soul-Link: `failed` when this trainer teamed up. The
    /// link starts at pairing: older failures don't cross (plan E16).
    #[serde(default)]
    pub pair_baseline: [u8; CHALLENGE_LOCATION_BYTES],
}

impl TrainerChallenge {
    /// Merges a ROM report; TRUE when something worth saving changed.
    fn merge(&mut self, up: &ChallengeUp) -> bool {
        let mut changed = false;
        for i in 0..CHALLENGE_LOCATION_BYTES {
            let spent = self.spent[i] | up.spent[i];
            let failed = self.failed[i] | up.failed[i];
            changed |= spent != self.spent[i] || failed != self.failed[i];
            self.spent[i] = spent;
            self.failed[i] = failed;
        }
        let spirit = up.state & CHALLENGE_STATE_SPIRIT != 0;
        changed |= spirit != self.spirit || up.living != self.living || up.deaths > self.deaths;
        self.spirit = spirit;
        self.living = up.living;
        self.deaths = self.deaths.max(up.deaths);
        if up.state & CHALLENGE_STATE_RUN_LOST != 0 && !self.run_lost_seen {
            self.run_lost_seen = true;
            changed = true;
        }
        changed
    }
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
    /// Committed trades not yet saved by both games (Milestone 3 Phase 2).
    #[serde(default)]
    pub pending_trades: Vec<PendingTrade>,
    /// Milestone 5 room rules (CHALLENGE_RULE_*), fixed once the roster locks.
    #[serde(default)]
    pub rules: u16,
    /// Milestone 5 Ghost Mode: every roster trainer became a spirit. The run
    /// is over: the room turns every join away (ROOM_STATUS_ERR_RUN_LOST).
    #[serde(default)]
    pub run_lost: bool,
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
    /// A team battle's other two players and who still has to accept.
    team: Option<TeamBattle>,
}

#[derive(Debug, Clone)]
struct TeamBattle {
    from_mate: usize,
    to_mate: usize,
    pending: Vec<usize>,
}

impl Session {
    /// Everyone in the session, the requester first.
    fn members(&self) -> Vec<usize> {
        let mut m = vec![self.from, self.to];
        if let Some(t) = &self.team {
            m.push(t.from_mate);
            m.push(t.to_mate);
        }
        m
    }

    /// Players who answer: the target, and in a team battle both teammates.
    fn answers(&self, slot: usize) -> bool {
        match &self.team {
            Some(t) => t.pending.contains(&slot),
            None => self.to == slot,
        }
    }
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
    trades: Vec<trade::ActiveTrade>,
    pending_trades: Vec<PendingTrade>,
    trade_notices: Vec<trade::TradeNotice>,
    /// Virtual link state per slot (Milestone 3 Phase 3).
    links: [link::LinkPeer; MAX_PLAYERS],
    /// Members whose link battle was called off while they were quiet (the referee).
    link_called_off: [Option<link::CalledOff>; MAX_PLAYERS],
    /// Link messages for QUIC members' link streams, until the main loop writes them.
    link_stream_out: Vec<link::LinkStreamOut>,
    /// QUIC players' trade streams (protocol 9), by slot.
    trade_streams: [Option<TradeStream>; MAX_PLAYERS],
    /// Trade frames for them, until the main loop writes them.
    trade_stream_out: Vec<TradeStreamOut>,
    /// Milestone 5 room rules (CHALLENGE_RULE_*): set by the host while the
    /// room forms, fixed once the roster locks.
    rules: u16,
    /// Every roster trainer is a spirit (docs/MILESTONE5_PLAN.md §5.5.5).
    run_lost: bool,
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
            trades: Vec::new(),
            pending_trades: Vec::new(),
            trade_notices: Vec::new(),
            links: Default::default(),
            link_called_off: Default::default(),
            link_stream_out: Vec::new(),
            trade_streams: Default::default(),
            trade_stream_out: Vec::new(),
            rules: 0,
            run_lost: false,
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

    /// A QUIC player's trade stream did something (protocol 9).
    pub fn trade_stream_event(&mut self, addr: SocketAddr, event: TradeStreamEvent) {
        let Some(slot) = self.slot_of_addr(addr) else {
            return;
        };
        let now = Instant::now();
        self.last_activity = now;
        match event {
            TradeStreamEvent::Open => {
                let room_str = room_code_to_string(&self.room_code);
                info!("[Room {room_str} | TRADE] P{slot}'s trade travels on a QUIC stream");
                self.trade_streams[slot] = Some(TradeStream {
                    addr,
                    latest: None,
                    sent: None,
                });
            }
            TradeStreamEvent::Up(out) => {
                if let Some(ts) = self.trade_streams[slot].as_mut().filter(|ts| ts.addr == addr) {
                    ts.latest = Some(*out);
                    self.apply_trade_out(slot, Some(*out), now);
                }
            }
            TradeStreamEvent::Closed => {
                if self.trade_streams[slot].as_ref().is_some_and(|ts| ts.addr == addr) {
                    self.trade_streams[slot] = None;
                }
            }
        }
    }

    /// Trade frames for QUIC players' trade streams, to write now.
    pub fn take_trade_stream_out(&mut self) -> Vec<TradeStreamOut> {
        std::mem::take(&mut self.trade_stream_out)
    }

    /// The slot a player's (current) address holds.
    pub fn slot_of_addr(&self, addr: SocketAddr) -> Option<usize> {
        self.slots
            .iter()
            .position(|s| s.as_ref().is_some_and(|p| p.addr == addr))
    }

    /// A QUIC player's link stream did something (protocol 9). Link messages for UDP members
    /// go out at once, as for a packet's.
    pub fn link_stream_event(&mut self, addr: SocketAddr, event: LinkStreamEvent) {
        let Some(slot) = self.slot_of_addr(addr) else {
            return;
        };
        let now = Instant::now();
        self.last_activity = now;
        self.apply_link_stream(slot, event, now);
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
        // Only the room creator can configure the room while forming: its
        // size and (Milestone 5) the challenge rules.
        let from_host = pkt.host_token != 0 && pkt.host_token == self.host_token;
        if !self.campaign_started && from_host {
            if pkt.target_party_size >= 2 {
                self.target_party_size = (pkt.target_party_size as usize).clamp(2, MAX_PLAYERS);
            }
            self.rules = sanitize_challenge_rules(pkt.challenge_rules);
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
            // A trade stream's latest TradeOut stands in for the packets' trailer (protocol 9).
            let trade = match &self.trade_streams[slot_idx] {
                Some(ts) if ts.addr == addr => ts.latest,
                _ => pkt.trade,
            };
            self.apply_trade_out(slot_idx, trade, now);
            self.apply_link_up(slot_idx, pkt.link, now);

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

        self.apply_challenge_up(slot_idx, pkt.challenge.as_ref());
        // A lost run: a game that has shown the end and saved it is done here
        // (it resets to the title; its next join is refused too).
        if self.run_lost
            && self
                .roster
                .iter()
                .any(|r| r.slot_id as usize == slot_idx && r.challenge.run_lost_seen)
        {
            info!("[Room {room_str} | CHALLENGE] P{slot_idx} saw the end of the run: turned away");
            self.remove_client_by_addr(addr);
            return Err(ROOM_STATUS_ERR_RUN_LOST);
        }
        self.check_and_lock_start_gate();
        Ok(slot_idx as u8)
    }

    /// Milestone 5: merges a ROM's challenge report into its roster entry
    /// (only once the roster is locked: before that there is no run).
    fn apply_challenge_up(&mut self, slot_idx: usize, up: Option<&ChallengeUp>) {
        let Some(up) = up else {
            return;
        };
        if !self.campaign_started {
            return;
        }
        let Some(trainer_id) = self.trainer_at(slot_idx) else {
            return;
        };
        let changed = self
            .roster
            .iter_mut()
            .find(|r| r.trainer_id == trainer_id)
            .is_some_and(|r| r.challenge.merge(up));
        if changed {
            self.check_run_lost();
            self.save_campaign_to_disk();
        }
    }

    pub fn rules(&self) -> u16 {
        self.rules
    }

    pub fn is_run_lost(&self) -> bool {
        self.run_lost
    }

    /// Milestone 5 (Q7, Q8): a Nuzlocke run is lost once every roster
    /// trainer is a spirit, offline ones by their last report. Never undone.
    fn check_run_lost(&mut self) {
        if self.run_lost
            || !self.campaign_started
            || self.rules & CHALLENGE_RULE_NUZLOCKE == 0
            || self.roster.is_empty()
            || !self.roster.iter().all(|r| r.challenge.spirit)
        {
            return;
        }
        self.run_lost = true;
        warn!(
            "[Room {} | CHALLENGE] every trainer is a spirit: the run is lost",
            room_code_to_string(&self.room_code)
        );
    }

    /// A spirit (Ghost Mode): from the roster's last challenge report, or the
    /// live player state's flag.
    fn is_spirit(&self, slot: usize) -> bool {
        let Some(p) = self.slots[slot].as_ref() else {
            return false;
        };
        p.state.challenge_flags & CHALLENGE_FLAG_SPIRIT != 0
            || self
                .roster
                .iter()
                .any(|r| r.trainer_id == p.state.trainer_id && r.challenge.spirit)
    }

    /// The recipient's Monolocke type from the roster (MONO_TYPE_NONE before
    /// the lock, or without Monolocke).
    fn mono_type_for(&self, slot_idx: usize) -> u8 {
        self.roster
            .iter()
            .find(|r| r.slot_id as usize == slot_idx)
            .map_or(MONO_TYPE_NONE, |r| r.assigned_monolocke)
    }

    /// Milestone 5 soul-link (docs/MILESTONE5_PLAN.md §4.4, §5.6): the failed
    /// locations of the recipient's link group. Room-wide: every roster
    /// trainer's (offline ones by their last report). Duo: the recipient and
    /// its teammate, each without what failed before they paired; a player
    /// without a team is unlinked.
    fn link_union_for(&self, slot_idx: usize) -> [u8; CHALLENGE_LOCATION_BYTES] {
        let mut union = [0u8; CHALLENGE_LOCATION_BYTES];
        match soul_link_mode(self.rules) {
            SOUL_LINK_ROOM_WIDE => {
                for r in &self.roster {
                    for (u, f) in union.iter_mut().zip(r.challenge.failed.iter()) {
                        *u |= f;
                    }
                }
            }
            SOUL_LINK_DUO => {
                let Some(me) = self.trainer_at(slot_idx) else {
                    return union;
                };
                let Some(partner) = self.team_partner_of(me) else {
                    return union;
                };
                for r in self
                    .roster
                    .iter()
                    .filter(|r| r.trainer_id == me || r.trainer_id == partner)
                {
                    let c = &r.challenge;
                    for i in 0..CHALLENGE_LOCATION_BYTES {
                        union[i] |= c.failed[i] & !c.pair_baseline[i];
                    }
                }
            }
            _ => {}
        }
        union
    }

    fn challenge_down_for(&self, slot_idx: usize) -> ChallengeDown {
        let own = self
            .trainer_at(slot_idx)
            .and_then(|t| self.roster.iter().find(|r| r.trainer_id == t))
            .map(|r| r.challenge.clone())
            .unwrap_or_default();
        ChallengeDown {
            mono_type: self.mono_type_for(slot_idx),
            room_state: self.room_state(),
            rules: self.rules,
            team_deaths: self
                .roster
                .iter()
                .fold(0u16, |n, r| n.saturating_add(r.challenge.deaths)),
            link_union: self.link_union_for(slot_idx),
            spent: own.spent,
            failed: own.failed,
        }
    }

    fn room_state(&self) -> u8 {
        if self.run_lost {
            ROOM_STATE_RUN_LOST
        } else {
            0
        }
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

        // A lost run takes no joins; players still connected see the end.
        if self.run_lost
            && !self
                .slots
                .iter()
                .any(|s| s.as_ref().is_some_and(|p| p.addr == addr))
        {
            warn!("[Room {room_str}] Rejected {addr}: this run is lost");
            return Err(ROOM_STATUS_ERR_RUN_LOST);
        }

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

        // Monolocke: one type per player, without replacement (18 types,
        // at most 6 players), so no two players share one.
        let mono_types = if self.rules & CHALLENGE_RULE_MONOLOCKE != 0 {
            shuffled_mono_types(self.room_uid)
        } else {
            [MONO_TYPE_NONE; MONO_TYPE_COUNT as usize]
        };

        for (i, p) in active.iter().enumerate() {
            self.roster.push(RegisteredTrainer {
                slot_id: p.slot_id,
                trainer_id: p.state.trainer_id,
                last_badge_mask: p.badge_mask,
                assigned_monolocke: mono_types[i],
                player_secret: p.player_secret,
                allow_ahead: p.allow_ahead,
                challenge: TrainerChallenge::default(),
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
        // A link battle waits for a quiet player up to the referee's deadline, and a
        // called-off one may still come back for its BYEs: keep its slot meanwhile.
        let timeouts: [Duration; MAX_PLAYERS] = std::array::from_fn(|i| {
            if self.link_holds_slot(i) {
                self.client_timeout.max(LINK_DEADLINE * 2)
            } else {
                self.client_timeout
            }
        });
        for (idx, slot) in self.slots.iter_mut().enumerate() {
            if let Some(player) = slot
                && now.duration_since(player.last_seen) > timeouts[idx]
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

    pub fn build_broadcast_batch(&mut self) -> Vec<(SocketAddr, Vec<u8>)> {
        self.server_tick = self.server_tick.wrapping_add(1);
        let now = Instant::now();
        self.expire_sessions(now);
        self.expire_trades(now);
        self.expire_links(now);
        self.build_packets(now, false)
    }

    /// Link messages without waiting for the next tick (link latency,
    /// rebalance plan §25): the usual packet, now, for each member with a
    /// link message never sent. The tick still carries acks and resends.
    pub fn build_link_flush(&mut self) -> Vec<(SocketAddr, Vec<u8>)> {
        self.build_packets(Instant::now(), true)
    }

    fn build_packets(&mut self, now: Instant, link_flush: bool) -> Vec<(SocketAddr, Vec<u8>)> {
        let links: Vec<Option<LinkDown>> = (0..MAX_PLAYERS)
            .map(|i| {
                if link_flush && !self.link_has_unsent(i) {
                    None
                } else {
                    self.link_down_for(i, now)
                }
            })
            .collect();

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
            if link_flush && links[recipient_idx].is_none() {
                continue;
            }

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

            let trade = self.trade_in_for(recipient_idx);
            // A trade stream's player gets the trade on it, when it changes, not in packets.
            let mut packet_trade = trade;
            if let Some(ts) = self.trade_streams[recipient_idx]
                .as_mut()
                .filter(|ts| ts.addr == recipient.addr)
            {
                if ts.sent != Some(trade) {
                    ts.sent = Some(trade);
                    self.trade_stream_out.push(TradeStreamOut {
                        addr: recipient.addr,
                        frame: encode_trade_down(trade.as_ref()),
                    });
                }
                packet_trade = None;
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
                assigned_monolocke: self.mono_type_for(recipient_idx),
                challenge_room_state: self.room_state(),
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
                trade_pending: u8::from(trade.is_some()),
            };

            batch.push((
                recipient.addr,
                pkt.encode_full(
                    packet_trade.as_ref(),
                    links.get(recipient_idx).and_then(Option::as_ref),
                    Some(&self.challenge_down_for(recipient_idx)),
                ),
            ));
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
            // A spirit never holds the others back (docs/MILESTONE5_PLAN.md §4.4).
            .filter(|reg| !reg.challenge.spirit)
            .all(|reg| match &self.slots[reg.slot_id as usize] {
                Some(p) => {
                    p.badge_mask.count_ones() >= mine
                        || p.allow_ahead
                        || p.state.challenge_flags & CHALLENGE_FLAG_SPIRIT != 0
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

    /// Busy for a new request: in a pending request, an open trade or a
    /// trade still to finish, linked (a co-op battle), waiting at a team boss
    /// for the teammate, or in a battle of their own.
    fn in_session(&self, slot: usize) -> bool {
        self.sessions.iter().any(|s| s.members().contains(&slot))
            || self.trade_busy(slot)
            || self.link_busy(slot)
            || self.slots[slot]
                .as_ref()
                .is_some_and(|p| p.state.boss_wait != 0 || p.state.activity == ACTIVITY_BATTLE)
    }

    fn slot_of_trainer(&self, trainer: u32) -> Option<usize> {
        (0..MAX_PLAYERS).find(|&j| trainer != 0 && self.trainer_at(j) == Some(trainer))
    }

    /// Tells every player of a session how it ended: the requester sees the
    /// target (or `about`, e.g. who declined), everyone else the requester.
    fn end_session(&mut self, s: &Session, state: u8, about: Option<usize>) {
        self.set_view(
            s.from,
            state,
            s.kind,
            about.or(Some(s.to)),
            SESSION_REASON_NONE,
        );
        for m in s.members().into_iter().skip(1) {
            self.set_view(m, state, s.kind, Some(s.from), SESSION_REASON_NONE);
        }
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
                if kind == SESSION_KIND_LEAVE_TEAM {
                    return self.leave_team(slot);
                }
                if kind == SESSION_KIND_REVIVE {
                    return self.request_revive(slot, target, now);
                }
                // A spirit (Ghost Mode) only takes a revive.
                if self.is_spirit(slot) {
                    return refuse(self, SESSION_REASON_INVALID);
                }
                if target.is_some_and(|t| self.is_spirit(t)) {
                    return refuse(self, SESSION_REASON_BUSY);
                }
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
                // Two teams: a 2v2 that both teammates are asked about too.
                let team = if kind == SESSION_KIND_BATTLE && self.is_teamed(slot) {
                    let me = self.trainer_at(slot).unwrap_or(0);
                    let them = self.trainer_at(target).unwrap_or(0);
                    let (Some(from_mate), Some(to_mate)) = (
                        self.team_partner_of(me)
                            .and_then(|t| self.slot_of_trainer(t)),
                        self.team_partner_of(them)
                            .and_then(|t| self.slot_of_trainer(t)),
                    ) else {
                        return refuse(self, SESSION_REASON_TEAMMATE_AWAY);
                    };
                    if self.in_session(from_mate) || self.in_session(to_mate) {
                        return refuse(self, SESSION_REASON_BUSY);
                    }
                    Some(TeamBattle {
                        from_mate,
                        to_mate,
                        pending: vec![target, from_mate, to_mate],
                    })
                } else {
                    None
                };
                let kind = if team.is_some() {
                    SESSION_KIND_TEAM_BATTLE
                } else {
                    kind
                };
                info!("[Room {room_str} | SESSION] P{slot} asks P{target} (kind {kind})");
                if let Some(t) = &team {
                    // The requester's teammate is told whom they would battle.
                    let opponent = remote_slot_of(t.from_mate, target);
                    self.set_view(
                        t.from_mate,
                        SESSION_STATE_INCOMING,
                        kind,
                        Some(slot),
                        opponent,
                    );
                    self.set_view(
                        t.to_mate,
                        SESSION_STATE_INCOMING,
                        kind,
                        Some(slot),
                        SESSION_REASON_NONE,
                    );
                }
                self.sessions.push(Session {
                    kind,
                    from: slot,
                    to: target,
                    created: now,
                    team,
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
                let Some(i) = self.sessions.iter().position(|s| s.answers(slot)) else {
                    return;
                };
                let accepted = out.op == SESSION_OP_ACCEPT;
                if let Some(t) = self.sessions[i].team.as_mut() {
                    // A team battle starts once all three have said yes; one
                    // no ends it for everyone.
                    if accepted {
                        t.pending.retain(|&p| p != slot);
                        info!("[Room {room_str} | SESSION] P{slot} accepted a team battle");
                        if !t.pending.is_empty() {
                            return;
                        }
                    }
                    let session = self.sessions.remove(i);
                    let state = if accepted {
                        SESSION_STATE_ACCEPTED
                    } else {
                        SESSION_STATE_DECLINED
                    };
                    info!(
                        "[Room {room_str} | SESSION] team battle of P{} {}",
                        session.from,
                        if accepted { "starts" } else { "declined" }
                    );
                    let about = if accepted { None } else { Some(slot) };
                    return self.end_session(&session, state, about);
                }
                let session = self.sessions.remove(i);
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
                if accepted && session.kind == SESSION_KIND_TRADE {
                    self.start_trade(session.from, session.to, false, now);
                }
                if accepted
                    && session.kind == SESSION_KIND_TEAM_UP
                    && let (Some(a), Some(b)) =
                        (self.trainer_at(session.from), self.trainer_at(session.to))
                    && self.team_partner_of(a).is_none()
                    && self.team_partner_of(b).is_none()
                {
                    self.teams.push([a, b]);
                    // Duo Soul-Link starts here: what failed before stays apart.
                    for r in self
                        .roster
                        .iter_mut()
                        .filter(|r| r.trainer_id == a || r.trainer_id == b)
                    {
                        r.challenge.pair_baseline = r.challenge.failed;
                    }
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
                self.end_session(&session, SESSION_STATE_CANCELLED, None);
            }
            _ => {}
        }
    }

    /// Milestone 5 Ghost Mode (docs/MILESTONE5_PLAN.md §5.5.4): `slot` gives a
    /// Pokemon to the spirit `target`. The spirit doesn't answer: a one-sided
    /// trade opens at once and both games see the request ACCEPTED.
    fn request_revive(&mut self, slot: usize, target: Option<usize>, now: Instant) {
        let kind = SESSION_KIND_REVIVE;
        let Some(target) = target else {
            return self.set_view(
                slot,
                SESSION_STATE_REFUSED,
                kind,
                None,
                SESSION_REASON_NOT_HERE,
            );
        };
        if self.rules & CHALLENGE_RULE_NUZLOCKE == 0
            || self.run_lost
            || self.is_spirit(slot)
            || !self.is_spirit(target)
        {
            return self.set_view(
                slot,
                SESSION_STATE_REFUSED,
                kind,
                Some(target),
                SESSION_REASON_INVALID,
            );
        }
        if self.in_session(slot) || self.in_session(target) {
            return self.set_view(
                slot,
                SESSION_STATE_REFUSED,
                kind,
                Some(target),
                SESSION_REASON_BUSY,
            );
        }
        info!(
            "[Room {} | SESSION] P{slot} revives P{target}",
            room_code_to_string(&self.room_code)
        );
        self.start_trade(slot, target, true, now);
        self.set_view(
            slot,
            SESSION_STATE_ACCEPTED,
            kind,
            Some(target),
            SESSION_REASON_NONE,
        );
        self.set_view(
            target,
            SESSION_STATE_ACCEPTED,
            kind,
            Some(slot),
            SESSION_REASON_NONE,
        );
    }

    /// Leaves the team `slot` is in: no answer needed. The team is dropped
    /// and saved at once; the ex-teammate, when here and not in a session of
    /// their own, gets a notice.
    fn leave_team(&mut self, slot: usize) {
        let room_str = room_code_to_string(&self.room_code);
        let kind = SESSION_KIND_LEAVE_TEAM;
        let me = self.trainer_at(slot).unwrap_or(0);
        let Some(partner) = self.team_partner_of(me).filter(|_| me != 0) else {
            return self.set_view(
                slot,
                SESSION_STATE_REFUSED,
                kind,
                None,
                SESSION_REASON_INVALID,
            );
        };
        let partner_slot =
            (0..MAX_PLAYERS).find(|&j| j != slot && self.trainer_at(j) == Some(partner));
        if self.in_session(slot) {
            return self.set_view(
                slot,
                SESSION_STATE_REFUSED,
                kind,
                partner_slot,
                SESSION_REASON_BUSY,
            );
        }
        // Duo Soul-Link: a team is for the whole run (plan §5.6).
        if soul_link_mode(self.rules) == SOUL_LINK_DUO {
            return self.set_view(
                slot,
                SESSION_STATE_REFUSED,
                kind,
                partner_slot,
                SESSION_REASON_TEAM_RULE,
            );
        }
        self.teams.retain(|t| !t.contains(&me));
        info!("[Room {room_str} | TEAM] 0x{me:08X} left the team with 0x{partner:08X}");
        self.save_campaign_to_disk();
        self.set_view(
            slot,
            SESSION_STATE_ACCEPTED,
            kind,
            partner_slot,
            SESSION_REASON_NONE,
        );
        if let Some(p) = partner_slot
            && !self.in_session(p)
        {
            self.set_view(
                p,
                SESSION_STATE_NOTICE,
                kind,
                Some(slot),
                SESSION_REASON_NONE,
            );
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
            self.end_session(&s, SESSION_STATE_TIMEOUT, None);
        }
    }

    /// A side went offline: the other side's request (or open trade) is cancelled.
    fn drop_sessions_of_missing_players(&mut self) {
        self.abort_trades_of_missing_players();
        self.drop_links_of_missing_players();
        let (gone, kept): (Vec<Session>, Vec<Session>) = self
            .sessions
            .drain(..)
            .partition(|s| s.members().iter().any(|&m| self.slots[m].is_none()));
        self.sessions = kept;
        for s in gone {
            self.end_session(&s, SESSION_STATE_CANCELLED, None);
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
    /// FALSE when a save directory is configured and the write failed.
    fn save_campaign_to_disk(&self) -> bool {
        let Some(path) = self.save_file_path() else {
            return true;
        };
        if let Some(parent) = path.parent()
            && let Err(err) = fs::create_dir_all(parent)
        {
            error!(
                "[Room {}] Cannot create save dir {parent:?}: {err}",
                room_code_to_string(&self.room_code)
            );
            return false;
        }
        let payload = CampaignSaveFile {
            room_code: room_code_to_string(&self.room_code),
            room_uid: self.room_uid,
            host_token: self.host_token,
            campaign_started: self.campaign_started,
            expected_party_size: self.roster.len() as u8,
            roster: self.roster.clone(),
            teams: self.teams.clone(),
            pending_trades: self.pending_trades.clone(),
            rules: self.rules,
            run_lost: self.run_lost,
        };
        let json = match serde_json::to_string_pretty(&payload) {
            Ok(json) => json,
            Err(err) => {
                error!(
                    "[Room {}] Cannot serialize campaign: {err}",
                    payload.room_code
                );
                return false;
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
            return false;
        }
        true
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
        self.pending_trades = saved.pending_trades;
        self.rules = sanitize_challenge_rules(saved.rules);
        self.run_lost = saved.run_lost;
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
pub(crate) fn remote_slot_of(recipient: usize, other: usize) -> u8 {
    if other < recipient {
        other as u8
    } else {
        (other - 1) as u8
    }
}

/// A random, non-zero room identity. It only has to differ between rooms
/// that reuse a code (the per-room save files are keyed by it), not resist
/// guessing, so a per-process random hasher over the code and time is enough.
pub(crate) fn new_room_uid(room_code: [u8; 6]) -> u32 {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let uid = RandomState::new().hash_one((room_code, nanos)) as u32;
    uid.max(1)
}

/// The 18 Monolocke types in a random order (seeded like the room UID).
pub(crate) fn shuffled_mono_types(room_uid: u32) -> [u8; MONO_TYPE_COUNT as usize] {
    let mut types: [u8; MONO_TYPE_COUNT as usize] = core::array::from_fn(|i| i as u8);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let state = RandomState::new();
    for i in (1..types.len()).rev() {
        let j = (state.hash_one((room_uid, nanos, i)) % (i as u64 + 1)) as usize;
        types.swap(i, j);
    }
    types
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
    /// Packets to send right away (link messages, `take_immediate`).
    immediate: Vec<(SocketAddr, Vec<u8>)>,
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
            immediate: Vec::new(),
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
        let link_code = pkt
            .link
            .as_ref()
            .is_some_and(|l| !l.msgs.is_empty())
            .then_some(pkt.room_code);
        let result = self.dispatch(addr, pkt).map_err(|status| {
            self.failures.record_failure(addr.ip(), now);
            Rejection::Status(status)
        });
        // Link messages go on at once instead of on the next tick.
        if result.is_ok()
            && let Some(code) = link_code
            && let Some(room) = self.rooms.get_mut(&code)
        {
            let packets = room.build_link_flush();
            self.immediate.extend(packets);
        }
        result
    }

    /// Packets built outside the tick (link messages), to send now.
    pub fn take_immediate(&mut self) -> Vec<(SocketAddr, Vec<u8>)> {
        std::mem::take(&mut self.immediate)
    }

    /// A QUIC player's link stream did something: to its room. Messages for UDP members go
    /// out at once (`take_immediate`), for QUIC members' streams by `take_link_stream_out`.
    pub fn link_stream_event(&mut self, addr: SocketAddr, event: LinkStreamEvent) {
        let msg = matches!(event, LinkStreamEvent::Msg(..));
        let Some(room) = self
            .rooms
            .values_mut()
            .find(|r| r.slot_of_addr(addr).is_some())
        else {
            return;
        };
        room.link_stream_event(addr, event);
        if msg {
            let packets = room.build_link_flush();
            self.immediate.extend(packets);
        }
    }

    /// A QUIC player's trade stream did something: to its room.
    pub fn trade_stream_event(&mut self, addr: SocketAddr, event: TradeStreamEvent) {
        if let Some(room) = self
            .rooms
            .values_mut()
            .find(|r| r.slot_of_addr(addr).is_some())
        {
            room.trade_stream_event(addr, event);
        }
    }

    /// Trade frames for QUIC players' trade streams, from every room.
    pub fn take_trade_stream_out(&mut self) -> Vec<TradeStreamOut> {
        self.rooms
            .values_mut()
            .flat_map(|r| r.take_trade_stream_out())
            .collect()
    }

    /// Link messages for QUIC players' link streams, from every room.
    pub fn take_link_stream_out(&mut self) -> Vec<LinkStreamOut> {
        self.rooms
            .values_mut()
            .flat_map(|r| r.take_link_stream_out())
            .collect()
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

    pub fn tick_all_rooms(&mut self) -> Vec<(SocketAddr, Vec<u8>)> {
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
