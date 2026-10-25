use std::fmt;

pub const COOP_MAGIC: u32 = 0x434F_4F50; // 'COOP'
pub const PROTOCOL_VERSION: u16 = 3;
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
/// Reserved: the relay is game-agnostic and never sends this (mid-game saves may join).
pub const ROOM_STATUS_ERR_MIDGAME_SAVE: u8 = 6;
/// Not an error: echo `cookie` (bytes 16..24 of the reply) and retry.
pub const ROOM_STATUS_CHALLENGE: u8 = 7;
/// Too many rejected attempts from this IP; back off for a few seconds.
pub const ROOM_STATUS_ERR_RATE_LIMITED: u8 = 8;

// Wire Payload Sizes (in bytes)
pub const PLAYER_STATE_SIZE: usize = 32;
pub const FOLLOWER_STATE_SIZE: usize = 24;
pub const CLIENT_PACKET_SIZE: usize = 80;
pub const SERVER_PACKET_SIZE: usize =
    16 + (REMOTE_SLOTS * PLAYER_STATE_SIZE) + (REMOTE_SLOTS * FOLLOWER_STATE_SIZE) + 24; // 320 bytes
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

/// Matches `CoopPlayerState` (32 bytes) in `hg-engine/include/coop_sync.h`
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PlayerState {
    pub map_id: u16,         // 0x00
    pub x: i16,              // 0x02
    pub y: i16,              // 0x04
    pub prev_x: i16,         // 0x06
    pub prev_y: i16,         // 0x08
    pub direction: u8,       // 0x0A
    pub movement_type: u8,   // 0x0B
    pub sprite_id: u16,      // 0x0C
    pub animation_frame: u8, // 0x0E
    pub is_active: u8,       // 0x0F
    pub lead_species: u16,   // 0x10
    pub lead_form: u16,      // 0x12
    pub lead_is_shiny: u8,   // 0x14
    pub lead_gender: u8,     // 0x15
    pub map_npc_count: u8,   // 0x16
    pub client_ready: u8,    // 0x17 (computed by the ROM)
    pub trainer_id: u32,     // 0x18..0x1B (4-byte aligned)
    pub pad: [u8; 4],        // 0x1C..0x1F
}

impl PlayerState {
    pub fn encode_le(&self, out: &mut [u8]) {
        debug_assert!(out.len() >= PLAYER_STATE_SIZE);
        out[0..2].copy_from_slice(&self.map_id.to_le_bytes());
        out[2..4].copy_from_slice(&self.x.to_le_bytes());
        out[4..6].copy_from_slice(&self.y.to_le_bytes());
        out[6..8].copy_from_slice(&self.prev_x.to_le_bytes());
        out[8..10].copy_from_slice(&self.prev_y.to_le_bytes());
        out[10] = self.direction;
        out[11] = self.movement_type;
        out[12..14].copy_from_slice(&self.sprite_id.to_le_bytes());
        out[14] = self.animation_frame;
        out[15] = self.is_active;
        out[16..18].copy_from_slice(&self.lead_species.to_le_bytes());
        out[18..20].copy_from_slice(&self.lead_form.to_le_bytes());
        out[20] = self.lead_is_shiny;
        out[21] = self.lead_gender;
        out[22] = self.map_npc_count;
        out[23] = self.client_ready;
        out[24..28].copy_from_slice(&self.trainer_id.to_le_bytes());
        out[28..32].copy_from_slice(&self.pad);
    }

    pub fn decode_le(buf: &[u8]) -> Result<Self, ProtocolError> {
        if buf.len() < PLAYER_STATE_SIZE {
            return Err(ProtocolError::PacketTooShort {
                expected: PLAYER_STATE_SIZE,
                actual: buf.len(),
            });
        }
        let mut pad = [0u8; 4];
        pad.copy_from_slice(&buf[28..32]);

        Ok(Self {
            map_id: u16::from_le_bytes([buf[0], buf[1]]),
            x: i16::from_le_bytes([buf[2], buf[3]]),
            y: i16::from_le_bytes([buf[4], buf[5]]),
            prev_x: i16::from_le_bytes([buf[6], buf[7]]),
            prev_y: i16::from_le_bytes([buf[8], buf[9]]),
            direction: buf[10],
            movement_type: buf[11],
            sprite_id: u16::from_le_bytes([buf[12], buf[13]]),
            animation_frame: buf[14],
            is_active: buf[15],
            lead_species: u16::from_le_bytes([buf[16], buf[17]]),
            lead_form: u16::from_le_bytes([buf[18], buf[19]]),
            lead_is_shiny: buf[20],
            lead_gender: buf[21],
            map_npc_count: buf[22],
            client_ready: buf[23],
            trainer_id: u32::from_le_bytes([buf[24], buf[25], buf[26], buf[27]]),
            pad,
        })
    }
}

/// Matches `CoopFollowerState` (24 bytes) in `hg-engine/include/coop_sync.h`
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct FollowerState {
    pub species: u16,   // 0x00
    pub form: u16,      // 0x02
    pub is_shiny: u8,   // 0x04
    pub gender: u8,     // 0x05
    pub x: i16,         // 0x06
    pub y: i16,         // 0x08
    pub direction: u8,  // 0x0A
    pub is_visible: u8, // 0x0B
    pub pad: [u8; 12],  // 0x0C..0x17
}

impl FollowerState {
    pub fn encode_le(&self, out: &mut [u8]) {
        debug_assert!(out.len() >= FOLLOWER_STATE_SIZE);
        out[0..2].copy_from_slice(&self.species.to_le_bytes());
        out[2..4].copy_from_slice(&self.form.to_le_bytes());
        out[4] = self.is_shiny;
        out[5] = self.gender;
        out[6..8].copy_from_slice(&self.x.to_le_bytes());
        out[8..10].copy_from_slice(&self.y.to_le_bytes());
        out[10] = self.direction;
        out[11] = self.is_visible;
        out[12..24].copy_from_slice(&self.pad);
    }

    pub fn decode_le(buf: &[u8]) -> Result<Self, ProtocolError> {
        if buf.len() < FOLLOWER_STATE_SIZE {
            return Err(ProtocolError::PacketTooShort {
                expected: FOLLOWER_STATE_SIZE,
                actual: buf.len(),
            });
        }
        let mut pad = [0u8; 12];
        pad.copy_from_slice(&buf[12..24]);

        Ok(Self {
            species: u16::from_le_bytes([buf[0], buf[1]]),
            form: u16::from_le_bytes([buf[2], buf[3]]),
            is_shiny: buf[4],
            gender: buf[5],
            x: i16::from_le_bytes([buf[6], buf[7]]),
            y: i16::from_le_bytes([buf[8], buf[9]]),
            direction: buf[10],
            is_visible: buf[11],
            pad,
        })
    }
}

/// Sent by BizHawk C# Plugin -> Rust Relay Server (64 bytes)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClientToServerPacket {
    pub magic: u32,                // 0..4
    pub version: u16,              // 4..6
    pub room_code: [u8; 6],        // 6..12
    pub room_action: u8,           // 12 (1 = Create, 2 = Join, 3 = Sync, 4 = Leave)
    pub requested_player_id: u8,   // 13 (0..5 or 0xFF)
    pub reserved_14: u8,           // 14 (readiness travels in local_player.client_ready)
    pub target_party_size: u8,     // 15 (host's room size, 2..6; only honored from the host)
    pub host_token: u32,           // 16..20
    pub badge_mask_local: u16,     // 20..22
    pub sequence: u32,             // 22..26
    pub heartbeat_rom: u32,        // 26..30
    pub reserved: u16,             // 30..32
    pub local_player: PlayerState, // 32..64 (32 bytes)
    pub cookie: u64,               // 64..72 (0 until the relay issues one)
    pub player_secret: u64,        // 72..80 (random per bridge install, never 0)
}

impl ClientToServerPacket {
    pub fn encode_le(&self) -> [u8; CLIENT_PACKET_SIZE] {
        let mut buf = [0u8; CLIENT_PACKET_SIZE];
        buf[0..4].copy_from_slice(&self.magic.to_le_bytes());
        buf[4..6].copy_from_slice(&self.version.to_le_bytes());
        buf[6..12].copy_from_slice(&self.room_code);
        buf[12] = self.room_action;
        buf[13] = self.requested_player_id;
        buf[14] = self.reserved_14;
        buf[15] = self.target_party_size;
        buf[16..20].copy_from_slice(&self.host_token.to_le_bytes());
        buf[20..22].copy_from_slice(&self.badge_mask_local.to_le_bytes());
        buf[22..26].copy_from_slice(&self.sequence.to_le_bytes());
        buf[26..30].copy_from_slice(&self.heartbeat_rom.to_le_bytes());
        buf[30..32].copy_from_slice(&self.reserved.to_le_bytes());
        self.local_player.encode_le(&mut buf[32..64]);
        buf[64..72].copy_from_slice(&self.cookie.to_le_bytes());
        buf[72..80].copy_from_slice(&self.player_secret.to_le_bytes());
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

        let local_player = PlayerState::decode_le(&buf[32..64])?;

        Ok(Self {
            magic,
            version,
            room_code: raw_room,
            room_action: buf[12],
            requested_player_id: buf[13],
            reserved_14: buf[14],
            target_party_size: buf[15],
            host_token: u32::from_le_bytes([buf[16], buf[17], buf[18], buf[19]]),
            badge_mask_local: u16::from_le_bytes([buf[20], buf[21]]),
            sequence: u32::from_le_bytes([buf[22], buf[23], buf[24], buf[25]]),
            heartbeat_rom: u32::from_le_bytes([buf[26], buf[27], buf[28], buf[29]]),
            reserved: u16::from_le_bytes([buf[30], buf[31]]),
            local_player,
            cookie: u64::from_le_bytes(buf[64..72].try_into().expect("8 bytes")),
            player_secret: u64::from_le_bytes(buf[72..80].try_into().expect("8 bytes")),
        })
    }
}

/// Sent by Rust Relay Server -> BizHawk C# Plugin (320 bytes)
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
    pub remote_followers: [FollowerState; REMOTE_SLOTS],
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
    pub gates_pad: [u8; 12],
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

        let followers_base = 16 + REMOTE_SLOTS * PLAYER_STATE_SIZE;
        for (i, follower) in self.remote_followers.iter().enumerate() {
            let start = followers_base + i * FOLLOWER_STATE_SIZE;
            follower.encode_le(&mut buf[start..start + FOLLOWER_STATE_SIZE]);
        }

        let gates_base = followers_base + REMOTE_SLOTS * FOLLOWER_STATE_SIZE;
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
        buf[gates_base + 12..gates_base + 24].copy_from_slice(&self.gates_pad);

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

    #[test]
    fn test_client_packet_roundtrip() {
        let pkt = ClientToServerPacket {
            magic: COOP_MAGIC,
            version: PROTOCOL_VERSION,
            room_code: *b"JOHTO1",
            room_action: ROOM_ACTION_CREATE,
            requested_player_id: AUTO_PLAYER_ID,
            reserved_14: 0,
            target_party_size: 4,
            host_token: 0x1234_5678,
            badge_mask_local: 0b0000_0011,
            sequence: 512,
            heartbeat_rom: 1024,
            reserved: 0,
            local_player: PlayerState {
                map_id: 4,
                x: 12,
                y: 25,
                prev_x: 12,
                prev_y: 24,
                direction: 0,
                movement_type: 1,
                sprite_id: 99,
                animation_frame: 2,
                is_active: 1,
                lead_species: 155,
                lead_form: 0,
                lead_is_shiny: 1,
                lead_gender: 0,
                map_npc_count: 14,
                client_ready: 1,
                trainer_id: 0xA1B2_C3D4,
                pad: [0u8; 4],
            },
            cookie: 0x0102_0304_0506_0708,
            player_secret: 0x1122_3344_5566_7788,
        };

        let bytes = pkt.encode_le();
        let decoded = ClientToServerPacket::decode_le(&bytes).expect("valid decode");
        assert_eq!(pkt, decoded);
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
        let st = PlayerState {
            map_id: 0x0102,
            sprite_id: 0x0304,
            lead_species: 0x0506,
            client_ready: 0x07,
            trainer_id: 0x0809_0A0B,
            ..PlayerState::default()
        };
        let mut buf = [0u8; PLAYER_STATE_SIZE];
        st.encode_le(&mut buf);
        assert_eq!(&buf[0x00..0x02], &[0x02, 0x01]);
        assert_eq!(&buf[0x0C..0x0E], &[0x04, 0x03]);
        assert_eq!(&buf[0x10..0x12], &[0x06, 0x05]);
        assert_eq!(buf[0x17], 0x07);
        assert_eq!(&buf[0x18..0x1C], &[0x0B, 0x0A, 0x09, 0x08]);
        // The bridge copies packet[16..320] to mailbox 0x030..0x15F, so the
        // gates block (mailbox 0x148) must start at packet offset 0x148 - 0x30 + 16.
        let gates_packet_offset =
            16 + REMOTE_SLOTS * PLAYER_STATE_SIZE + REMOTE_SLOTS * FOLLOWER_STATE_SIZE;
        assert_eq!(gates_packet_offset, 0x148 - 0x30 + 16);
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
