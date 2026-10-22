use std::fmt;

pub const COOP_MAGIC: u32 = 0x434F_4F50; // 'COOP'
pub const PROTOCOL_VERSION: u16 = 1;
pub const MAX_PLAYERS: usize = 6;
pub const REMOTE_SLOTS: usize = MAX_PLAYERS - 1; // 5 remote slots per client
pub const AUTO_PLAYER_ID: u8 = 0xFF;
pub const DEFAULT_ROOM_CODE: [u8; 6] = *b"JOHTO1";

/// Wire size constants (in bytes)
pub const PLAYER_STATE_SIZE: usize = 32;
pub const FOLLOWER_STATE_SIZE: usize = 24;
pub const CLIENT_PACKET_SIZE: usize = 56;
pub const SERVER_PACKET_SIZE: usize =
    16 + (REMOTE_SLOTS * PLAYER_STATE_SIZE) + (REMOTE_SLOTS * FOLLOWER_STATE_SIZE) + 24; // 320 bytes

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
                write!(f, "packet too short: expected {expected} bytes, got {actual}")
            }
            Self::InvalidMagic(m) => write!(f, "invalid magic: 0x{m:08X} (expected 0x{COOP_MAGIC:08X})"),
            Self::UnsupportedVersion(v) => write!(f, "unsupported protocol version: {v}"),
        }
    }
}

impl std::error::Error for ProtocolError {}

/// normalizes a 6-byte room code into uppercase ASCII alphanumeric bytes
pub fn sanitize_room_code(raw: [u8; 6]) -> [u8; 6] {
    let mut out = DEFAULT_ROOM_CODE;
    for (i, b) in raw.into_iter().enumerate() {
        let c = (b as char).to_ascii_uppercase();
        if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
            out[i] = c as u8;
        } else if b == 0 {
            out[i] = b'_';
        }
    }
    out
}

pub fn room_code_to_string(code: &[u8; 6]) -> String {
    String::from_utf8_lossy(code).trim_end_matches('_').to_string()
}

/// Matches `struct CoopPlayerState` (32 bytes) in `include/coop_sync.h`
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PlayerState {
    pub map_id: u16,          // 0x00
    pub x: i16,               // 0x02
    pub y: i16,               // 0x04
    pub prev_x: i16,          // 0x06
    pub prev_y: i16,          // 0x08
    pub direction: u8,        // 0x0A
    pub movement_type: u8,    // 0x0B
    pub sprite_id: u16,       // 0x0C
    pub animation_frame: u8,  // 0x0E
    pub is_active: u8,        // 0x0F
    pub lead_species: u16,    // 0x10
    pub lead_form: u16,       // 0x12
    pub lead_is_shiny: u8,    // 0x14
    pub lead_gender: u8,      // 0x15
    pub map_npc_count: u8,    // 0x16
    pub client_ready: u8,     // 0x17
    pub trainer_id: u32,      // 0x18..0x1B (4-byte aligned)
    pub pad: [u8; 4],         // 0x1C..0x1F
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

/// Matches `struct CoopFollowerState` (24 bytes)
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct FollowerState {
    pub species: u16,
    pub form: u16,
    pub is_shiny: u8,
    pub gender: u8,
    pub x: i16,
    pub y: i16,
    pub direction: u8,
    pub is_visible: u8,
    pub pad: [u8; 12],
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

/// Sent by BizHawk C# Plugin -> Rust Relay Server (56 bytes)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClientToServerPacket {
    pub magic: u32,              // 0..4
    pub version: u16,            // 4..6
    pub room_code: [u8; 6],      // 6..12 (6-char ASCII room identifier, e.g. "JOHTO1")
    pub requested_player_id: u8, // 12 (0..5 or 0xFF for auto)
    pub client_ready: u8,        // 13 (1 = Ready to lock party & start adventure)
    pub badge_mask_local: u16,   // 14..16
    pub sequence: u32,           // 16..20
    pub heartbeat_rom: u32,      // 20..24
    pub local_player: PlayerState, // 24..56 (32 bytes)
}

impl ClientToServerPacket {
    pub fn encode_le(&self) -> [u8; CLIENT_PACKET_SIZE] {
        let mut buf = [0u8; CLIENT_PACKET_SIZE];
        buf[0..4].copy_from_slice(&self.magic.to_le_bytes());
        buf[4..6].copy_from_slice(&self.version.to_le_bytes());
        buf[6..12].copy_from_slice(&sanitize_room_code(self.room_code));
        buf[12] = self.requested_player_id;
        buf[13] = self.client_ready;
        buf[14..16].copy_from_slice(&self.badge_mask_local.to_le_bytes());
        buf[16..20].copy_from_slice(&self.sequence.to_le_bytes());
        buf[20..24].copy_from_slice(&self.heartbeat_rom.to_le_bytes());
        self.local_player.encode_le(&mut buf[24..56]);
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

        let mut local_player = PlayerState::decode_le(&buf[24..56])?;
        let client_ready = buf[13];
        local_player.client_ready = client_ready;

        Ok(Self {
            magic,
            version,
            room_code: sanitize_room_code(raw_room),
            requested_player_id: buf[12],
            client_ready,
            badge_mask_local: u16::from_le_bytes([buf[14], buf[15]]),
            sequence: u32::from_le_bytes([buf[16], buf[17], buf[18], buf[19]]),
            heartbeat_rom: u32::from_le_bytes([buf[20], buf[21], buf[22], buf[23]]),
            local_player,
        })
    }
}

/// Sent by Rust Relay Server -> BizHawk C# Plugin at 30 Hz (320 bytes)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ServerToClientPacket {
    pub magic: u32,
    pub version: u16,
    pub assigned_player_id: u8,
    pub active_players_mask: u8,
    pub server_tick: u32,
    pub reserved_header: u32,
    pub remote_players: [PlayerState; REMOTE_SLOTS],
    pub remote_followers: [FollowerState; REMOTE_SLOTS],
    // Progression Gates (24 bytes -> 0x148..0x15F in gCoopCommBlock)
    pub badge_mask_local_echo: u16, // 0x148
    pub badge_mask_synced: u16,     // 0x14A
    pub flag_coop_sync_ready: u8,   // 0x14C
    pub current_checkpoint: u8,     // 0x14D
    pub assigned_monolocke: u8,     // 0x14E
    pub nuzlocke_failed: u8,        // 0x14F
    pub flag_campaign_started: u8,  // 0x150 (1 = Bedroom Start Gate unlocked, roster locked)
    pub expected_party_size: u8,    // 0x151 (Locked party count from start of run)
    pub online_party_count: u8,     // 0x152 (Currently connected players)
    pub ready_party_count: u8,      // 0x153 (Players who checked Ready)
    pub gates_pad: [u8; 12],        // 0x154..0x15F
}

impl ServerToClientPacket {
    pub fn encode_le(&self) -> [u8; SERVER_PACKET_SIZE] {
        let mut buf = [0u8; SERVER_PACKET_SIZE];
        buf[0..4].copy_from_slice(&self.magic.to_le_bytes());
        buf[4..6].copy_from_slice(&self.version.to_le_bytes());
        buf[6] = self.assigned_player_id;
        buf[7] = self.active_players_mask;
        buf[8..12].copy_from_slice(&self.server_tick.to_le_bytes());
        buf[12..16].copy_from_slice(&self.reserved_header.to_le_bytes());

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
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_client_packet_roundtrip_with_room_and_trainer_id() {
        let pkt = ClientToServerPacket {
            magic: COOP_MAGIC,
            version: PROTOCOL_VERSION,
            room_code: *b"JOHTO1",
            requested_player_id: AUTO_PLAYER_ID,
            client_ready: 1,
            badge_mask_local: 0b0000_0011,
            sequence: 512,
            heartbeat_rom: 1024,
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
        };

        let bytes = pkt.encode_le();
        let decoded = ClientToServerPacket::decode_le(&bytes).expect("valid decode");
        assert_eq!(pkt, decoded);
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
