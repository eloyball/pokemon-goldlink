use std::fmt;

pub const COOP_MAGIC: u32 = 0x434F_4F50; // 'COOP' in ASCII
pub const PROTOCOL_VERSION: u16 = 1;
pub const MAX_PLAYERS: usize = 6;
pub const REMOTE_SLOTS: usize = MAX_PLAYERS - 1; // 5 remote slots per client
pub const AUTO_PLAYER_ID: u8 = 0xFF;

/// Wire size constants (in bytes)
pub const PLAYER_STATE_SIZE: usize = 32;
pub const FOLLOWER_STATE_SIZE: usize = 24;
pub const CLIENT_PACKET_SIZE: usize = 16 + PLAYER_STATE_SIZE + 8; // 56 bytes
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

/// Matches `struct CoopPlayerState` (32 bytes) in `docs/coop_memory_spec.md`
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PlayerState {
    pub map_id: u16,
    pub x: i16,
    pub y: i16,
    pub prev_x: i16,
    pub prev_y: i16,
    pub direction: u8,
    pub movement_type: u8,
    pub sprite_id: u16,
    pub animation_frame: u8,
    pub is_active: u8,
    pub pad: [u8; 16],
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
        out[16..32].copy_from_slice(&self.pad);
    }

    pub fn decode_le(buf: &[u8]) -> Result<Self, ProtocolError> {
        if buf.len() < PLAYER_STATE_SIZE {
            return Err(ProtocolError::PacketTooShort {
                expected: PLAYER_STATE_SIZE,
                actual: buf.len(),
            });
        }
        let mut pad = [0u8; 16];
        pad.copy_from_slice(&buf[16..32]);

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
            pad,
        })
    }
}

/// Matches `struct CoopFollowerState` (24 bytes) in `docs/coop_memory_spec.md`
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
    pub magic: u32,
    pub version: u16,
    pub requested_player_id: u8, // 0..5, or 0xFF for auto-assign
    pub map_npc_count: u8,       // Used for follower crowding checks
    pub sequence: u32,
    pub heartbeat_rom: u32,
    pub local_player: PlayerState,
    // Lead follower metadata & local badge progression
    pub lead_species: u16,
    pub lead_form: u16,
    pub lead_is_shiny: u8,
    pub lead_gender: u8,
    pub badge_mask_local: u16,
}

impl ClientToServerPacket {
    pub fn encode_le(&self) -> [u8; CLIENT_PACKET_SIZE] {
        let mut buf = [0u8; CLIENT_PACKET_SIZE];
        buf[0..4].copy_from_slice(&self.magic.to_le_bytes());
        buf[4..6].copy_from_slice(&self.version.to_le_bytes());
        buf[6] = self.requested_player_id;
        buf[7] = self.map_npc_count;
        buf[8..12].copy_from_slice(&self.sequence.to_le_bytes());
        buf[12..16].copy_from_slice(&self.heartbeat_rom.to_le_bytes());
        self.local_player.encode_le(&mut buf[16..48]);
        buf[48..50].copy_from_slice(&self.lead_species.to_le_bytes());
        buf[50..52].copy_from_slice(&self.lead_form.to_le_bytes());
        buf[52] = self.lead_is_shiny;
        buf[53] = self.lead_gender;
        buf[54..56].copy_from_slice(&self.badge_mask_local.to_le_bytes());
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

        Ok(Self {
            magic,
            version,
            requested_player_id: buf[6],
            map_npc_count: buf[7],
            sequence: u32::from_le_bytes([buf[8], buf[9], buf[10], buf[11]]),
            heartbeat_rom: u32::from_le_bytes([buf[12], buf[13], buf[14], buf[15]]),
            local_player: PlayerState::decode_le(&buf[16..48])?,
            lead_species: u16::from_le_bytes([buf[48], buf[49]]),
            lead_form: u16::from_le_bytes([buf[50], buf[51]]),
            lead_is_shiny: buf[52],
            lead_gender: buf[53],
            badge_mask_local: u16::from_le_bytes([buf[54], buf[55]]),
        })
    }
}

/// Sent by Rust Relay Server -> BizHawk C# Plugin at 30 Hz (320 bytes)
/// Maps directly to the writable regions of `gCoopCommBlock`
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
    pub badge_mask_local_echo: u16,
    pub badge_mask_synced: u16,
    pub flag_coop_sync_ready: u8,
    pub current_checkpoint: u8,
    pub assigned_monolocke: u8,
    pub nuzlocke_failed: u8,
    pub gates_pad: [u8; 16],
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

        // 0x010..0x0B0 in packet -> writes directly to 0x030..0x0CF in gCoopCommBlock
        for (i, player) in self.remote_players.iter().enumerate() {
            let start = 16 + i * PLAYER_STATE_SIZE;
            player.encode_le(&mut buf[start..start + PLAYER_STATE_SIZE]);
        }

        // 0x0B0..0x128 in packet -> writes directly to 0x0D0..0x147 in gCoopCommBlock
        let followers_base = 16 + REMOTE_SLOTS * PLAYER_STATE_SIZE;
        for (i, follower) in self.remote_followers.iter().enumerate() {
            let start = followers_base + i * FOLLOWER_STATE_SIZE;
            follower.encode_le(&mut buf[start..start + FOLLOWER_STATE_SIZE]);
        }

        // 0x128..0x140 in packet -> writes directly to 0x148..0x15F in gCoopCommBlock
        let gates_base = followers_base + REMOTE_SLOTS * FOLLOWER_STATE_SIZE;
        buf[gates_base..gates_base + 2].copy_from_slice(&self.badge_mask_local_echo.to_le_bytes());
        buf[gates_base + 2..gates_base + 4].copy_from_slice(&self.badge_mask_synced.to_le_bytes());
        buf[gates_base + 4] = self.flag_coop_sync_ready;
        buf[gates_base + 5] = self.current_checkpoint;
        buf[gates_base + 6] = self.assigned_monolocke;
        buf[gates_base + 7] = self.nuzlocke_failed;
        buf[gates_base + 8..gates_base + 24].copy_from_slice(&self.gates_pad);

        buf
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_client_packet_roundtrip() {
        let mut pad = [0u8; 16];
        pad[0] = 42;
        let pkt = ClientToServerPacket {
            magic: COOP_MAGIC,
            version: PROTOCOL_VERSION,
            requested_player_id: AUTO_PLAYER_ID,
            map_npc_count: 14,
            sequence: 1024,
            heartbeat_rom: 600,
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
                pad,
            },
            lead_species: 155, // Cyndaquil
            lead_form: 0,
            lead_is_shiny: 1,
            lead_gender: 0,
            badge_mask_local: 0b0000_0001_0000_0011,
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
