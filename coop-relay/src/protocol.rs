use std::fmt;

pub const COOP_MAGIC: u32 = 0x434F_4F50; // 'COOP'
pub const PROTOCOL_VERSION: u16 = 8;
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
/// Milestone 5: every roster player of this Nuzlocke run became a spirit, so
/// the run is over and the room takes no more joins (docs/MILESTONE5_PLAN.md §5.5.5).
pub const ROOM_STATUS_ERR_RUN_LOST: u8 = 10;
/// The client speaks another protocol version. Sent in the 16-byte error header, whose bytes
/// 4..6 name this relay's `PROTOCOL_VERSION`, so the client can say which versions differ
/// (docs/RELEASE_PLAN.md §5).
pub const ROOM_STATUS_ERR_PROTOCOL: u8 = 11;

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
pub const SESSION_KIND_LEAVE_TEAM: u8 = 4; // one-sided: applied at once, no answer
/// Two teams battle (2v2): set by the relay for a BATTLE request between
/// two teamed players; all four must accept. For the requester's teammate,
/// `session_reason` carries the opponent's remote slot while it is asked.
pub const SESSION_KIND_TEAM_BATTLE: u8 = 5;
/// Milestone 5 Ghost Mode: a living player gives a Pokemon to a spirit. No
/// answer: the relay opens a one-sided trade at once (TRADE_FLAG_REVIVE).
pub const SESSION_KIND_REVIVE: u8 = 6;

pub const SESSION_STATE_NONE: u8 = 0;
pub const SESSION_STATE_OUTGOING: u8 = 1; // my request, waiting for the answer
pub const SESSION_STATE_INCOMING: u8 = 2; // someone asks me
pub const SESSION_STATE_ACCEPTED: u8 = 3;
pub const SESSION_STATE_DECLINED: u8 = 4;
pub const SESSION_STATE_CANCELLED: u8 = 5; // withdrawn, or the other side left
pub const SESSION_STATE_TIMEOUT: u8 = 6;
pub const SESSION_STATE_REFUSED: u8 = 7; // the relay turned the request down (reason)
pub const SESSION_STATE_NOTICE: u8 = 8; // to the ex-teammate: the peer left the team (kind LEAVE_TEAM)

pub const SESSION_REASON_NONE: u8 = 0;
pub const SESSION_REASON_NOT_HERE: u8 = 1; // target not connected
pub const SESSION_REASON_BUSY: u8 = 2; // either side already busy (a session, a link, a boss, a battle)

/// `PlayerState.activity` (the ROM's enum CoopActivity): in a battle.
pub const ACTIVITY_BATTLE: u8 = 2;
pub const SESSION_REASON_TEAM_RULE: u8 = 3; // a team cannot battle a solo player or its own teammate
pub const SESSION_REASON_ALREADY_TEAMED: u8 = 4;
pub const SESSION_REASON_INVALID: u8 = 5; // unknown kind, or a request to oneself
pub const SESSION_REASON_TEAMMATE_AWAY: u8 = 6; // a team battle needs all four players here

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
    /// Milestone 5: TRADE_FLAG_* (byte 6).
    pub flags: u8,
    pub trade_id: u32,
    pub give_pid: u32,
    pub mon: [u8; TRADE_MON_SIZE],
}

/// TradeIn.flags: a revive (Ghost Mode donation, docs/MILESTONE5_PLAN.md
/// §5.5.4) is one-sided: the donor gives, the spirit (RECEIVER) only takes.
pub const TRADE_FLAG_REVIVE: u8 = 0x01;
pub const TRADE_FLAG_RECEIVER: u8 = 0x02;

impl TradeIn {
    pub fn empty(state: u8) -> Self {
        Self {
            state,
            reason: 0,
            partner_offer_id: 0,
            my_offer_id: 0,
            partner_confirmed: 0,
            peer_slot: NO_SLOT,
            flags: 0,
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
        out[6] = self.flags;
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
            flags: buf[6],
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
    pub boss_wait: u8, // 0x39 team boss waited at (Phase 5): the ROM's boss index + 1; 0 none
    pub badge_mask: [u8; 2], // 0x3A the player's sixteen badges (ROM: badge-scaled team bosses use the lower count)
    pub avatar_id: u8,       // 0x3C
    pub anim: u8,            // 0x3D pose animation index (forwarded untouched)
    pub field_busy: u8,      // 0x3E (a field task or message is running; forwarded as is)
    pub challenge_flags: u8, // 0x3F (Milestone 5: bit 0 spirit / Ghost Mode; forwarded as is)
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
        out[0x39] = self.boss_wait;
        out[0x3A..0x3C].copy_from_slice(&self.badge_mask);
        out[0x3C] = self.avatar_id;
        out[0x3D] = self.anim;
        out[0x3E] = self.field_busy;
        out[0x3F] = self.challenge_flags;
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
            boss_wait: buf[0x39],
            badge_mask: [buf[0x3A], buf[0x3B]],
            avatar_id: buf[0x3C],
            anim: buf[0x3D],
            field_busy: buf[0x3E],
            challenge_flags: buf[0x3F],
        })
    }
}

// Virtual link (docs/MILESTONE3_PLAN.md §15). While the ROM runs a
// local-wireless session, its link block (hg-engine coop_sync.h
// CoopLinkBlock) is carried by a trailer after any trade trailer, starting
// with LINK_MAGIC: the client's messages and beacon up, the routed messages
// and the room's beacons down. Each direction of each hop is a reliable,
// ordered stream: consecutive u16 sequence numbers, a cumulative ack (the
// next sequence number expected), resend until acked.
pub const LINK_MAGIC: u32 = 0x314B_4E4C; // "LNK1"
pub const LINK_DATA_MAX: usize = 0xC0;
pub const LINK_BEACON_MAX: usize = 0x70;
pub const LINK_MAX_MSGS: usize = 16; // per trailer
/// Bytes of messages (4-byte header + data each) per trailer, both ways
/// (coop-bridge CoopLink.cs SendBudget); keeps a packet under one MTU.
pub const LINK_SEND_BUDGET: usize = 1000;
/// The largest link trailer up: header, a full beacon, a full budget.
pub const LINK_UP_MAX: usize = 12 + 8 + LINK_BEACON_MAX + LINK_SEND_BUDGET;
/// The largest client packet: fixed part, trade, link and challenge trailers.
pub const CLIENT_PACKET_MAX: usize =
    CLIENT_PACKET_SIZE + TRADE_OUT_SIZE + LINK_UP_MAX + CHALLENGE_UP_SIZE;

/// The ROM's 4-player multi battle link modes (ccd COMM_TYPE_MULTI_BATTLE_1 / _2):
/// a CONNECT's first data byte. Teams sit on opposite sides by aid.
pub const LINK_MODE_MULTI_BATTLE: [u8; 2] = [4, 5];

pub const LINK_MSG_DATA: u8 = 1;
pub const LINK_MSG_CONNECT: u8 = 2;
pub const LINK_MSG_ACCEPT: u8 = 3;
pub const LINK_MSG_REFUSE: u8 = 4;
pub const LINK_MSG_BYE: u8 = 5;

pub const LINK_ROLE_NONE: u8 = 0;
pub const LINK_ROLE_PARENT: u8 = 1;
pub const LINK_ROLE_CHILD: u8 = 2;

/// One message of the link block's rings (CoopLinkMsg).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct LinkMsg {
    pub kind: u8,
    pub aid: u8,
    pub data: Vec<u8>,
}

/// A parent's beacon (CoopLinkBeacon): the game's CommBeacon bytes.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct LinkBeacon {
    pub ggid: u32,
    pub entry: u8,
    pub seq: u8,
    pub data: Vec<u8>,
}

/// Client -> relay link trailer.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct LinkUp {
    pub epoch: u8,      // bumped by the bridge for every new link block
    pub role: u8,       // LINK_ROLE_*
    pub ack: u16,       // next relay->client sequence number expected
    pub first_seq: u16, // sequence number of msgs[0]
    pub beacon: Option<LinkBeacon>,
    pub msgs: Vec<LinkMsg>,
}

/// Relay -> client link trailer.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct LinkDown {
    pub ack: u16,                       // next client->relay sequence number expected
    pub first_seq: u16,                 // sequence number of msgs[0]
    /// The link block (the client's epoch) this stream belongs to: bytes 10-11 (epoch, 1).
    /// A late trailer of an earlier block must not ack or deliver into the next one (the
    /// stream restarts at 0). Relays before it sent 0, 0 there: no epoch, accepted.
    pub epoch: Option<u8>,
    pub beacons: Vec<(u8, LinkBeacon)>, // (absolute player id, beacon) of the room's parents
    pub msgs: Vec<LinkMsg>,
}

fn put_msgs(buf: &mut Vec<u8>, msgs: &[LinkMsg]) {
    for m in msgs {
        buf.push(m.kind);
        buf.push(m.aid);
        buf.extend_from_slice(&(m.data.len() as u16).to_le_bytes());
        buf.extend_from_slice(&m.data);
    }
}

/// Bounds-checked reader for the trailers (untrusted input).
struct Reader<'a> {
    buf: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Option<&'a [u8]> {
        let end = self.pos.checked_add(n)?;
        let s = self.buf.get(self.pos..end)?;
        self.pos = end;
        Some(s)
    }
    fn u8(&mut self) -> Option<u8> {
        self.take(1).map(|s| s[0])
    }
    fn u16(&mut self) -> Option<u16> {
        self.take(2).map(|s| u16::from_le_bytes([s[0], s[1]]))
    }
    fn u32(&mut self) -> Option<u32> {
        self.take(4)
            .map(|s| u32::from_le_bytes([s[0], s[1], s[2], s[3]]))
    }
    fn msgs(&mut self, count: usize) -> Option<Vec<LinkMsg>> {
        if count > LINK_MAX_MSGS {
            return None;
        }
        (0..count)
            .map(|_| {
                let kind = self.u8()?;
                let aid = self.u8()?;
                let len = self.u16()? as usize;
                if len > LINK_DATA_MAX {
                    return None;
                }
                Some(LinkMsg {
                    kind,
                    aid,
                    data: self.take(len)?.to_vec(),
                })
            })
            .collect()
    }
}

impl LinkUp {
    pub fn encode(&self, buf: &mut Vec<u8>) {
        buf.extend_from_slice(&LINK_MAGIC.to_le_bytes());
        buf.push(self.epoch);
        buf.push(self.role);
        buf.push(u8::from(self.beacon.is_some()));
        buf.push(self.msgs.len() as u8);
        buf.extend_from_slice(&self.ack.to_le_bytes());
        buf.extend_from_slice(&self.first_seq.to_le_bytes());
        if let Some(b) = &self.beacon {
            buf.extend_from_slice(&b.ggid.to_le_bytes());
            buf.push(b.data.len() as u8);
            buf.push(b.entry);
            buf.push(b.seq);
            buf.push(0);
            buf.extend_from_slice(&b.data);
        }
        put_msgs(buf, &self.msgs);
    }

    /// None when the trailer is missing or malformed (it is then ignored).
    pub fn decode(buf: &[u8]) -> Option<Self> {
        let mut r = Reader { buf, pos: 0 };
        if r.u32()? != LINK_MAGIC {
            return None;
        }
        let epoch = r.u8()?;
        let role = r.u8()?;
        let flags = r.u8()?;
        let count = r.u8()? as usize;
        let ack = r.u16()?;
        let first_seq = r.u16()?;
        let beacon = if flags & 1 != 0 {
            let ggid = r.u32()?;
            let len = r.u8()? as usize;
            let entry = r.u8()?;
            let seq = r.u8()?;
            r.u8()?;
            if len > LINK_BEACON_MAX {
                return None;
            }
            Some(LinkBeacon {
                ggid,
                entry,
                seq,
                data: r.take(len)?.to_vec(),
            })
        } else {
            None
        };
        let msgs = r.msgs(count)?;
        Some(Self {
            epoch,
            role,
            ack,
            first_seq,
            beacon,
            msgs,
        })
    }
}

impl LinkDown {
    pub fn encode(&self, buf: &mut Vec<u8>) {
        buf.extend_from_slice(&LINK_MAGIC.to_le_bytes());
        buf.push(self.beacons.len() as u8);
        buf.push(self.msgs.len() as u8);
        buf.extend_from_slice(&self.ack.to_le_bytes());
        buf.extend_from_slice(&self.first_seq.to_le_bytes());
        match self.epoch {
            Some(e) => buf.extend_from_slice(&[e, 1]),
            None => buf.extend_from_slice(&[0, 0]),
        }
        for (player, b) in &self.beacons {
            buf.push(*player);
            buf.push(b.data.len() as u8);
            buf.push(b.entry);
            buf.push(b.seq);
            buf.extend_from_slice(&b.ggid.to_le_bytes());
            buf.extend_from_slice(&b.data);
        }
        put_msgs(buf, &self.msgs);
    }

    pub fn decode(buf: &[u8]) -> Option<Self> {
        let mut r = Reader { buf, pos: 0 };
        if r.u32()? != LINK_MAGIC {
            return None;
        }
        let beacon_count = r.u8()? as usize;
        let count = r.u8()? as usize;
        let ack = r.u16()?;
        let first_seq = r.u16()?;
        let epoch_byte = r.u8()?;
        let epoch = if r.u8()? == 1 { Some(epoch_byte) } else { None };
        if beacon_count > MAX_PLAYERS {
            return None;
        }
        let mut beacons = Vec::with_capacity(beacon_count);
        for _ in 0..beacon_count {
            let player = r.u8()?;
            let len = r.u8()? as usize;
            let entry = r.u8()?;
            let seq = r.u8()?;
            let ggid = r.u32()?;
            if len > LINK_BEACON_MAX {
                return None;
            }
            beacons.push((
                player,
                LinkBeacon {
                    ggid,
                    entry,
                    seq,
                    data: r.take(len)?.to_vec(),
                },
            ));
        }
        let msgs = r.msgs(count)?;
        Some(Self {
            ack,
            first_seq,
            epoch,
            beacons,
            msgs,
        })
    }
}

fn starts_with_link_magic(buf: &[u8]) -> bool {
    buf.len() >= 4 && u32::from_le_bytes([buf[0], buf[1], buf[2], buf[3]]) == LINK_MAGIC
}

// --- Milestone 5: challenge rules and ledgers (docs/MILESTONE5_PLAN.md §4.2, §4.4) ---
//
// The host's rules travel in client bytes 30..32 (honored only while the room
// forms). Each ROM reports its challenge state in a fixed-size trailer that is
// always the last part of its packet; the relay answers with one, always the
// last part of the server packet. Both end with a checksum, so a link trailer
// whose tail happens to spell the magic is not mistaken for one.

pub const CHALLENGE_RULE_RANDOM_MASK: u16 = 0x0003; // 0 none, 1 semi-random, 2 preserve difficulty, 3 fully random
pub const CHALLENGE_RULE_LEGENDS_WILD: u16 = 0x0004;
pub const CHALLENGE_RULE_RANDOM_ABILITIES: u16 = 0x0008;
pub const CHALLENGE_RULE_NUZLOCKE: u16 = 0x0010;
pub const CHALLENGE_RULE_SOUL_LINK_SHIFT: u16 = 5; // 0 off, 1 room-wide, 2 duo
pub const CHALLENGE_RULE_SOUL_LINK_MASK: u16 = 0x0060;
pub const CHALLENGE_RULE_MONOLOCKE: u16 = 0x0080;
pub const CHALLENGE_RULE_LEVEL_CAP: u16 = 0x0100;
pub const CHALLENGE_RULE_RANDOM_ITEMS: u16 = 0x0200; // protocol 8
pub const CHALLENGE_RULES_VALID: u16 = 0x03FF;

pub const SOUL_LINK_OFF: u16 = 0;
pub const SOUL_LINK_ROOM_WIDE: u16 = 1;
pub const SOUL_LINK_DUO: u16 = 2;

/// Monolocke types are 0..17 (type 9 is Fairy in hg-engine); 18 means none.
pub const MONO_TYPE_COUNT: u8 = 18;
pub const MONO_TYPE_NONE: u8 = 18;

/// Room state bits (gates +7 and the down trailer).
pub const CHALLENGE_ROOM_RUN_LOST: u8 = 0x01;

/// Player state 0x3F bits.
pub const CHALLENGE_FLAG_SPIRIT: u8 = 0x01;

/// Location bitmaps: 128 places (docs/MILESTONE5_PLAN.md §4.3).
pub const CHALLENGE_LOCATION_BYTES: usize = 16;

pub const CHALLENGE_MAGIC: u32 = 0x314C_4843; // "CHL1"
pub const CHALLENGE_VERSION: u8 = 1;
pub const CHALLENGE_UP_SIZE: usize = 48;
pub const CHALLENGE_DOWN_SIZE: usize = 64;

/// The ROM's challenge state bits (up trailer `state`).
pub const CHALLENGE_STATE_LATCHED: u8 = 0x01;
pub const CHALLENGE_STATE_SPIRIT: u8 = 0x02;
pub const CHALLENGE_STATE_RUN_LOST: u8 = 0x04;
pub const CHALLENGE_STATE_HAD_POKEMON: u8 = 0x08;

/// ChallengeDown.room_state and gates `challenge_room_state` bits.
pub const ROOM_STATE_RUN_LOST: u8 = 0x01;

/// Drops what the bridge should never have sent: unknown bits, an unknown
/// soul-link mode, and rules that need another one (soul-link needs Nuzlocke;
/// legends and random abilities need the randomizer).
pub fn sanitize_challenge_rules(raw: u16) -> u16 {
    let mut rules = raw & CHALLENGE_RULES_VALID;
    let soul_link = (rules & CHALLENGE_RULE_SOUL_LINK_MASK) >> CHALLENGE_RULE_SOUL_LINK_SHIFT;
    if soul_link > SOUL_LINK_DUO || rules & CHALLENGE_RULE_NUZLOCKE == 0 {
        rules &= !CHALLENGE_RULE_SOUL_LINK_MASK;
    }
    if rules & CHALLENGE_RULE_RANDOM_MASK == 0 {
        rules &= !(CHALLENGE_RULE_LEGENDS_WILD | CHALLENGE_RULE_RANDOM_ABILITIES);
    }
    rules
}

pub fn soul_link_mode(rules: u16) -> u16 {
    (rules & CHALLENGE_RULE_SOUL_LINK_MASK) >> CHALLENGE_RULE_SOUL_LINK_SHIFT
}

/// 16-bit sum of the bytes before the checksum field.
fn challenge_checksum(bytes: &[u8]) -> u16 {
    bytes
        .iter()
        .fold(0u16, |acc, &b| acc.wrapping_add(u16::from(b)))
}

/// Client -> relay (48 bytes, the ROM's `CoopChallengeSync.out`):
/// magic u32 @0, version u8 @4, state u8 @5, living u8 @6, pad @7,
/// rules u16 @8 (what the save latched), deaths u16 @10, spent [16] @12,
/// failed [16] @28, checksum u16 @44, pad u16 @46.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ChallengeUp {
    pub state: u8,
    pub living: u8,
    pub rules: u16,
    pub deaths: u16,
    pub spent: [u8; CHALLENGE_LOCATION_BYTES],
    pub failed: [u8; CHALLENGE_LOCATION_BYTES],
}

impl ChallengeUp {
    pub fn encode(&self) -> [u8; CHALLENGE_UP_SIZE] {
        let mut out = [0u8; CHALLENGE_UP_SIZE];
        out[0..4].copy_from_slice(&CHALLENGE_MAGIC.to_le_bytes());
        out[4] = CHALLENGE_VERSION;
        out[5] = self.state;
        out[6] = self.living;
        out[8..10].copy_from_slice(&self.rules.to_le_bytes());
        out[10..12].copy_from_slice(&self.deaths.to_le_bytes());
        out[12..28].copy_from_slice(&self.spent);
        out[28..44].copy_from_slice(&self.failed);
        let sum = challenge_checksum(&out[..44]);
        out[44..46].copy_from_slice(&sum.to_le_bytes());
        out
    }

    /// None unless `buf` is exactly a well-formed trailer.
    pub fn decode(buf: &[u8]) -> Option<Self> {
        if buf.len() != CHALLENGE_UP_SIZE
            || u32::from_le_bytes([buf[0], buf[1], buf[2], buf[3]]) != CHALLENGE_MAGIC
            || buf[4] != CHALLENGE_VERSION
            || u16::from_le_bytes([buf[44], buf[45]]) != challenge_checksum(&buf[..44])
        {
            return None;
        }
        let mut spent = [0u8; CHALLENGE_LOCATION_BYTES];
        let mut failed = [0u8; CHALLENGE_LOCATION_BYTES];
        spent.copy_from_slice(&buf[12..28]);
        failed.copy_from_slice(&buf[28..44]);
        Some(Self {
            state: buf[5],
            living: buf[6],
            rules: u16::from_le_bytes([buf[8], buf[9]]),
            deaths: u16::from_le_bytes([buf[10], buf[11]]),
            spent,
            failed,
        })
    }
}

/// Relay -> client (64 bytes, written into the ROM's `CoopChallengeSync.in`):
/// magic u32 @0, version u8 @4, mono_type u8 @5, room_state u8 @6, pad @7,
/// rules u16 @8, team_deaths u16 @10, link_union [16] @12, spent [16] @28 (the relay's
/// copy of this player's), failed [16] @44 (likewise), checksum u16 @60, pad @62.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChallengeDown {
    pub mono_type: u8,
    pub room_state: u8,
    pub rules: u16,
    /// Every roster trainer's deaths, for the "Run lost" summary.
    pub team_deaths: u16,
    pub link_union: [u8; CHALLENGE_LOCATION_BYTES],
    pub spent: [u8; CHALLENGE_LOCATION_BYTES],
    pub failed: [u8; CHALLENGE_LOCATION_BYTES],
}

impl ChallengeDown {
    pub fn encode(&self) -> [u8; CHALLENGE_DOWN_SIZE] {
        let mut out = [0u8; CHALLENGE_DOWN_SIZE];
        out[0..4].copy_from_slice(&CHALLENGE_MAGIC.to_le_bytes());
        out[4] = CHALLENGE_VERSION;
        out[5] = self.mono_type;
        out[6] = self.room_state;
        out[8..10].copy_from_slice(&self.rules.to_le_bytes());
        out[10..12].copy_from_slice(&self.team_deaths.to_le_bytes());
        out[12..28].copy_from_slice(&self.link_union);
        out[28..44].copy_from_slice(&self.spent);
        out[44..60].copy_from_slice(&self.failed);
        let sum = challenge_checksum(&out[..60]);
        out[60..62].copy_from_slice(&sum.to_le_bytes());
        out
    }

    pub fn decode(buf: &[u8]) -> Option<Self> {
        if buf.len() != CHALLENGE_DOWN_SIZE
            || u32::from_le_bytes([buf[0], buf[1], buf[2], buf[3]]) != CHALLENGE_MAGIC
            || buf[4] != CHALLENGE_VERSION
            || u16::from_le_bytes([buf[60], buf[61]]) != challenge_checksum(&buf[..60])
        {
            return None;
        }
        let mut link_union = [0u8; CHALLENGE_LOCATION_BYTES];
        let mut spent = [0u8; CHALLENGE_LOCATION_BYTES];
        let mut failed = [0u8; CHALLENGE_LOCATION_BYTES];
        link_union.copy_from_slice(&buf[12..28]);
        spent.copy_from_slice(&buf[28..44]);
        failed.copy_from_slice(&buf[44..60]);
        Some(Self {
            mono_type: buf[5],
            room_state: buf[6],
            rules: u16::from_le_bytes([buf[8], buf[9]]),
            team_deaths: u16::from_le_bytes([buf[10], buf[11]]),
            link_union,
            spent,
            failed,
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

/// Sent by BizHawk C# Plugin -> Rust Relay Server (120 bytes, plus trailers)
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClientToServerPacket {
    pub magic: u32,                     // 0..4
    pub version: u16,                   // 4..6
    pub room_code: [u8; 6],             // 6..12
    pub room_action: u8,                // 12 (1 = Create, 2 = Join, 3 = Sync, 4 = Leave)
    pub requested_player_id: u8,        // 13 (0..5 or 0xFF)
    pub allow_ahead: u8, // 14 (1 = this player lets teammates go ahead past the gym gate)
    pub target_party_size: u8, // 15 (host's room size, 2..6; only honored from the host)
    pub host_token: u32, // 16..20
    pub badge_mask_local: u16, // 20..22
    pub sequence: u32,   // 22..26
    pub heartbeat_rom: u32, // 26..30
    pub challenge_rules: u16, // 30..32 (Milestone 5 room rules, CHALLENGE_RULE_*; only honored from the host while the room forms)
    pub local_player: PlayerState, // 32..96 (64 bytes)
    pub cookie: u64,          // 96..104 (0 until the relay issues one)
    pub player_secret: u64,   // 104..112 (random per bridge install, never 0)
    pub session: SessionOut,  // 112..120 (seq, op, kind, pad, u32 target trainer ID)
    pub trade: Option<TradeOut>, // 120..364 while the ROM has a trade open
    pub link: Option<LinkUp>, // after the trade trailer (or at 120) while a link block exists
    pub challenge: Option<ChallengeUp>, // the last CHALLENGE_UP_SIZE bytes while the ROM's field code runs (Milestone 5)
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
        buf[30..32].copy_from_slice(&self.challenge_rules.to_le_bytes());
        self.local_player.encode_le(&mut buf[32..96]);
        buf[96..104].copy_from_slice(&self.cookie.to_le_bytes());
        buf[104..112].copy_from_slice(&self.player_secret.to_le_bytes());
        buf[112] = self.session.seq;
        buf[113] = self.session.op;
        buf[114] = self.session.kind;
        buf[116..120].copy_from_slice(&self.session.target_trainer.to_le_bytes());
        buf
    }

    /// The fixed packet plus the trade and link trailers when there are some.
    pub fn encode_full(&self) -> Vec<u8> {
        let mut buf = self.encode_le().to_vec();
        if let Some(trade) = &self.trade {
            let mut trailer = [0u8; TRADE_OUT_SIZE];
            trade.encode_le(&mut trailer);
            buf.extend_from_slice(&trailer);
        }
        if let Some(link) = &self.link {
            link.encode(&mut buf);
        }
        if let Some(challenge) = &self.challenge {
            buf.extend_from_slice(&challenge.encode());
        }
        buf
    }

    pub fn decode_le(buf: &[u8]) -> Result<Self, ProtocolError> {
        // Magic and version first: another version's packet may have another size, and its
        // sender should still learn that the versions differ (main.rs answers it).
        if buf.len() >= 6 {
            let magic = u32::from_le_bytes([buf[0], buf[1], buf[2], buf[3]]);
            if magic != COOP_MAGIC {
                return Err(ProtocolError::InvalidMagic(magic));
            }
            let version = u16::from_le_bytes([buf[4], buf[5]]);
            if version != PROTOCOL_VERSION {
                return Err(ProtocolError::UnsupportedVersion(version));
            }
        }
        if buf.len() < CLIENT_PACKET_SIZE {
            return Err(ProtocolError::PacketTooShort {
                expected: CLIENT_PACKET_SIZE,
                actual: buf.len(),
            });
        }

        let mut raw_room = [0u8; 6];
        raw_room.copy_from_slice(&buf[6..12]);

        let local_player = PlayerState::decode_le(&buf[32..96])?;

        // Trailers: the challenge trailer (fixed size, always last, found from
        // the end by its magic and checksum), then, in what remains, the
        // trade's (fixed size) unless a link trailer starts right away, then
        // the link's (starts with LINK_MAGIC).
        let mut rest = &buf[CLIENT_PACKET_SIZE..];
        let challenge = (rest.len() >= CHALLENGE_UP_SIZE)
            .then(|| ChallengeUp::decode(&rest[rest.len() - CHALLENGE_UP_SIZE..]))
            .flatten();
        if challenge.is_some() {
            rest = &rest[..rest.len() - CHALLENGE_UP_SIZE];
        }
        let trade = (rest.len() >= TRADE_OUT_SIZE && !starts_with_link_magic(rest))
            .then(|| TradeOut::decode_le(rest));
        let link_at = if trade.is_some() { TRADE_OUT_SIZE } else { 0 };
        let link = rest.get(link_at..).and_then(LinkUp::decode);

        Ok(Self {
            magic: COOP_MAGIC, // both checked above (the packet is long enough for them)
            version: PROTOCOL_VERSION,
            room_code: raw_room,
            room_action: buf[12],
            requested_player_id: buf[13],
            allow_ahead: buf[14],
            target_party_size: buf[15],
            host_token: u32::from_le_bytes([buf[16], buf[17], buf[18], buf[19]]),
            badge_mask_local: u16::from_le_bytes([buf[20], buf[21]]),
            sequence: u32::from_le_bytes([buf[22], buf[23], buf[24], buf[25]]),
            heartbeat_rom: u32::from_le_bytes([buf[26], buf[27], buf[28], buf[29]]),
            challenge_rules: u16::from_le_bytes([buf[30], buf[31]]),
            local_player,
            cookie: u64::from_le_bytes(buf[96..104].try_into().expect("8 bytes")),
            player_secret: u64::from_le_bytes(buf[104..112].try_into().expect("8 bytes")),
            session: SessionOut {
                seq: buf[112],
                op: buf[113],
                kind: buf[114],
                target_trainer: u32::from_le_bytes([buf[116], buf[117], buf[118], buf[119]]),
            },
            trade,
            link,
            challenge,
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
    pub challenge_room_state: u8, // gates +7 (Milestone 5): bit 0 run lost
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
        buf[gates_base + 7] = self.challenge_room_state;
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
        self.encode_full(trade, None, None)
    }

    /// The fixed packet plus the recipient's trade, link and challenge
    /// trailers (the challenge trailer is always last).
    pub fn encode_full(
        &self,
        trade: Option<&TradeIn>,
        link: Option<&LinkDown>,
        challenge: Option<&ChallengeDown>,
    ) -> Vec<u8> {
        let mut buf = self.encode_le().to_vec();
        if let Some(trade) = trade {
            let mut trailer = [0u8; TRADE_IN_SIZE];
            trade.encode_le(&mut trailer);
            buf.extend_from_slice(&trailer);
        }
        if let Some(link) = link {
            link.encode(&mut buf);
        }
        if let Some(challenge) = challenge {
            buf.extend_from_slice(&challenge.encode());
        }
        buf
    }

    /// Compact rejection: the 16-byte header only (see `ERROR_PACKET_SIZE`).
    ///
    /// FROZEN across protocol versions: magic (0..4), the relay's protocol version (4..6) and the
    /// status (12) stay where they are, so a client of any version can read a relay's
    /// `ROOM_STATUS_ERR_PROTOCOL` and its version.
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

    #[test]
    fn another_protocol_version_is_named_before_the_size_is_checked() {
        // A future (or older) client's packet, of a size this version doesn't know.
        let mut pkt = vec![0u8; 40];
        pkt[0..4].copy_from_slice(&COOP_MAGIC.to_le_bytes());
        pkt[4..6].copy_from_slice(&(PROTOCOL_VERSION + 1).to_le_bytes());
        assert_eq!(
            ClientToServerPacket::decode_le(&pkt).unwrap_err(),
            ProtocolError::UnsupportedVersion(PROTOCOL_VERSION + 1)
        );
        // The answer names this relay's version in the frozen header.
        let reply = ServerToClientPacket::create_error_response(ROOM_STATUS_ERR_PROTOCOL);
        assert_eq!(reply.len(), ERROR_PACKET_SIZE);
        assert_eq!(u32::from_le_bytes(reply[0..4].try_into().unwrap()), COOP_MAGIC);
        assert_eq!(u16::from_le_bytes(reply[4..6].try_into().unwrap()), PROTOCOL_VERSION);
        assert_eq!(reply[12], ROOM_STATUS_ERR_PROTOCOL);
        // Too short to be anything: still a size error.
        assert!(matches!(
            ClientToServerPacket::decode_le(&pkt[..4]),
            Err(ProtocolError::PacketTooShort { .. })
        ));
    }

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
            boss_wait: 0,
            badge_mask: [0; 2],
            avatar_id: 2,
            anim: 3,
            field_busy: 1,
            challenge_flags: 0,
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
            challenge_rules: 0,
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
            link: None,
            challenge: None,
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

        // The largest packet a bridge sends (a trade, a full beacon and a full
        // message budget: five 192-byte frames) fits CLIENT_PACKET_MAX, which
        // sizes the relay's receive buffer. A 512-byte buffer once cut every
        // packet with two or more frames, and the link stalled.
        let frame = LinkMsg {
            kind: LINK_MSG_DATA,
            aid: 0,
            data: vec![0x5A; LINK_DATA_MAX],
        };
        let msgs = vec![frame; LINK_SEND_BUDGET / (4 + LINK_DATA_MAX)];
        let biggest = ClientToServerPacket {
            link: Some(LinkUp {
                epoch: 1,
                role: LINK_ROLE_PARENT,
                ack: 7,
                first_seq: 40,
                beacon: Some(LinkBeacon {
                    ggid: 0x0040_0318,
                    entry: 1,
                    seq: 3,
                    data: vec![0xA5; LINK_BEACON_MAX],
                }),
                msgs,
            }),
            ..with_trade
        };
        let full = biggest.encode_full();
        assert!(
            full.len() > 512 && full.len() <= CLIENT_PACKET_MAX,
            "{} bytes",
            full.len()
        );
        let decoded = ClientToServerPacket::decode_le(&full).expect("valid decode");
        assert_eq!(decoded.link.expect("link trailer").msgs.len(), 5);
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
            challenge_room_state: 0,
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
