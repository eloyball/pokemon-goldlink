use std::fmt;

pub const COOP_MAGIC: u32 = 0x434F_4F50; // 'COOP'
pub const PROTOCOL_VERSION: u16 = 5;
pub const MAX_PLAYERS: usize = 6;
pub const REMOTE_SLOTS: usize = MAX_PLAYERS - 1;
pub const AUTO_PLAYER_ID: u8 = 0xFF;

// Client Room Actions (Byte 12 of ClientToServerPacket)
pub const ROOM_ACTION_CREATE: u8 = 1;
pub const ROOM_ACTION_JOIN: u8 = 2;
pub const ROOM_ACTION_SYNC: u8 = 3;
pub const ROOM_ACTION_LEAVE: u8 = 4;

// Server Status & Error Codes (Byte 12 of ServerToClientPacket)
pub const ROOM_STATUS_OK: u8 = 0;
pub const ROOM_STATUS_ERR_NOT_FOUND: u8 = 1;
pub const ROOM_STATUS_ERR_ALREADY_EXISTS: u8 = 2;
pub const ROOM_STATUS_ERR_LOCKED: u8 = 3;
pub const ROOM_STATUS_ERR_FULL: u8 = 4;
pub const ROOM_STATUS_ERR_INVALID_CODE: u8 = 5;
/// A room that has not started yet only takes fresh saves: the ROM reported
/// `save_status == SAVE_STATUS_IN_PROGRESS` while in the overworld.
pub const ROOM_STATUS_ERR_MIDGAME_SAVE: u8 = 6;
/// Not an error: echo `cookie` (bytes 16..24 of the reply) and retry.
pub const ROOM_STATUS_CHALLENGE: u8 = 7;
/// Too many rejected attempts from this IP; back off for a few seconds.
pub const ROOM_STATUS_ERR_RATE_LIMITED: u8 = 8;
/// The save's trainer ID is registered in another locked campaign on this relay.
pub const ROOM_STATUS_ERR_SAVE_OTHER_ROOM: u8 = 9;

// PlayerState.save_status, computed by the ROM (the relay never inspects maps).
// Player-to-player sessions (docs/MILESTONE3_PLAN.md, Phase 1). The client
// latches its last operation (repeated every packet under one sequence
// number); the relay applies it once and reports the outcome per recipient
// in the gates block, again latched under a sequence number.
pub const SESSION_OP_NONE: u8 = 0;
pub const SESSION_OP_REQUEST: u8 = 1;
pub const SESSION_OP_ACCEPT: u8 = 2;
pub const SESSION_OP_DECLINE: u8 = 3;
pub const SESSION_OP_CANCEL: u8 = 4;

pub const SESSION_KIND_TRADE: u8 = 1;
pub const SESSION_KIND_BATTLE: u8 = 2;
pub const SESSION_KIND_TEAM_UP: u8 = 3;

pub const SESSION_STATE_NONE: u8 = 0;
pub const SESSION_STATE_OUTGOING: u8 = 1; // my request, waiting for the answer
pub const SESSION_STATE_INCOMING: u8 = 2; // someone asks me
pub const SESSION_STATE_ACCEPTED: u8 = 3;
pub const SESSION_STATE_DECLINED: u8 = 4;
pub const SESSION_STATE_CANCELLED: u8 = 5; // withdrawn, or the other side left
pub const SESSION_STATE_TIMEOUT: u8 = 6;
pub const SESSION_STATE_REFUSED: u8 = 7; // the relay turned the request down (reason)

pub const SESSION_REASON_NONE: u8 = 0;
pub const SESSION_REASON_NOT_HERE: u8 = 1; // target not connected
pub const SESSION_REASON_BUSY: u8 = 2; // either side already in a session
pub const SESSION_REASON_TEAM_RULE: u8 = 3; // a team cannot battle a solo player or its own teammate
pub const SESSION_REASON_ALREADY_TEAMED: u8 = 4;
pub const SESSION_REASON_INVALID: u8 = 5; // unknown kind, or a request to oneself

pub const NO_SLOT: u8 = 0xFF;

// Trading (docs/MILESTONE3_PLAN.md, Phase 2). While the ROM has a trade open,
// the client packet carries a trailer (its CoopTradeOut, bytes 120..364);
// while the relay holds a trade for a recipient, the server packet carries
// one (its CoopTradeIn, bytes 360..612) and gates.trade_pending is 1. Layouts:
// hg-engine/include/coop_sync.h.
pub const TRADE_MON_SIZE: usize = 0xEC;
pub const TRADE_OUT_SIZE: usize = 8 + TRADE_MON_SIZE; // 244
pub const TRADE_IN_SIZE: usize = 16 + TRADE_MON_SIZE; // 252

pub const TRADE_OUT_NONE: u8 = 0;
pub const TRADE_OUT_PICKING: u8 = 1;
pub const TRADE_OUT_OFFERED: u8 = 2;
pub const TRADE_OUT_CONFIRMED: u8 = 3;
pub const TRADE_OUT_CANCEL: u8 = 4;
pub const TRADE_OUT_DONE: u8 = 5;

pub const TRADE_IN_WAITING: u8 = 1;
pub const TRADE_IN_OFFERED: u8 = 2;
pub const TRADE_IN_COMMIT: u8 = 3;
pub const TRADE_IN_ABORTED: u8 = 4;

pub const TRADE_ABORT_CANCELLED: u8 = 1;
pub const TRADE_ABORT_LEFT: u8 = 2;
pub const TRADE_ABORT_TIMEOUT: u8 = 3;

/// The ROM's side of an open trade (client trailer).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TradeOut {
    pub state: u8,
    pub offer_id: u8,
    pub confirm_id: u8,
    pub trade_id: u32,
    pub mon: [u8; TRADE_MON_SIZE],
}

impl TradeOut {
    pub fn encode_le(&self, out: &mut [u8]) {
        out[0] = self.state;
        out[1] = self.offer_id;
        out[2] = self.confirm_id;
        out[3] = 0;
        out[4..8].copy_from_slice(&self.trade_id.to_le_bytes());
        out[8..8 + TRADE_MON_SIZE].copy_from_slice(&self.mon);
    }

    pub fn decode_le(buf: &[u8]) -> Self {
        let mut mon = [0u8; TRADE_MON_SIZE];
        mon.copy_from_slice(&buf[8..8 + TRADE_MON_SIZE]);
        Self {
            state: buf[0],
            offer_id: buf[1],
            confirm_id: buf[2],
            trade_id: u32::from_le_bytes([buf[4], buf[5], buf[6], buf[7]]),
            mon,
        }
    }
}

/// The relay's view of a recipient's trade (server trailer).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TradeIn {
    pub state: u8,
    pub reason: u8,
    pub partner_offer_id: u8,
    pub my_offer_id: u8,
    pub partner_confirmed: u8,
    pub peer_slot: u8,
    pub trade_id: u32,
    pub give_pid: u32,
    pub mon: [u8; TRADE_MON_SIZE],
}

impl TradeIn {
    pub fn empty(state: u8) -> Self {
        Self {
            state,
            reason: 0,
            partner_offer_id: 0,
            my_offer_id: 0,
            partner_confirmed: 0,
            peer_slot: NO_SLOT,
            trade_id: 0,
            give_pid: 0,
            mon: [0; TRADE_MON_SIZE],
        }
    }

    pub fn encode_le(&self, out: &mut [u8]) {
        out[0] = self.state;
        out[1] = self.reason;
        out[2] = self.partner_offer_id;
        out[3] = self.my_offer_id;
        out[4] = self.partner_confirmed;
        out[5] = self.peer_slot;
        out[6] = 0;
        out[7] = 0;
        out[8..12].copy_from_slice(&self.trade_id.to_le_bytes());
        out[12..16].copy_from_slice(&self.give_pid.to_le_bytes());
        out[16..16 + TRADE_MON_SIZE].copy_from_slice(&self.mon);
    }

    pub fn decode_le(buf: &[u8]) -> Self {
        let mut mon = [0u8; TRADE_MON_SIZE];
        mon.copy_from_slice(&buf[16..16 + TRADE_MON_SIZE]);
        Self {
            state: buf[0],
            reason: buf[1],
            partner_offer_id: buf[2],
            my_offer_id: buf[3],
            partner_confirmed: buf[4],
            peer_slot: buf[5],
            trade_id: u32::from_le_bytes([buf[8], buf[9], buf[10], buf[11]]),
            give_pid: u32::from_le_bytes([buf[12], buf[13], buf[14], buf[15]]),
            mon,
        }
    }
}

/// Personality value of an encrypted party Pokemon (stored in the clear at 0).
pub fn mon_pid(mon: &[u8; TRADE_MON_SIZE]) -> u32 {
    u32::from_le_bytes([mon[0], mon[1], mon[2], mon[3]])
}
/// `teamed_mask` bit for "the recipient is in a team" (bits 0..4: remote slots).
pub const TEAMED_SELF_BIT: u8 = 0x80;

pub const SAVE_STATUS_UNKNOWN: u8 = 0; // not in the overworld yet
pub const SAVE_STATUS_FRESH: u8 = 1; // in the bedroom with an empty party
pub const SAVE_STATUS_IN_PROGRESS: u8 = 2;

// Wire Payload Sizes (in bytes)
pub const PLAYER_STATE_SIZE: usize = 64;
pub const GATES_SIZE: usize = 24;
pub const SESSION_BLOCK_SIZE: usize = 8;
pub const CLIENT_PACKET_SIZE: usize = 32 + PLAYER_STATE_SIZE + 16 + SESSION_BLOCK_SIZE; // 120 bytes
pub const SERVER_PACKET_SIZE: usize = 16 + (REMOTE_SLOTS * PLAYER_STATE_SIZE) + GATES_SIZE; // 360 bytes
/// Error replies carry only the 16-byte header, so a rejected request never
/// produces a reply larger than the 64-byte request (no UDP amplification).
pub const ERROR_PACKET_SIZE: usize = 16;
/// Challenge replies: the 16-byte header plus the 8-byte cookie.
pub const CHALLENGE_PACKET_SIZE: usize = 24;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProtocolError {
    PacketTooShort { expected: usize, actual: usize },
    InvalidMagic(u32),
    UnsupportedVersion(u16),
}

impl fmt::Display for ProtocolError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::PacketTooShort { expected, actual } => {
                write!(
                    f,
                    "packet too short: expected {expected} bytes, got {actual}"
                )
            }
            Self::InvalidMagic(m) => {
                write!(f, "invalid magic: 0x{m:08X} (expected 0x{COOP_MAGIC:08X})")
            }
            Self::UnsupportedVersion(v) => write!(f, "unsupported protocol version: {v}"),
        }
    }
}

impl std::error::Error for ProtocolError {}

/// Strictly validates that a 6-byte room code consists of 6 ASCII alphanumeric characters (A-Z, 0-9).
pub fn validate_room_code(raw: [u8; 6]) -> Result<[u8; 6], u8> {
    let mut out = [0u8; 6];
    for (i, b) in raw.into_iter().enumerate() {
        let c = (b as char).to_ascii_uppercase();
        if c.is_ascii_uppercase() || c.is_ascii_digit() {
            out[i] = c as u8;
        } else {
            return Err(ROOM_STATUS_ERR_INVALID_CODE);
        }
    }
    Ok(out)
}

pub fn room_code_to_string(code: &[u8; 6]) -> String {
    String::from_utf8_lossy(code).to_string()
}

/// Matches `CoopPlayerState` (64 bytes) in `hg-engine/include/coop_sync.h`.
/// The relay reads only a few fields (identity, readiness, save status) and
/// forwards the rest untouched.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PlayerState {
    pub map_id: u16,              // 0x00
    pub matrix_id: u8,            // 0x02
    pub avatar_state: u8,         // 0x03
    pub x: i16,                   // 0x04
    pub z: i16,                   // 0x06
    pub height: i16,              // 0x08
    pub direction: u8,            // 0x0A
    pub gender: u8,               // 0x0B
    pub sprite_id: u16,           // 0x0C
    pub move_action: u8,          // 0x0E
    pub move_seq: u8,             // 0x0F
    pub is_active: u8,            // 0x10
    pub client_ready: u8,         // 0x11 (computed by the ROM)
    pub save_status: u8,          // 0x12 SAVE_STATUS_* (computed by the ROM)
    pub activity: u8,             // 0x13
    pub trainer_id: u32,          // 0x14
    pub name: [u16; 8],           // 0x18
    pub follower_species: u16,    // 0x28
    pub follower_sprite_id: u16,  // 0x2A
    pub follower_form: u8,        // 0x2C
    pub follower_shiny: u8,       // 0x2D
    pub follower_gender: u8,      // 0x2E
    pub follower_visible: u8,     // 0x2F
    pub follower_x: i16,          // 0x30
    pub follower_z: i16,          // 0x32
    pub follower_height: i16,     // 0x34
    pub follower_direction: u8,   // 0x36
    pub follower_move_action: u8, // 0x37
    pub follower_move_seq: u8,    // 0x38
    pub pad39: [u8; 3],           // 0x39
    pub avatar_id: u8,            // 0x3C
    pub anim: u8,                 // 0x3D pose animation index (forwarded untouched)
    pub field_busy: u8,           // 0x3E (a field task or message is running; forwarded as is)
    pub pad3f: u8,                // 0x3F
}

fn put16(out: &mut [u8], at: usize, v: [u8; 2]) {
    out[at..at + 2].copy_from_slice(&v);
}

fn get_u16(buf: &[u8], at: usize) -> u16 {
    u16::from_le_bytes([buf[at], buf[at + 1]])
}

fn get_i16(buf: &[u8], at: usize) -> i16 {
    i16::from_le_bytes([buf[at], buf[at + 1]])
}

impl PlayerState {
    pub fn encode_le(&self, out: &mut [u8]) {
        debug_assert!(out.len() >= PLAYER_STATE_SIZE);
        put16(out, 0x00, self.map_id.to_le_bytes());
        out[0x02] = self.matrix_id;
        out[0x03] = self.avatar_state;
        put16(out, 0x04, self.x.to_le_bytes());
        put16(out, 0x06, self.z.to_le_bytes());
        put16(out, 0x08, self.height.to_le_bytes());
        out[0x0A] = self.direction;
        out[0x0B] = self.gender;
        put16(out, 0x0C, self.sprite_id.to_le_bytes());
        out[0x0E] = self.move_action;
        out[0x0F] = self.move_seq;
        out[0x10] = self.is_active;
        out[0x11] = self.client_ready;
        out[0x12] = self.save_status;
        out[0x13] = self.activity;
        out[0x14..0x18].copy_from_slice(&self.trainer_id.to_le_bytes());
        for (i, ch) in self.name.iter().enumerate() {
            put16(out, 0x18 + i * 2, ch.to_le_bytes());
        }
        put16(out, 0x28, self.follower_species.to_le_bytes());
        put16(out, 0x2A, self.follower_sprite_id.to_le_bytes());
        out[0x2C] = self.follower_form;
        out[0x2D] = self.follower_shiny;
        out[0x2E] = self.follower_gender;
        out[0x2F] = self.follower_visible;
        put16(out, 0x30, self.follower_x.to_le_bytes());
        put16(out, 0x32, self.follower_z.to_le_bytes());
        put16(out, 0x34, self.follower_height.to_le_bytes());
        out[0x36] = self.follower_direction;
        out[0x37] = self.follower_move_action;
        out[0x38] = self.follower_move_seq;
        out[0x39..0x3C].copy_from_slice(&self.pad39);
        out[0x3C] = self.avatar_id;
        out[0x3D] = self.anim;
        out[0x3E] = self.field_busy;
        out[0x3F] = self.pad3f;
    }

    pub fn decode_le(buf: &[u8]) -> Result<Self, ProtocolError> {
        if buf.len() < PLAYER_STATE_SIZE {
            return Err(ProtocolError::PacketTooShort {
                expected: PLAYER_STATE_SIZE,
                actual: buf.len(),
            });
        }
        let mut name = [0u16; 8];
        for (i, ch) in name.iter_mut().enumerate() {
            *ch = get_u16(buf, 0x18 + i * 2);
        }
        Ok(Self {
            map_id: get_u16(buf, 0x00),
            matrix_id: buf[0x02],
            avatar_state: buf[0x03],
            x: get_i16(buf, 0x04),
            z: get_i16(buf, 0x06),
            height: get_i16(buf, 0x08),
            direction: buf[0x0A],
            gender: buf[0x0B],
            sprite_id: get_u16(buf, 0x0C),
            move_action: buf[0x0E],
            move_seq: buf[0x0F],
            is_active: buf[0x10],
            client_ready: buf[0x11],
            save_status: buf[0x12],
            activity: buf[0x13],
            trainer_id: u32::from_le_bytes([buf[0x14], buf[0x15], buf[0x16], buf[0x17]]),
            name,
            follower_species: get_u16(buf, 0x28),
            follower_sprite_id: get_u16(buf, 0x2A),
            follower_form: buf[0x2C],
            follower_shiny: buf[0x2D],
            follower_gender: buf[0x2E],
            follower_visible: buf[0x2F],
            follower_x: get_i16(buf, 0x30),
            follower_z: get_i16(buf, 0x32),
            follower_height: get_i16(buf, 0x34),
            follower_direction: buf[0x36],
            follower_move_action: buf[0x37],
            follower_move_seq: buf[0x38],
            pad39: [buf[0x39], buf[0x3A], buf[0x3B]],
            avatar_id: buf[0x3C],
            anim: buf[0x3D],
            field_busy: buf[0x3E],
            pad3f: buf[0x3F],
        })
    }
}

/// The client's latched session operation (bytes 112..120 of its packet).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct SessionOut {
    pub seq: u8,
    pub op: u8,
    pub kind: u8,
    pub target_trainer: u32,
}

/// Sent by BizHawk C# Plugin -> Rust Relay Server (120 bytes)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClientToServerPacket {
    pub magic: u32,                // 0..4
    pub version: u16,              // 4..6
    pub room_code: [u8; 6],        // 6..12
    pub room_action: u8,           // 12 (1 = Create, 2 = Join, 3 = Sync, 4 = Leave)
    pub requested_player_id: u8,   // 13 (0..5 or 0xFF)
    pub allow_ahead: u8,           // 14 (1 = this player lets teammates go ahead past the gym gate)
    pub target_party_size: u8,     // 15 (host's room size, 2..6; only honored from the host)
    pub host_token: u32,           // 16..20
    pub badge_mask_local: u16,     // 20..22
    pub sequence: u32,             // 22..26
    pub heartbeat_rom: u32,        // 26..30
    pub reserved: u16,             // 30..32
    pub local_player: PlayerState, // 32..96 (64 bytes)
    pub cookie: u64,               // 96..104 (0 until the relay issues one)
    pub player_secret: u64,        // 104..112 (random per bridge install, never 0)
    pub session: SessionOut,       // 112..120 (seq, op, kind, pad, u32 target trainer ID)
    pub trade: Option<TradeOut>,   // 120..364 while the ROM has a trade open
}

impl ClientToServerPacket {
    pub fn encode_le(&self) -> [u8; CLIENT_PACKET_SIZE] {
        let mut buf = [0u8; CLIENT_PACKET_SIZE];
        buf[0..4].copy_from_slice(&self.magic.to_le_bytes());
        buf[4..6].copy_from_slice(&self.version.to_le_bytes());
        buf[6..12].copy_from_slice(&self.room_code);
        buf[12] = self.room_action;
        buf[13] = self.requested_player_id;
        buf[14] = self.allow_ahead;
        buf[15] = self.target_party_size;
        buf[16..20].copy_from_slice(&self.host_token.to_le_bytes());
        buf[20..22].copy_from_slice(&self.badge_mask_local.to_le_bytes());
        buf[22..26].copy_from_slice(&self.sequence.to_le_bytes());
        buf[26..30].copy_from_slice(&self.heartbeat_rom.to_le_bytes());
        buf[30..32].copy_from_slice(&self.reserved.to_le_bytes());
        self.local_player.encode_le(&mut buf[32..96]);
        buf[96..104].copy_from_slice(&self.cookie.to_le_bytes());
        buf[104..112].copy_from_slice(&self.player_secret.to_le_bytes());
        buf[112] = self.session.seq;
        buf[113] = self.session.op;
        buf[114] = self.session.kind;
        buf[116..120].copy_from_slice(&self.session.target_trainer.to_le_bytes());
        buf
    }

    /// The fixed packet plus the trade trailer when there is one.
    pub fn encode_full(&self) -> Vec<u8> {
        let mut buf = self.encode_le().to_vec();
        if let Some(trade) = &self.trade {
            let mut trailer = [0u8; TRADE_OUT_SIZE];
            trade.encode_le(&mut trailer);
            buf.extend_from_slice(&trailer);
        }
        buf
    }

    pub fn decode_le(buf: &[u8]) -> Result<Self, ProtocolError> {
        if buf.len() < CLIENT_PACKET_SIZE {
            return Err(ProtocolError::PacketTooShort {
                expected: CLIENT_PACKET_SIZE,
                actual: buf.len(),
            });
        }
        let magic = u32::from_le_bytes([buf[0], buf[1], buf[2], buf[3]]);
        if magic != COOP_MAGIC {
            return Err(ProtocolError::InvalidMagic(magic));
        }
        let version = u16::from_le_bytes([buf[4], buf[5]]);
        if version != PROTOCOL_VERSION {
            return Err(ProtocolError::UnsupportedVersion(version));
        }

        let mut raw_room = [0u8; 6];
        raw_room.copy_from_slice(&buf[6..12]);

        let local_player = PlayerState::decode_le(&buf[32..96])?;

        Ok(Self {
            magic,
            version,
            room_code: raw_room,
            room_action: buf[12],
            requested_player_id: buf[13],
            allow_ahead: buf[14],
            target_party_size: buf[15],
            host_token: u32::from_le_bytes([buf[16], buf[17], buf[18], buf[19]]),
            badge_mask_local: u16::from_le_bytes([buf[20], buf[21]]),
            sequence: u32::from_le_bytes([buf[22], buf[23], buf[24], buf[25]]),
            heartbeat_rom: u32::from_le_bytes([buf[26], buf[27], buf[28], buf[29]]),
            reserved: u16::from_le_bytes([buf[30], buf[31]]),
            local_player,
            cookie: u64::from_le_bytes(buf[96..104].try_into().expect("8 bytes")),
            player_secret: u64::from_le_bytes(buf[104..112].try_into().expect("8 bytes")),
            session: SessionOut {
                seq: buf[112],
                op: buf[113],
                kind: buf[114],
                target_trainer: u32::from_le_bytes([buf[116], buf[117], buf[118], buf[119]]),
            },
            trade: (buf.len() >= CLIENT_PACKET_SIZE + TRADE_OUT_SIZE)
                .then(|| TradeOut::decode_le(&buf[CLIENT_PACKET_SIZE..])),
        })
    }
}

/// Sent by Rust Relay Server -> BizHawk C# Plugin (360 bytes). Bytes 16..360
/// are copied verbatim to mailbox 0x050..0x1A8 (remote players, then gates).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ServerToClientPacket {
    pub magic: u32,
    pub version: u16,
    pub assigned_player_id: u8,
    pub active_players_mask: u8,
    pub server_tick: u32,
    pub status_code: u8,          // Byte 12: ROOM_STATUS_*
    pub reserved_header: [u8; 3], // Bytes 13..16
    pub remote_players: [PlayerState; REMOTE_SLOTS],
    pub badge_mask_local_echo: u16,
    pub badge_mask_synced: u16,
    pub flag_coop_sync_ready: u8,
    pub current_checkpoint: u8,
    pub assigned_monolocke: u8,
    pub nuzlocke_failed: u8,
    pub flag_campaign_started: u8,
    pub expected_party_size: u8,
    pub online_party_count: u8,
    pub ready_party_count: u8,
    pub room_uid: u32,
    pub team_partner_slot: u8, // gates +16: my teammate's remote slot (NO_SLOT: none or offline)
    pub teamed_mask: u8,       // +17: bit per remote slot in a team; TEAMED_SELF_BIT = me
    pub session_state: u8,     // +18: SESSION_STATE_*
    pub session_kind: u8,      // +19
    pub session_peer_slot: u8, // +20: the other side's remote slot (NO_SLOT: gone)
    pub session_seq: u8,       // +21: bumped on every change of the four fields above
    pub session_reason: u8,    // +22: SESSION_REASON_* when refused
    pub trade_pending: u8,     // +23: 1 while the relay holds a trade for the recipient
}

impl ServerToClientPacket {
    pub fn encode_le(&self) -> [u8; SERVER_PACKET_SIZE] {
        let mut buf = [0u8; SERVER_PACKET_SIZE];
        buf[0..4].copy_from_slice(&self.magic.to_le_bytes());
        buf[4..6].copy_from_slice(&self.version.to_le_bytes());
        buf[6] = self.assigned_player_id;
        buf[7] = self.active_players_mask;
        buf[8..12].copy_from_slice(&self.server_tick.to_le_bytes());
        buf[12] = self.status_code;
        buf[13..16].copy_from_slice(&self.reserved_header);

        for (i, player) in self.remote_players.iter().enumerate() {
            let start = 16 + i * PLAYER_STATE_SIZE;
            player.encode_le(&mut buf[start..start + PLAYER_STATE_SIZE]);
        }

        let gates_base = 16 + REMOTE_SLOTS * PLAYER_STATE_SIZE;
        buf[gates_base..gates_base + 2].copy_from_slice(&self.badge_mask_local_echo.to_le_bytes());
        buf[gates_base + 2..gates_base + 4].copy_from_slice(&self.badge_mask_synced.to_le_bytes());
        buf[gates_base + 4] = self.flag_coop_sync_ready;
        buf[gates_base + 5] = self.current_checkpoint;
        buf[gates_base + 6] = self.assigned_monolocke;
        buf[gates_base + 7] = self.nuzlocke_failed;
        buf[gates_base + 8] = self.flag_campaign_started;
        buf[gates_base + 9] = self.expected_party_size;
        buf[gates_base + 10] = self.online_party_count;
        buf[gates_base + 11] = self.ready_party_count;
        buf[gates_base + 12..gates_base + 16].copy_from_slice(&self.room_uid.to_le_bytes());
        buf[gates_base + 16] = self.team_partner_slot;
        buf[gates_base + 17] = self.teamed_mask;
        buf[gates_base + 18] = self.session_state;
        buf[gates_base + 19] = self.session_kind;
        buf[gates_base + 20] = self.session_peer_slot;
        buf[gates_base + 21] = self.session_seq;
        buf[gates_base + 22] = self.session_reason;
        buf[gates_base + 23] = self.trade_pending;

        buf
    }

    /// The fixed packet plus the recipient's trade trailer when there is one.
    pub fn encode_with_trade(&self, trade: Option<&TradeIn>) -> Vec<u8> {
        let mut buf = self.encode_le().to_vec();
        if let Some(trade) = trade {
            let mut trailer = [0u8; TRADE_IN_SIZE];
            trade.encode_le(&mut trailer);
            buf.extend_from_slice(&trailer);
        }
        buf
    }

    /// Compact rejection: the 16-byte header only (see `ERROR_PACKET_SIZE`).
    pub fn create_error_response(status_code: u8) -> [u8; ERROR_PACKET_SIZE] {
        let mut buf = [0u8; ERROR_PACKET_SIZE];
        buf[0..4].copy_from_slice(&COOP_MAGIC.to_le_bytes());
        buf[4..6].copy_from_slice(&PROTOCOL_VERSION.to_le_bytes());
        buf[6] = AUTO_PLAYER_ID;
        buf[12] = status_code;
        buf
    }

    /// Return-routability challenge: only a client that really receives at
    /// its source address learns the cookie it must echo.
    pub fn create_challenge_response(cookie: u64) -> [u8; CHALLENGE_PACKET_SIZE] {
        let mut buf = [0u8; CHALLENGE_PACKET_SIZE];
        buf[..ERROR_PACKET_SIZE]
            .copy_from_slice(&Self::create_error_response(ROOM_STATUS_CHALLENGE));
        buf[16..24].copy_from_slice(&cookie.to_le_bytes());
        buf
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_state() -> PlayerState {
        PlayerState {
            map_id: 4,
            matrix_id: 0,
            avatar_state: 1,
            x: 12,
            z: 25,
            height: 16,
            direction: 3,
            gender: 1,
            sprite_id: 99,
            move_action: 15,
            move_seq: 7,
            is_active: 1,
            client_ready: 1,
            save_status: SAVE_STATUS_FRESH,
            activity: 1,
            trainer_id: 0xA1B2_C3D4,
            name: [0x0133, 0x0134, 0xFFFF, 0, 0, 0, 0, 0],
            follower_species: 155,
            follower_sprite_id: 400,
            follower_form: 1,
            follower_shiny: 1,
            follower_gender: 0,
            follower_visible: 1,
            follower_x: 12,
            follower_z: 26,
            follower_height: 16,
            follower_direction: 3,
            follower_move_action: 15,
            follower_move_seq: 6,
            pad39: [0; 3],
            avatar_id: 2,
            anim: 3,
            field_busy: 1,
            pad3f: 0,
        }
    }

    #[test]
    fn test_client_packet_roundtrip() {
        let pkt = ClientToServerPacket {
            magic: COOP_MAGIC,
            version: PROTOCOL_VERSION,
            room_code: *b"JOHTO1",
            room_action: ROOM_ACTION_CREATE,
            requested_player_id: AUTO_PLAYER_ID,
            allow_ahead: 0,
            target_party_size: 4,
            host_token: 0x1234_5678,
            badge_mask_local: 0b0000_0011,
            sequence: 512,
            heartbeat_rom: 1024,
            reserved: 0,
            local_player: sample_state(),
            cookie: 0x0102_0304_0506_0708,
            player_secret: 0x1122_3344_5566_7788,
            session: SessionOut {
                seq: 9,
                op: SESSION_OP_REQUEST,
                kind: SESSION_KIND_TEAM_UP,
                target_trainer: 0xA1B2_C3D4,
            },
            trade: None,
        };

        let bytes = pkt.encode_le();
        assert_eq!(bytes.len(), 120);
        assert_eq!(
            &bytes[112..116],
            &[9, SESSION_OP_REQUEST, SESSION_KIND_TEAM_UP, 0]
        );
        assert_eq!(&bytes[116..120], &[0xD4, 0xC3, 0xB2, 0xA1]);
        let decoded = ClientToServerPacket::decode_le(&bytes).expect("valid decode");
        assert_eq!(pkt, decoded);

        // With a trade open, the CoopTradeOut trailer follows (coop_sync.h).
        let mut mon = [0u8; TRADE_MON_SIZE];
        mon[0..4].copy_from_slice(&0x1234_5678u32.to_le_bytes());
        mon[TRADE_MON_SIZE - 1] = 0xEE;
        let with_trade = ClientToServerPacket {
            trade: Some(TradeOut {
                state: TRADE_OUT_CONFIRMED,
                offer_id: 2,
                confirm_id: 3,
                trade_id: 0xAABB_CCDD,
                mon,
            }),
            ..pkt
        };
        let full = with_trade.encode_full();
        assert_eq!(full.len(), CLIENT_PACKET_SIZE + TRADE_OUT_SIZE);
        assert_eq!(
            &full[120..128],
            &[TRADE_OUT_CONFIRMED, 2, 3, 0, 0xDD, 0xCC, 0xBB, 0xAA]
        );
        assert_eq!(full[128..132], 0x1234_5678u32.to_le_bytes());
        let decoded = ClientToServerPacket::decode_le(&full).expect("valid decode");
        assert_eq!(decoded, with_trade);
        assert_eq!(mon_pid(&decoded.trade.unwrap().mon), 0x1234_5678);
    }

    #[test]
    fn test_trade_in_trailer_layout() {
        let mut t = TradeIn::empty(TRADE_IN_COMMIT);
        t.reason = 0;
        t.partner_offer_id = 4;
        t.my_offer_id = 5;
        t.partner_confirmed = 1;
        t.peer_slot = 2;
        t.trade_id = 0x0102_0304;
        t.give_pid = 0xA0B0_C0D0;
        t.mon[0] = 0x77;
        let mut buf = [0u8; TRADE_IN_SIZE];
        t.encode_le(&mut buf);
        // CoopTradeIn: state, reason, partner_offer_id, my_offer_id,
        // partner_confirmed, peer_slot, pad[2], trade_id @8, give_pid @12, mon @16.
        assert_eq!(&buf[0..8], &[TRADE_IN_COMMIT, 0, 4, 5, 1, 2, 0, 0]);
        assert_eq!(&buf[8..16], &[4, 3, 2, 1, 0xD0, 0xC0, 0xB0, 0xA0]);
        assert_eq!(buf[16], 0x77);
        assert_eq!(TradeIn::decode_le(&buf), t);
    }

    #[test]
    fn test_strict_room_code_validation() {
        assert_eq!(validate_room_code(*b"johTo9"), Ok(*b"JOHTO9"));
        assert_eq!(
            validate_room_code(*b"JOHT_1"),
            Err(ROOM_STATUS_ERR_INVALID_CODE)
        );
        assert_eq!(
            validate_room_code([b'A', b'B', b'C', 0, 0, 0]),
            Err(ROOM_STATUS_ERR_INVALID_CODE)
        );
    }

    #[test]
    fn test_player_state_layout_matches_coop_sync_h() {
        // Offsets pinned by _Static_assert in hg-engine/include/coop_sync.h.
        let st = sample_state();
        let mut buf = [0u8; PLAYER_STATE_SIZE];
        st.encode_le(&mut buf);
        assert_eq!(&buf[0x00..0x02], &[4, 0]);
        assert_eq!(buf[0x03], 1); // avatar_state
        assert_eq!(&buf[0x08..0x0A], &[16, 0]); // height
        assert_eq!(&buf[0x0C..0x0E], &[99, 0]); // sprite_id
        assert_eq!(buf[0x0F], 7); // move_seq
        assert_eq!(buf[0x12], SAVE_STATUS_FRESH);
        assert_eq!(&buf[0x14..0x18], &[0xD4, 0xC3, 0xB2, 0xA1]);
        assert_eq!(&buf[0x18..0x1A], &[0x33, 0x01]); // name[0]
        assert_eq!(&buf[0x28..0x2A], &[155, 0]); // follower_species
        assert_eq!(&buf[0x30..0x32], &[12, 0]); // follower_x
        assert_eq!(buf[0x38], 6); // follower_move_seq
        assert_eq!(buf[0x3C], 2); // avatar_id
        assert_eq!(buf[0x3D], 3); // anim
        assert_eq!(buf[0x3E], 1); // field_busy
        assert_eq!(PlayerState::decode_le(&buf).unwrap(), st);

        // The bridge copies packet[16..360] to mailbox 0x050..0x1A8, so the
        // gates block (mailbox 0x190) must start at packet offset 0x190 - 0x50 + 16.
        let gates_packet_offset = 16 + REMOTE_SLOTS * PLAYER_STATE_SIZE;
        assert_eq!(gates_packet_offset, 0x190 - 0x50 + 16);
        assert_eq!(SERVER_PACKET_SIZE, 360);
    }

    #[test]
    fn test_room_uid_lands_in_the_gates_block() {
        let pkt = ServerToClientPacket {
            magic: COOP_MAGIC,
            version: PROTOCOL_VERSION,
            assigned_player_id: 0,
            active_players_mask: 1,
            server_tick: 1,
            status_code: ROOM_STATUS_OK,
            reserved_header: [0; 3],
            remote_players: [PlayerState::default(); REMOTE_SLOTS],
            badge_mask_local_echo: 0,
            badge_mask_synced: 0,
            flag_coop_sync_ready: 0,
            current_checkpoint: 0,
            assigned_monolocke: 0,
            nuzlocke_failed: 0,
            flag_campaign_started: 1,
            expected_party_size: 2,
            online_party_count: 1,
            ready_party_count: 1,
            room_uid: 0xCAFE_F00D,
            team_partner_slot: 2,
            teamed_mask: TEAMED_SELF_BIT | 0b100,
            session_state: SESSION_STATE_INCOMING,
            session_kind: SESSION_KIND_TRADE,
            session_peer_slot: 3,
            session_seq: 7,
            session_reason: SESSION_REASON_NONE,
            trade_pending: 0,
        };
        let buf = pkt.encode_le();
        let gates = 16 + REMOTE_SLOTS * PLAYER_STATE_SIZE;
        assert_eq!(buf[gates + 8], 1); // flag_campaign_started (CoopProgressionGates 0x08)
        assert_eq!(&buf[gates + 12..gates + 16], &[0x0D, 0xF0, 0xFE, 0xCA]); // room_uid (0x0C)
        // Session and team fields: CoopProgressionGates 0x10..0x17 (coop_sync.h).
        assert_eq!(
            &buf[gates + 16..gates + 24],
            &[
                2,
                0x84,
                SESSION_STATE_INCOMING,
                SESSION_KIND_TRADE,
                3,
                7,
                0,
                0
            ]
        );
    }

    #[test]
    fn test_error_response_is_smaller_than_request() {
        let err = ServerToClientPacket::create_error_response(ROOM_STATUS_ERR_FULL);
        assert!(err.len() < CLIENT_PACKET_SIZE);
        let challenge = ServerToClientPacket::create_challenge_response(0xDEAD_BEEF);
        assert!(challenge.len() < CLIENT_PACKET_SIZE);
        assert_eq!(challenge[12], ROOM_STATUS_CHALLENGE);
        assert_eq!(
            u64::from_le_bytes(challenge[16..24].try_into().unwrap()),
            0xDEAD_BEEF
        );
        assert_eq!(
            u32::from_le_bytes([err[0], err[1], err[2], err[3]]),
            COOP_MAGIC
        );
        assert_eq!(err[12], ROOM_STATUS_ERR_FULL);
    }

    #[test]
    fn test_rejects_bad_magic() {
        let mut bytes = [0u8; CLIENT_PACKET_SIZE];
        bytes[0..4].copy_from_slice(&0xDEAD_BEEFu32.to_le_bytes());
        assert!(matches!(
            ClientToServerPacket::decode_le(&bytes),
            Err(ProtocolError::InvalidMagic(0xDEAD_BEEF))
        ));
    }
}
