//! Milestone 5 challenge rules and ledgers (docs/MILESTONE5_PLAN.md §4.2, §4.4).

use std::collections::HashSet;
use std::fs;
use std::path::PathBuf;
use std::time::Duration;

use crate::lobby::LobbyState;
use crate::protocol::{
    CHALLENGE_DOWN_SIZE, CHALLENGE_RULE_LEGENDS_WILD, CHALLENGE_RULE_MONOLOCKE,
    CHALLENGE_RULE_NUZLOCKE, CHALLENGE_RULE_RANDOM_ABILITIES, CHALLENGE_RULE_SOUL_LINK_SHIFT,
    CHALLENGE_STATE_SPIRIT, CHALLENGE_UP_SIZE, ChallengeDown, ChallengeUp, ClientToServerPacket,
    LINK_ROLE_PARENT, LinkUp, MONO_TYPE_COUNT, MONO_TYPE_NONE, PLAYER_STATE_SIZE, REMOTE_SLOTS,
    SOUL_LINK_DUO, SOUL_LINK_ROOM_WIDE, SessionOut, sanitize_challenge_rules,
};
use crate::session_tests::{addr, packet};

const HOST_TOKEN: u32 = 0x00C0_FFEE;
const RANDOM_SEMI: u16 = 1;
const RANDOM_FULL: u16 = 3;

fn room_link(mode: u16) -> u16 {
    mode << CHALLENGE_RULE_SOUL_LINK_SHIFT
}

/// A forming room whose player 0 is the host; `rules` is what each packet
/// carries (only the host's counts).
fn room_with_rules(n: usize, rules: u16, start: bool, dir: Option<PathBuf>) -> LobbyState {
    let mut lobby = LobbyState::with_room(
        *b"TEST01",
        HOST_TOKEN,
        None,
        Duration::from_secs(60),
        n,
        false,
        dir,
    )
    .unwrap();
    for i in 0..n {
        lobby
            .handle_client_packet(addr(i), forming(i, rules, start))
            .unwrap();
    }
    assert_eq!(lobby.is_campaign_started(), start);
    lobby
}

fn forming(i: usize, rules: u16, ready: bool) -> ClientToServerPacket {
    let mut pkt = packet(i, SessionOut::default(), false);
    pkt.challenge_rules = rules;
    pkt.host_token = if i == 0 { HOST_TOKEN } else { 0 };
    if !ready {
        pkt.local_player.client_ready = 0;
    }
    pkt
}

fn report(lobby: &mut LobbyState, i: usize, up: ChallengeUp) {
    let mut pkt = packet(i, SessionOut::default(), true);
    pkt.challenge = Some(up);
    lobby.handle_client_packet(addr(i), pkt).unwrap();
}

/// The down trailer (always the last bytes) and gates +6 / +7 for player i.
fn down(lobby: &mut LobbyState, i: usize) -> (ChallengeDown, u8, u8) {
    let batch = lobby.build_broadcast_batch();
    let (_, buf) = batch
        .iter()
        .find(|(a, _)| *a == addr(i))
        .expect("player in the batch");
    let trailer = ChallengeDown::decode(&buf[buf.len() - CHALLENGE_DOWN_SIZE..])
        .expect("every server packet ends with the challenge trailer");
    let gates = 16 + REMOTE_SLOTS * PLAYER_STATE_SIZE;
    (trailer, buf[gates + 6], buf[gates + 7])
}

fn bits(locations: &[usize]) -> [u8; 16] {
    let mut out = [0u8; 16];
    for &l in locations {
        out[l / 8] |= 1 << (l % 8);
    }
    out
}

#[test]
fn test_trailers_round_trip_and_reject_damage() {
    let up = ChallengeUp {
        state: CHALLENGE_STATE_SPIRIT,
        living: 3,
        rules: CHALLENGE_RULE_NUZLOCKE,
        deaths: 7,
        spent: bits(&[0, 9, 127]),
        failed: bits(&[9]),
    };
    let bytes = up.encode();
    assert_eq!(bytes.len(), CHALLENGE_UP_SIZE);
    assert_eq!(ChallengeUp::decode(&bytes), Some(up));
    let mut damaged = bytes;
    damaged[20] ^= 1;
    assert_eq!(ChallengeUp::decode(&damaged), None, "checksum");
    assert_eq!(
        ChallengeUp::decode(&bytes[..CHALLENGE_UP_SIZE - 1]),
        None,
        "length"
    );

    let down = ChallengeDown {
        mono_type: 9,
        room_state: 0,
        rules: CHALLENGE_RULE_MONOLOCKE,
        link_union: bits(&[1]),
        spent: bits(&[2]),
        failed: bits(&[3]),
    };
    assert_eq!(ChallengeDown::decode(&down.encode()), Some(down));
}

#[test]
fn test_the_challenge_trailer_follows_a_link_trailer() {
    let mut pkt = packet(0, SessionOut::default(), true);
    pkt.link = Some(LinkUp {
        epoch: 1,
        role: LINK_ROLE_PARENT,
        ack: 0,
        first_seq: 0,
        beacon: None,
        msgs: Vec::new(),
    });
    let up = ChallengeUp {
        spent: bits(&[5]),
        ..ChallengeUp::default()
    };
    pkt.challenge = Some(up);
    let decoded = ClientToServerPacket::decode_le(&pkt.encode_full()).unwrap();
    assert_eq!(decoded.challenge, Some(up));
    assert!(decoded.link.is_some(), "the link trailer still parses");
    assert!(decoded.trade.is_none());

    pkt.challenge = None;
    let decoded = ClientToServerPacket::decode_le(&pkt.encode_full()).unwrap();
    assert_eq!(decoded.challenge, None);
    assert!(decoded.link.is_some());
}

#[test]
fn test_rules_are_sanitized() {
    let soul_link = room_link(SOUL_LINK_ROOM_WIDE);
    // Soul-Link needs Nuzlocke.
    assert_eq!(sanitize_challenge_rules(soul_link), 0);
    assert_eq!(
        sanitize_challenge_rules(soul_link | CHALLENGE_RULE_NUZLOCKE),
        soul_link | CHALLENGE_RULE_NUZLOCKE
    );
    // Mode 3 is not a soul-link mode.
    assert_eq!(
        sanitize_challenge_rules(room_link(3) | CHALLENGE_RULE_NUZLOCKE),
        CHALLENGE_RULE_NUZLOCKE
    );
    // Legends and random abilities need the randomizer.
    let extras = CHALLENGE_RULE_LEGENDS_WILD | CHALLENGE_RULE_RANDOM_ABILITIES;
    assert_eq!(sanitize_challenge_rules(extras), 0);
    assert_eq!(
        sanitize_challenge_rules(extras | RANDOM_SEMI),
        extras | RANDOM_SEMI
    );
    // Unknown bits are dropped.
    assert_eq!(sanitize_challenge_rules(0xFE00), 0);
}

#[test]
fn test_only_the_host_sets_rules_and_only_while_forming() {
    let rules = CHALLENGE_RULE_NUZLOCKE | RANDOM_FULL;
    let mut lobby = room_with_rules(2, 0, false, None);
    // A guest's rules are ignored.
    let mut guest = forming(1, rules, false);
    guest.host_token = HOST_TOKEN ^ 1;
    lobby.handle_client_packet(addr(1), guest).unwrap();
    assert_eq!(lobby.rules(), 0);
    // The host's are taken, and every member sees them.
    lobby
        .handle_client_packet(addr(0), forming(0, rules, false))
        .unwrap();
    assert_eq!(lobby.rules(), rules);
    assert_eq!(down(&mut lobby, 1).0.rules, rules);
    // Once the roster locks, they are fixed.
    for i in 0..2 {
        lobby
            .handle_client_packet(addr(i), forming(i, rules, true))
            .unwrap();
    }
    assert!(lobby.is_campaign_started());
    lobby
        .handle_client_packet(addr(0), forming(0, 0, true))
        .unwrap();
    assert_eq!(lobby.rules(), rules);
}

#[test]
fn test_monolocke_types_are_unique_and_none_without_the_rule() {
    let mut lobby = room_with_rules(4, CHALLENGE_RULE_MONOLOCKE, true, None);
    let mut seen = HashSet::new();
    for i in 0..4 {
        let (d, gate_type, _) = down(&mut lobby, i);
        assert!(d.mono_type < MONO_TYPE_COUNT);
        assert_eq!(gate_type, d.mono_type, "gates +6 carries the same type");
        assert!(seen.insert(d.mono_type), "type {} given twice", d.mono_type);
    }

    let mut plain = room_with_rules(2, CHALLENGE_RULE_NUZLOCKE, true, None);
    assert_eq!(down(&mut plain, 0).0.mono_type, MONO_TYPE_NONE);
    // Before the lock nobody has a type yet.
    let mut forming_room = room_with_rules(2, CHALLENGE_RULE_MONOLOCKE, false, None);
    assert_eq!(down(&mut forming_room, 0).0.mono_type, MONO_TYPE_NONE);
}

#[test]
fn test_ledgers_only_gain_bits_and_survive_a_restart() {
    let dir = PathBuf::from("target/test_saves_challenge_ledger");
    let _ = fs::remove_dir_all(&dir);
    let rules = CHALLENGE_RULE_NUZLOCKE;
    let mut lobby = room_with_rules(2, rules, true, Some(dir.clone()));

    report(
        &mut lobby,
        0,
        ChallengeUp {
            rules,
            spent: bits(&[3, 4]),
            failed: bits(&[4]),
            deaths: 2,
            ..ChallengeUp::default()
        },
    );
    // A restored backup reports fewer bits: nothing is reopened.
    report(
        &mut lobby,
        0,
        ChallengeUp {
            rules,
            spent: bits(&[3]),
            deaths: 1,
            ..ChallengeUp::default()
        },
    );
    let (d, _, _) = down(&mut lobby, 0);
    assert_eq!(d.spent, bits(&[3, 4]));
    assert_eq!(d.failed, bits(&[4]));
    assert_eq!(
        down(&mut lobby, 1).0.spent,
        [0u8; 16],
        "ledgers are per player"
    );

    let mut restored = LobbyState::with_room(
        *b"TEST01",
        HOST_TOKEN,
        None,
        Duration::from_secs(60),
        2,
        false,
        Some(dir),
    )
    .unwrap();
    assert_eq!(restored.rules(), rules);
    restored
        .handle_client_packet(addr(0), packet(0, SessionOut::default(), true))
        .unwrap();
    let (d, _, _) = down(&mut restored, 0);
    assert_eq!(d.spent, bits(&[3, 4]));
    assert_eq!(d.failed, bits(&[4]));
}

#[test]
fn test_room_wide_soul_link_shares_failures_and_duo_waits_for_phase_6() {
    let room_wide = CHALLENGE_RULE_NUZLOCKE | room_link(SOUL_LINK_ROOM_WIDE);
    let mut lobby = room_with_rules(3, room_wide, true, None);
    report(
        &mut lobby,
        1,
        ChallengeUp {
            failed: bits(&[10]),
            ..ChallengeUp::default()
        },
    );
    for i in 0..3 {
        assert_eq!(down(&mut lobby, i).0.link_union, bits(&[10]), "player {i}");
    }

    let duo = CHALLENGE_RULE_NUZLOCKE | room_link(SOUL_LINK_DUO);
    let mut lobby = room_with_rules(2, duo, true, None);
    report(
        &mut lobby,
        1,
        ChallengeUp {
            failed: bits(&[10]),
            ..ChallengeUp::default()
        },
    );
    assert_eq!(down(&mut lobby, 0).0.link_union, [0u8; 16]);
}
