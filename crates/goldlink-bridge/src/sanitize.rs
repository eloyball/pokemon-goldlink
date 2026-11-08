//! What the relay sends is untrusted until checked (the C# bridge's `SanitizeServerPayload`,
//! `WriteTradeIn` and `TakeChallengeDown`). The ROM checks again when it reads
//! (`Coop_ReadRemotePlayer`); this keeps garbage from ever reaching emulated RAM.

use crate::layout::*;
use goldlink_proto::{
    CHALLENGE_DOWN_SIZE, CHALLENGE_MAGIC, CHALLENGE_RULES_VALID, CHALLENGE_VERSION, MONO_TYPE_NONE,
};

/// The ROM's own limits (published in the mailbox, v3+).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    /// `map_id < limit_map_id`.
    pub map_id: u16,
    /// `species <= limit_species`.
    pub species: u16,
}

pub fn is_valid_move_action(action: u8) -> bool {
    action < MAX_HELD_MOVEMENT || action == MOVE_ACTION_NONE
}

fn slot_ok(b: u8) -> bool {
    (b as usize) < REMOTE_SLOTS || b == PLAYER_ID_NONE
}

/// Copies the 344-byte payload and zeroes every remote player, follower or gates block that
/// holds out-of-range values.
pub fn server_payload(payload: &[u8], limits: Limits) -> [u8; SERVER_PAYLOAD_SIZE] {
    let mut p = [0u8; SERVER_PAYLOAD_SIZE];
    p.copy_from_slice(&payload[..SERVER_PAYLOAD_SIZE]);

    for i in 0..REMOTE_SLOTS {
        let o = i * PLAYER_STATE_SIZE;
        let map = u16_at(&p, o + PS_MAP_ID);
        let active = p[o + PS_IS_ACTIVE];
        let ok = active <= 1
            && map < limits.map_id
            && (active == 0 || map != 0)
            && p[o + PS_DIRECTION] <= 3
            && p[o + PS_GENDER] <= 1
            && p[o + PS_AVATAR_STATE] <= MAX_AVATAR_STATE
            && is_valid_move_action(p[o + PS_MOVE_ACTION])
            && p[o + PS_CLIENT_READY] <= 1
            && p[o + PS_SAVE_STATUS] <= 2
            && p[o + PS_ACTIVITY] <= MAX_ACTIVITY;
        if !ok {
            p[o..o + PLAYER_STATE_SIZE].fill(0);
            continue;
        }
        if p[o + PS_ANIM] > MAX_ANIM {
            p[o + PS_ANIM] = 0; // cosmetic only
        }
        if p[o + PS_FIELD_BUSY] > 1 {
            p[o + PS_FIELD_BUSY] = 1;
        }
        p[o + PS_CHALLENGE_FLAGS] &= CHALLENGE_FLAGS_MASK;
        // A bad follower hides only the follower, not the player.
        let follower_ok = u16_at(&p, o + PS_FOLLOWER_SPECIES) <= limits.species
            && p[o + PS_FOLLOWER_SHINY] <= 1
            && p[o + PS_FOLLOWER_GENDER] <= 2
            && p[o + PS_FOLLOWER_VISIBLE] <= 1
            && p[o + PS_FOLLOWER_DIRECTION] <= 3
            && is_valid_move_action(p[o + PS_FOLLOWER_MOVE_ACTION]);
        if !follower_ok {
            p[o + PS_FOLLOWER_SPECIES..o + PS_AVATAR_ID].fill(0);
        }
    }

    let g = PAYLOAD_GATES;
    let max_players = MAX_PLAYERS as u8;
    let gates_ok = p[g + GATE_SYNC_READY] <= 1
        && p[g + GATE_CHECKPOINT] <= 16
        && p[g + GATE_MONOLOCKE] <= 18
        && p[g + GATE_CHALLENGE_ROOM_STATE] <= 1
        && p[g + GATE_CAMPAIGN_STARTED] <= 1
        && p[g + GATE_EXPECTED_PARTY] <= max_players
        && p[g + GATE_ONLINE_PARTY] <= max_players
        && p[g + GATE_READY_PARTY] <= max_players;
    if !gates_ok {
        p[g..g + GATES_SIZE].fill(0);
    }
    // Session and team fields: a bad value drops only those (the ROM reads state 0 as "no
    // session" and 0xFF as "no slot").
    let session_ok = slot_ok(p[g + GATE_TEAM_PARTNER])
        && p[g + GATE_TEAMED_MASK] & 0x60 == 0
        && p[g + GATE_SESSION_STATE] <= MAX_SESSION_STATE
        && p[g + GATE_SESSION_KIND] <= MAX_SESSION_KIND
        && slot_ok(p[g + GATE_SESSION_PEER])
        && p[g + GATE_SESSION_REASON] <= MAX_SESSION_REASON;
    if p[g + GATE_TRADE_PENDING] > 1 {
        p[g + GATE_TRADE_PENDING] = 1;
    }
    if !session_ok {
        p[g + GATE_TEAM_PARTNER..g + GATE_TEAM_PARTNER + GATE_SESSION_SIZE].fill(0);
        p[g + GATE_TEAM_PARTNER] = PLAYER_ID_NONE;
        p[g + GATE_SESSION_PEER] = PLAYER_ID_NONE;
    }
    p
}

/// The relay's trade trailer, checked: out-of-range header values give "nothing" (zeros). The
/// Pokémon bytes are the ROM's to validate.
pub fn trade_in(trailer: &[u8]) -> [u8; TRADE_IN_SIZE] {
    let mut t = [0u8; TRADE_IN_SIZE];
    t.copy_from_slice(&trailer[..TRADE_IN_SIZE]);
    let ok = t[TRADE_IN_STATE] <= TRADE_IN_STATE_MAX
        && t[TRADE_IN_REASON] <= TRADE_IN_REASON_MAX
        && slot_ok(t[TRADE_IN_PEER_SLOT]);
    if !ok {
        t.fill(0);
    }
    t
}

/// The C# bridge's 16-bit byte sum (`ChallengeRules.Checksum`).
pub fn checksum(buf: &[u8]) -> u16 {
    buf.iter().fold(0u16, |s, &b| s.wrapping_add(u16::from(b)))
}

/// Splits the relay's challenge trailer off the end of a server packet (it's always last), checks
/// and sanitizes it. Returns the packet without it, and the trailer when it was valid.
pub fn take_challenge_down(pkt: &[u8]) -> (&[u8], Option<[u8; CHALLENGE_DOWN_SIZE]>) {
    let server = goldlink_proto::SERVER_PACKET_SIZE;
    if pkt.len() < server + CHALLENGE_DOWN_SIZE {
        return (pkt, None);
    }
    let at = pkt.len() - CHALLENGE_DOWN_SIZE;
    let t = &pkt[at..];
    if u32_at(t, 0) != CHALLENGE_MAGIC
        || t[4] != CHALLENGE_VERSION
        || u16_at(t, 60) != checksum(&t[..60])
    {
        return (pkt, None);
    }
    let mut down = [0u8; CHALLENGE_DOWN_SIZE];
    down.copy_from_slice(t);
    // Unknown rule bits, a type out of range, an unknown room state.
    let rules = u16_at(&down, 8) & CHALLENGE_RULES_VALID;
    if down[5] > MONO_TYPE_NONE {
        down[5] = MONO_TYPE_NONE;
    }
    down[6] &= 0x01;
    down[8..10].copy_from_slice(&rules.to_le_bytes());
    let sum = checksum(&down[..60]);
    down[60..62].copy_from_slice(&sum.to_le_bytes());
    (&pkt[..at], Some(down))
}

#[cfg(test)]
mod tests {
    use super::*;

    const LIMITS: Limits = Limits {
        map_id: 600,
        species: 1100,
    };

    fn valid_player(p: &mut [u8], slot: usize) {
        let o = slot * PLAYER_STATE_SIZE;
        p[o + PS_MAP_ID..o + PS_MAP_ID + 2].copy_from_slice(&100u16.to_le_bytes());
        p[o + PS_IS_ACTIVE] = 1;
        p[o + PS_MOVE_ACTION] = MOVE_ACTION_NONE;
        p[o + PS_FOLLOWER_MOVE_ACTION] = MOVE_ACTION_NONE;
        p[o + PS_TRAINER_ID] = 7;
    }

    #[test]
    fn keeps_valid_and_zeroes_bad_players() {
        let mut raw = [0u8; SERVER_PAYLOAD_SIZE];
        valid_player(&mut raw, 0);
        valid_player(&mut raw, 1);
        raw[PLAYER_STATE_SIZE + PS_ACTIVITY] = MAX_ACTIVITY + 1; // slot 1: bad
        valid_player(&mut raw, 2);
        raw[2 * PLAYER_STATE_SIZE + PS_ACTIVITY] = MAX_ACTIVITY; // slot 2: the event "!" is fine
        let p = server_payload(&raw, LIMITS);
        assert_eq!(p[PS_TRAINER_ID], 7);
        assert!(
            p[PLAYER_STATE_SIZE..2 * PLAYER_STATE_SIZE]
                .iter()
                .all(|&b| b == 0)
        );
        assert_eq!(p[2 * PLAYER_STATE_SIZE + PS_ACTIVITY], 5);
    }

    #[test]
    fn active_player_on_map_zero_or_past_the_limit_is_dropped() {
        let mut raw = [0u8; SERVER_PAYLOAD_SIZE];
        valid_player(&mut raw, 0);
        raw[PS_MAP_ID..PS_MAP_ID + 2].copy_from_slice(&0u16.to_le_bytes());
        assert_eq!(server_payload(&raw, LIMITS)[PS_TRAINER_ID], 0);
        raw[PS_MAP_ID..PS_MAP_ID + 2].copy_from_slice(&600u16.to_le_bytes());
        assert_eq!(server_payload(&raw, LIMITS)[PS_TRAINER_ID], 0);
        // Inactive on map 0 is an empty slot, kept as is.
        raw[PS_MAP_ID..PS_MAP_ID + 2].copy_from_slice(&0u16.to_le_bytes());
        raw[PS_IS_ACTIVE] = 0;
        assert_eq!(server_payload(&raw, LIMITS)[PS_TRAINER_ID], 7);
    }

    #[test]
    fn a_bad_follower_hides_only_the_follower() {
        let mut raw = [0u8; SERVER_PAYLOAD_SIZE];
        valid_player(&mut raw, 0);
        raw[PS_FOLLOWER_SPECIES..PS_FOLLOWER_SPECIES + 2].copy_from_slice(&2000u16.to_le_bytes());
        raw[PS_FOLLOWER_VISIBLE] = 1;
        raw[PS_AVATAR_ID] = 9;
        raw[PS_ANIM] = 99;
        raw[PS_FIELD_BUSY] = 7;
        raw[PS_CHALLENGE_FLAGS] = 0xFF;
        let p = server_payload(&raw, LIMITS);
        assert_eq!(p[PS_TRAINER_ID], 7);
        assert!(p[PS_FOLLOWER_SPECIES..PS_AVATAR_ID].iter().all(|&b| b == 0));
        assert_eq!(
            (
                p[PS_AVATAR_ID],
                p[PS_ANIM],
                p[PS_FIELD_BUSY],
                p[PS_CHALLENGE_FLAGS]
            ),
            (9, 0, 1, 1)
        );
    }

    #[test]
    fn gates_and_session_fields() {
        let g = PAYLOAD_GATES;
        let mut raw = [0u8; SERVER_PAYLOAD_SIZE];
        raw[g + GATE_ROOM_UID] = 0x42;
        raw[g + GATE_TEAM_PARTNER] = PLAYER_ID_NONE;
        raw[g + GATE_SESSION_PEER] = 2;
        raw[g + GATE_SESSION_STATE] = 3;
        raw[g + GATE_TRADE_PENDING] = 9;
        let p = server_payload(&raw, LIMITS);
        assert_eq!(
            (
                p[g + GATE_ROOM_UID],
                p[g + GATE_SESSION_STATE],
                p[g + GATE_TRADE_PENDING]
            ),
            (0x42, 3, 1)
        );
        // A bad session value clears the session fields only.
        raw[g + GATE_SESSION_KIND] = 7;
        let p = server_payload(&raw, LIMITS);
        assert_eq!(p[g + GATE_ROOM_UID], 0x42);
        assert_eq!(
            (
                p[g + GATE_TEAM_PARTNER],
                p[g + GATE_SESSION_PEER],
                p[g + GATE_SESSION_STATE]
            ),
            (0xFF, 0xFF, 0)
        );
        // A bad gate clears all the gates.
        raw[g + GATE_CHECKPOINT] = 17;
        assert!(server_payload(&raw, LIMITS)[g..].iter().all(|&b| b == 0));
    }

    #[test]
    fn trade_trailer_header_checked() {
        let mut t = [0u8; TRADE_IN_SIZE];
        t[TRADE_IN_STATE] = 2;
        t[TRADE_IN_PEER_SLOT] = 4;
        t[100] = 5;
        assert_eq!(trade_in(&t)[100], 5);
        t[TRADE_IN_PEER_SLOT] = 5;
        assert!(trade_in(&t).iter().all(|&b| b == 0));
    }

    #[test]
    fn challenge_down_is_split_checked_and_sanitized() {
        let mut pkt = vec![1u8; goldlink_proto::SERVER_PACKET_SIZE];
        let mut down = [0u8; CHALLENGE_DOWN_SIZE];
        down[0..4].copy_from_slice(&CHALLENGE_MAGIC.to_le_bytes());
        down[4] = CHALLENGE_VERSION;
        down[5] = 200;
        down[6] = 0xFF;
        down[8..10].copy_from_slice(&0xFFFFu16.to_le_bytes());
        let sum = checksum(&down[..60]);
        down[60..62].copy_from_slice(&sum.to_le_bytes());
        pkt.extend_from_slice(&down);
        let (rest, got) = take_challenge_down(&pkt);
        assert_eq!(rest.len(), goldlink_proto::SERVER_PACKET_SIZE);
        let got = got.unwrap();
        assert_eq!(
            (got[5], got[6], u16_at(&got, 8)),
            (MONO_TYPE_NONE, 1, CHALLENGE_RULES_VALID)
        );
        assert_eq!(u16_at(&got, 60), checksum(&got[..60]));
        // A bad checksum: the packet stays whole.
        let n = pkt.len();
        pkt[n - 3] ^= 1;
        assert!(take_challenge_down(&pkt).1.is_none());
    }
}
