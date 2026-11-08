//! Where things are in the mailbox and the blocks it points to, from `goldlink-abi` (the ROM's
//! own headers) instead of the C# bridge's hand-copied constants (RFC_DELTA §5: MaxActivity
//! drifted). Offsets are relative to the mailbox, or to the block named.

#![allow(dead_code)]

use std::mem::{offset_of as of, size_of};

use goldlink_abi::{
    CoopActivity, CoopChallengeSync, CoopCommBlock as B, CoopFieldEntryHeader,
    CoopPlayerState as P, CoopProgressionGates as G, CoopSessionKind, CoopSessionReason,
    CoopSessionState, CoopTradeExchange, CoopTradeIn,
};

pub const MAILBOX_SIZE: usize = size_of::<B>();

pub const MAGIC: usize = of!(B, magic);
pub const VERSION: usize = of!(B, version);
pub const ASSIGNED_PLAYER_ID: usize = of!(B, assigned_player_id);
pub const ACTIVE_PLAYERS_MASK: usize = of!(B, active_players_mask);
pub const HEARTBEAT_ROM: usize = of!(B, heartbeat_rom);
pub const SERVER_TICK: usize = of!(B, server_tick);
pub const LOCAL_PLAYER: usize = of!(B, local_player);
/// The relay's payload lands here: the five remote players, then the gates.
pub const SERVER_PAYLOAD: usize = of!(B, remote_players);
pub const SERVER_PAYLOAD_SIZE: usize = REMOTE_SLOTS * PLAYER_STATE_SIZE + GATES_SIZE;
pub const GATES: usize = of!(B, gates);
pub const IS_CONNECTED_TO_ROOM: usize = of!(B, is_connected_to_room);
pub const WAS_IN_ACTIVE_SESSION: usize = of!(B, was_in_active_session);
pub const DISCONNECT_STATE: usize = of!(B, disconnect_state);
pub const BADGE_MASK_LOCAL: usize = of!(B, badge_mask_local);
pub const ROOM_REJECTION: usize = of!(B, room_rejection);
pub const HEARTBEAT_BRIDGE: usize = of!(B, heartbeat_bridge);
pub const LOCAL_PLAYER_SEQ: usize = of!(B, local_player_seq);
pub const LOCAL_AVATAR: usize = of!(B, local_avatar);
pub const LIMIT_MAP_ID: usize = of!(B, limit_map_id);
pub const LIMIT_SPECIES: usize = of!(B, limit_species);
pub const DIAG: usize = of!(B, diag);
pub const DEBUG_CMD: usize = of!(B, debug_cmd);
pub const DEBUG_SEQ: usize = of!(B, debug_seq);
pub const DEBUG_ARG: usize = of!(B, debug_arg);
pub const DEBUG_RESULT: usize = of!(B, debug_result);
pub const SAVE_COUNTER: usize = of!(B, save_counter);
pub const SAVE_SLOT_READY: usize = of!(B, save_slot_ready);
pub const RESET_REQUEST: usize = of!(B, reset_request);
pub const SESSION_OUT: usize = of!(B, session_out);
pub const SESSION_OUT_SIZE: usize = 8;
pub const EXCHANGE_PTR: usize = of!(B, exchange_ptr);

// CoopPlayerState (64 bytes, follower included).
pub const PLAYER_STATE_SIZE: usize = size_of::<P>();
pub const PS_MAP_ID: usize = of!(P, map_id);
pub const PS_AVATAR_STATE: usize = of!(P, avatar_state);
pub const PS_X: usize = of!(P, x);
pub const PS_Z: usize = of!(P, z);
pub const PS_DIRECTION: usize = of!(P, direction);
pub const PS_GENDER: usize = of!(P, gender);
pub const PS_MOVE_ACTION: usize = of!(P, move_action);
pub const PS_IS_ACTIVE: usize = of!(P, is_active);
pub const PS_CLIENT_READY: usize = of!(P, client_ready);
pub const PS_SAVE_STATUS: usize = of!(P, save_status);
pub const PS_ACTIVITY: usize = of!(P, activity);
pub const PS_TRAINER_ID: usize = of!(P, trainer_id);
pub const PS_NAME: usize = of!(P, name);
pub const PS_FOLLOWER_SPECIES: usize = of!(P, follower_species);
pub const PS_FOLLOWER_SHINY: usize = of!(P, follower_shiny);
pub const PS_FOLLOWER_GENDER: usize = of!(P, follower_gender);
pub const PS_FOLLOWER_VISIBLE: usize = of!(P, follower_visible);
pub const PS_FOLLOWER_DIRECTION: usize = of!(P, follower_direction);
pub const PS_FOLLOWER_MOVE_ACTION: usize = of!(P, follower_move_action);
pub const PS_AVATAR_ID: usize = of!(P, avatar_id);
pub const PS_ANIM: usize = of!(P, anim);
pub const PS_FIELD_BUSY: usize = of!(P, field_busy);
pub const PS_CHALLENGE_FLAGS: usize = of!(P, challenge_flags);

/// Highest valid values, as the ROM's own sanitizer (coop_remote.c) has them.
pub const MAX_ANIM: u8 = 15;
/// `MapObject_SetHeldMovement`'s limit.
pub const MAX_HELD_MOVEMENT: u8 = 0x71;
pub const MOVE_ACTION_NONE: u8 = 0xFF;
pub const MAX_AVATAR_STATE: u8 = 14;
pub const MAX_ACTIVITY: u8 = CoopActivity::COOP_ACTIVITY_EVENT as u8;
pub const CHALLENGE_FLAGS_MASK: u8 = goldlink_proto::CHALLENGE_FLAG_SPIRIT;

// CoopProgressionGates (24 bytes), relative to the gates.
pub const GATES_SIZE: usize = size_of::<G>();
pub const GATE_SYNC_READY: usize = of!(G, flag_coop_sync_ready);
pub const GATE_CHECKPOINT: usize = of!(G, current_checkpoint);
pub const GATE_MONOLOCKE: usize = of!(G, assigned_monolocke);
pub const GATE_CHALLENGE_ROOM_STATE: usize = of!(G, challenge_room_state);
pub const GATE_CAMPAIGN_STARTED: usize = of!(G, flag_campaign_started);
pub const GATE_EXPECTED_PARTY: usize = of!(G, expected_party_size);
pub const GATE_ONLINE_PARTY: usize = of!(G, online_party_count);
pub const GATE_READY_PARTY: usize = of!(G, ready_party_count);
pub const GATE_ROOM_UID: usize = of!(G, room_uid);
pub const GATE_TEAM_PARTNER: usize = of!(G, team_partner_slot);
pub const GATE_TEAMED_MASK: usize = of!(G, teamed_mask);
pub const GATE_SESSION_STATE: usize = of!(G, session_state);
pub const GATE_SESSION_KIND: usize = of!(G, session_kind);
pub const GATE_SESSION_PEER: usize = of!(G, session_peer_slot);
pub const GATE_SESSION_REASON: usize = of!(G, session_reason);
/// team_partner_slot .. session_reason.
pub const GATE_SESSION_SIZE: usize = GATE_SESSION_REASON + 1 - GATE_TEAM_PARTNER;
pub const GATE_TRADE_PENDING: usize = of!(G, trade_pending);
pub const MAX_SESSION_STATE: u8 = CoopSessionState::COOP_SESSION_STATE_MAX as u8;
pub const MAX_SESSION_KIND: u8 = CoopSessionKind::COOP_SESSION_KIND_REVIVE as u8;
pub const MAX_SESSION_REASON: u8 = CoopSessionReason::COOP_SESSION_REASON_TEAMMATE_AWAY as u8;

// The relay's payload, relative to its start.
pub const REMOTE_SLOTS: usize = goldlink_abi::COOP_REMOTE_SLOTS as usize;
pub const PAYLOAD_GATES: usize = REMOTE_SLOTS * PLAYER_STATE_SIZE;

pub const PLAYER_ID_NONE: u8 = goldlink_abi::COOP_PLAYER_ID_NONE as u8;
pub const MAX_PLAYERS: usize = goldlink_abi::COOP_MAX_PLAYERS as usize;

/// ARM9 address of main RAM's first byte.
pub const MAIN_RAM_BASE: u32 = 0x0200_0000;
pub const MAIN_RAM_SIZE: u32 = 0x0040_0000;

// CoopTradeExchange ("CTX1"): the ROM's `out` (to the relay) and `in` (from it).
pub const EXCHANGE_MAGIC: u32 = goldlink_abi::COOP_EXCHANGE_MAGIC;
pub const EXCHANGE_VERSION: u16 = goldlink_abi::COOP_EXCHANGE_VERSION as u16;
pub const EXCHANGE_SIZE: usize = size_of::<CoopTradeExchange>();
pub const EXCHANGE_OUT: usize = of!(CoopTradeExchange, out);
pub const EXCHANGE_IN: usize = of!(CoopTradeExchange, in_);
pub const TRADE_OUT_SIZE: usize = goldlink_proto::TRADE_OUT_SIZE;
pub const TRADE_IN_SIZE: usize = goldlink_proto::TRADE_IN_SIZE;
pub const TRADE_IN_STATE: usize = of!(CoopTradeIn, state);
pub const TRADE_IN_REASON: usize = of!(CoopTradeIn, reason);
pub const TRADE_IN_PEER_SLOT: usize = of!(CoopTradeIn, peer_slot);
pub const TRADE_IN_STATE_MAX: u8 = goldlink_proto::TRADE_IN_ABORTED;
pub const TRADE_IN_REASON_MAX: u8 = goldlink_proto::TRADE_ABORT_TIMEOUT;

// Overlay 131's pinned header ("COF1") and the challenge sync block ("CHS1") it points to.
pub const FIELD_ENTRY_HEADER: usize =
    (goldlink_abi::COOP_FIELD_ENTRY_ADDR - MAIN_RAM_BASE) as usize;
pub const FIELD_ENTRY_MAGIC: u32 = goldlink_abi::COOP_FIELD_ENTRY_MAGIC;
pub const FIELD_ENTRY_CHALLENGE: usize = of!(CoopFieldEntryHeader, challenge);
pub const OVERLAY131_START: u32 = goldlink_abi::COOP_FIELD_ENTRY_ADDR;
/// Overlay 129 (the mailbox's) starts here.
pub const OVERLAY131_END: u32 = 0x023D_8000;
pub const CHALLENGE_SYNC_MAGIC: u32 = 0x3153_4843; // "CHS1"
pub const CHALLENGE_SYNC_VERSION: u16 = 1;
pub const CHALLENGE_SYNC_SIZE: usize = size_of::<CoopChallengeSync>();
pub const CHALLENGE_SYNC_OUT: usize = of!(CoopChallengeSync, out);
pub const CHALLENGE_SYNC_IN: usize = of!(CoopChallengeSync, in_);

/// Little-endian reads and writes on a byte slice (main RAM or a buffer).
pub fn u16_at(b: &[u8], at: usize) -> u16 {
    u16::from_le_bytes([b[at], b[at + 1]])
}

pub fn u32_at(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]])
}

pub fn put_u32(b: &mut [u8], at: usize, v: u32) {
    b[at..at + 4].copy_from_slice(&v.to_le_bytes());
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The C# bridge's constants (CoopBridgeForm.cs `Mailbox`), checked against the ROM's
    /// headers: the port reads the same bytes.
    #[test]
    fn matches_the_csharp_bridge() {
        assert_eq!(MAILBOX_SIZE, 512);
        assert_eq!(
            [
                ASSIGNED_PLAYER_ID,
                ACTIVE_PLAYERS_MASK,
                HEARTBEAT_ROM,
                SERVER_TICK,
                LOCAL_PLAYER
            ],
            [0x006, 0x007, 0x008, 0x00C, 0x010]
        );
        assert_eq!(
            (SERVER_PAYLOAD, SERVER_PAYLOAD_SIZE, GATES),
            (0x050, 344, 0x190)
        );
        assert_eq!(
            [
                IS_CONNECTED_TO_ROOM,
                WAS_IN_ACTIVE_SESSION,
                DISCONNECT_STATE,
                BADGE_MASK_LOCAL,
                ROOM_REJECTION
            ],
            [0x1A8, 0x1AA, 0x1AB, 0x1AC, 0x1AE]
        );
        assert_eq!(
            [HEARTBEAT_BRIDGE, LOCAL_PLAYER_SEQ, LOCAL_AVATAR],
            [0x1B0, 0x1B4, 0x1BE]
        );
        assert_eq!([LIMIT_MAP_ID, LIMIT_SPECIES, DIAG], [0x1C0, 0x1C2, 0x1C4]);
        assert_eq!(
            [DEBUG_CMD, DEBUG_SEQ, DEBUG_ARG, DEBUG_RESULT],
            [0x1DC, 0x1DD, 0x1DE, 0x1DF]
        );
        assert_eq!(
            [SAVE_COUNTER, SAVE_SLOT_READY, RESET_REQUEST],
            [0x1E0, 0x1E4, 0x1E5]
        );
        assert_eq!([SESSION_OUT, EXCHANGE_PTR], [0x1F0, 0x1FC]);
        assert_eq!(
            [
                EXCHANGE_SIZE,
                EXCHANGE_OUT,
                TRADE_OUT_SIZE,
                EXCHANGE_IN,
                TRADE_IN_SIZE
            ],
            [0x1F8, 0x008, 244, 0x0FC, 252]
        );
        assert_eq!(
            (TRADE_IN_STATE_MAX, TRADE_IN_REASON_MAX, TRADE_IN_PEER_SLOT),
            (4, 3, 5)
        );
        assert_eq!(
            (FIELD_ENTRY_HEADER, FIELD_ENTRY_MAGIC, FIELD_ENTRY_CHALLENGE),
            (0x3C4000, 0x31464F43, 0x0C)
        );
        assert_eq!(
            (CHALLENGE_SYNC_SIZE, CHALLENGE_SYNC_OUT, CHALLENGE_SYNC_IN),
            (0x78, 0x08, 0x38)
        );
        assert_eq!(PLAYER_STATE_SIZE, 64);
        assert_eq!(
            [
                PS_MAP_ID,
                PS_AVATAR_STATE,
                PS_X,
                PS_Z,
                PS_DIRECTION,
                PS_GENDER,
                PS_MOVE_ACTION
            ],
            [0x00, 0x03, 0x04, 0x06, 0x0A, 0x0B, 0x0E]
        );
        assert_eq!(
            [
                PS_IS_ACTIVE,
                PS_CLIENT_READY,
                PS_SAVE_STATUS,
                PS_ACTIVITY,
                PS_TRAINER_ID,
                PS_NAME
            ],
            [0x10, 0x11, 0x12, 0x13, 0x14, 0x18]
        );
        assert_eq!(
            [
                PS_FOLLOWER_SPECIES,
                PS_FOLLOWER_SHINY,
                PS_FOLLOWER_GENDER,
                PS_FOLLOWER_VISIBLE
            ],
            [0x28, 0x2D, 0x2E, 0x2F]
        );
        assert_eq!(
            [
                PS_FOLLOWER_DIRECTION,
                PS_FOLLOWER_MOVE_ACTION,
                PS_AVATAR_ID,
                PS_ANIM,
                PS_FIELD_BUSY,
                PS_CHALLENGE_FLAGS
            ],
            [0x36, 0x37, 0x3C, 0x3D, 0x3E, 0x3F]
        );
        assert_eq!(GATES_SIZE, 24);
        assert_eq!(
            [
                GATE_SYNC_READY,
                GATE_CHECKPOINT,
                GATE_MONOLOCKE,
                GATE_CHALLENGE_ROOM_STATE,
                GATE_CAMPAIGN_STARTED
            ],
            [0x04, 0x05, 0x06, 0x07, 0x08]
        );
        assert_eq!(
            [
                GATE_EXPECTED_PARTY,
                GATE_ONLINE_PARTY,
                GATE_READY_PARTY,
                GATE_ROOM_UID
            ],
            [0x09, 0x0A, 0x0B, 0x0C]
        );
        assert_eq!(
            [
                GATE_TEAM_PARTNER,
                GATE_TEAMED_MASK,
                GATE_SESSION_STATE,
                GATE_SESSION_KIND,
                GATE_SESSION_PEER,
                GATE_SESSION_REASON
            ],
            [0x10, 0x11, 0x12, 0x13, 0x14, 0x16]
        );
        assert_eq!((GATE_SESSION_SIZE, GATE_TRADE_PENDING), (7, 0x17));
        assert_eq!(
            (MAX_SESSION_STATE, MAX_SESSION_KIND, MAX_SESSION_REASON),
            (8, 6, 6)
        );
        assert_eq!(
            (MAX_ACTIVITY, MAX_AVATAR_STATE, MAX_ANIM, MAX_HELD_MOVEMENT),
            (5, 14, 15, 0x71)
        );
        assert_eq!(PAYLOAD_GATES, 320);
    }
}
