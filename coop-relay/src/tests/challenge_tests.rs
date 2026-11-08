//! Milestone 5 challenge rules and ledgers (docs/MILESTONE5_PLAN.md §4.2, §4.4).

use std::collections::HashSet;
use std::fs;
use std::path::PathBuf;
use std::time::Duration;

use crate::lobby::LobbyState;
use crate::protocol::{
    CHALLENGE_DOWN_SIZE, CHALLENGE_RULE_LEGENDS_WILD, CHALLENGE_RULE_MONOLOCKE,
    CHALLENGE_RULE_NUZLOCKE, CHALLENGE_RULE_RANDOM_ABILITIES, CHALLENGE_RULE_RANDOM_ITEMS,
    CHALLENGE_RULE_SOUL_LINK_SHIFT, CHALLENGE_STATE_RUN_LOST, CHALLENGE_STATE_SPIRIT,
    CHALLENGE_UP_SIZE, ChallengeDown, ChallengeUp, ClientToServerPacket, LINK_ROLE_PARENT, LinkUp,
    MONO_TYPE_COUNT, MONO_TYPE_NONE, PLAYER_STATE_SIZE, REMOTE_SLOTS, ROOM_STATE_RUN_LOST,
    ROOM_STATUS_ERR_RUN_LOST, SESSION_KIND_BATTLE, SESSION_KIND_LEAVE_TEAM, SESSION_KIND_REVIVE,
    SESSION_KIND_TEAM_UP, SESSION_KIND_TRADE, SESSION_OP_ACCEPT, SESSION_OP_REQUEST,
    SESSION_REASON_BUSY, SESSION_REASON_INVALID, SESSION_REASON_TEAM_RULE, SESSION_STATE_ACCEPTED,
    SESSION_STATE_REFUSED, SOUL_LINK_DUO, SOUL_LINK_ROOM_WIDE, SessionOut,
    sanitize_challenge_rules,
};
use crate::session_tests::{addr, gates, op, packet};

const HOST_TOKEN: u32 = 0x00C0_FFEE;
const RANDOM_SEMI: u16 = 1;
const RANDOM_FULL: u16 = 3;

fn room_link(mode: u16) -> u16 {
    mode << CHALLENGE_RULE_SOUL_LINK_SHIFT
}

/// A forming room whose player 0 is the host; `rules` is what each packet
/// carries (only the host's counts).
pub(crate) fn room_with_rules(
    n: usize,
    rules: u16,
    start: bool,
    dir: Option<PathBuf>,
) -> LobbyState {
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

pub(crate) fn report(lobby: &mut LobbyState, i: usize, up: ChallengeUp) {
    let mut pkt = packet(i, SessionOut::default(), true);
    pkt.challenge = Some(up);
    lobby.handle_client_packet(addr(i), pkt).unwrap();
}

/// The down trailer (always the last bytes) and gates +6 / +7 for player i.
pub(crate) fn down(lobby: &mut LobbyState, i: usize) -> (ChallengeDown, u8, u8) {
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
        room_state: ROOM_STATE_RUN_LOST,
        rules: CHALLENGE_RULE_MONOLOCKE,
        team_deaths: 12,
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
    // Unknown bits are dropped; random items (protocol 8) is known and needs nothing.
    assert_eq!(sanitize_challenge_rules(0xFC00), 0);
    assert_eq!(
        sanitize_challenge_rules(0xFE00),
        CHALLENGE_RULE_RANDOM_ITEMS
    );
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
fn test_room_wide_soul_link_shares_failures() {
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
}

/// Phase 6: Duo links a team from its pairing on; the team is permanent.
#[test]
fn test_duo_soul_link_starts_at_pairing_and_locks_the_team() {
    let duo = CHALLENGE_RULE_NUZLOCKE | room_link(SOUL_LINK_DUO);
    let mut lobby = room_with_rules(3, duo, true, None);
    let failed = |l: &mut LobbyState, i: usize, at: &[usize]| {
        report(
            l,
            i,
            ChallengeUp {
                failed: bits(at),
                ..ChallengeUp::default()
            },
        )
    };
    // Before a team: nobody is linked.
    failed(&mut lobby, 0, &[5]);
    assert_eq!(down(&mut lobby, 0).0.link_union, [0u8; 16]);
    op(
        &mut lobby,
        0,
        1,
        SESSION_OP_REQUEST,
        SESSION_KIND_TEAM_UP,
        1,
    );
    op(&mut lobby, 1, 2, SESSION_OP_ACCEPT, 0, 0);
    assert_eq!(lobby.teams().len(), 1);
    // After it: both see the other's new failures, not the old one (E16).
    failed(&mut lobby, 1, &[10]);
    failed(&mut lobby, 0, &[5, 7]);
    for i in 0..2 {
        assert_eq!(
            down(&mut lobby, i).0.link_union,
            bits(&[7, 10]),
            "teammate {i}"
        );
    }
    assert_eq!(
        down(&mut lobby, 2).0.link_union,
        [0u8; 16],
        "a solo player is unlinked"
    );
    // An offline teammate's last report still counts.
    lobby.remove_client_by_addr(addr(1));
    assert_eq!(down(&mut lobby, 0).0.link_union, bits(&[7, 10]));
    // The team can't be left under Duo.
    op(
        &mut lobby,
        0,
        3,
        SESSION_OP_REQUEST,
        SESSION_KIND_LEAVE_TEAM,
        1,
    );
    let g = gates(&mut lobby, 0);
    assert_eq!(
        (g[2], g[6]),
        (SESSION_STATE_REFUSED, SESSION_REASON_TEAM_RULE)
    );
    assert_eq!(lobby.teams().len(), 1);
}

fn spirit(lobby: &mut LobbyState, i: usize, deaths: u16) {
    report(
        lobby,
        i,
        ChallengeUp {
            state: CHALLENGE_STATE_SPIRIT,
            rules: CHALLENGE_RULE_NUZLOCKE,
            deaths,
            ..ChallengeUp::default()
        },
    );
}

#[test]
fn test_a_revive_needs_a_spirit_and_spirits_take_nothing_else() {
    let mut lobby = room_with_rules(3, CHALLENGE_RULE_NUZLOCKE, true, None);
    // Nobody is a spirit yet: a revive is invalid.
    op(&mut lobby, 0, 1, SESSION_OP_REQUEST, SESSION_KIND_REVIVE, 1);
    let g = gates(&mut lobby, 0);
    assert_eq!(
        (g[2], g[3], g[6]),
        (
            SESSION_STATE_REFUSED,
            SESSION_KIND_REVIVE,
            SESSION_REASON_INVALID
        )
    );

    spirit(&mut lobby, 1, 6);
    // A spirit can't be asked to trade or battle...
    op(&mut lobby, 0, 2, SESSION_OP_REQUEST, SESSION_KIND_TRADE, 1);
    assert_eq!(gates(&mut lobby, 0)[6], SESSION_REASON_BUSY);
    // ...nor ask anything itself.
    op(&mut lobby, 1, 3, SESSION_OP_REQUEST, SESSION_KIND_BATTLE, 2);
    assert_eq!(gates(&mut lobby, 1)[6], SESSION_REASON_INVALID);
    op(&mut lobby, 1, 4, SESSION_OP_REQUEST, SESSION_KIND_REVIVE, 0);
    assert_eq!(gates(&mut lobby, 1)[6], SESSION_REASON_INVALID);

    // A living player's revive is accepted at once, for both, with a trade open.
    op(&mut lobby, 0, 5, SESSION_OP_REQUEST, SESSION_KIND_REVIVE, 1);
    let (g0, g1) = (gates(&mut lobby, 0), gates(&mut lobby, 1));
    assert_eq!(
        (g0[2], g0[3]),
        (SESSION_STATE_ACCEPTED, SESSION_KIND_REVIVE)
    );
    assert_eq!(
        (g1[2], g1[3]),
        (SESSION_STATE_ACCEPTED, SESSION_KIND_REVIVE)
    );
    assert_eq!(lobby.active_trade_count(), 1);
    // A second donor finds the spirit busy.
    op(&mut lobby, 2, 6, SESSION_OP_REQUEST, SESSION_KIND_REVIVE, 1);
    assert_eq!(gates(&mut lobby, 2)[6], SESSION_REASON_BUSY);

    // Without Nuzlocke there are no spirits to revive.
    let mut plain = room_with_rules(2, 0, true, None);
    spirit(&mut plain, 1, 0);
    op(&mut plain, 0, 1, SESSION_OP_REQUEST, SESSION_KIND_REVIVE, 1);
    assert_eq!(gates(&mut plain, 0)[6], SESSION_REASON_INVALID);
}

#[test]
fn test_the_run_is_lost_once_everyone_is_a_spirit() {
    let dir = PathBuf::from("target/test_saves_challenge_run_lost");
    let _ = fs::remove_dir_all(&dir);
    let rules = CHALLENGE_RULE_NUZLOCKE;
    let mut lobby = room_with_rules(2, rules, true, Some(dir.clone()));
    spirit(&mut lobby, 0, 4);
    assert!(
        !lobby.is_run_lost(),
        "one living player keeps the run going (Q8)"
    );
    let (d, _, room_state) = down(&mut lobby, 1);
    assert_eq!((d.room_state, room_state, d.team_deaths), (0, 0, 4));

    spirit(&mut lobby, 1, 3);
    assert!(lobby.is_run_lost());
    let (d, _, room_state) = down(&mut lobby, 0);
    assert_eq!(
        (d.room_state, room_state),
        (ROOM_STATE_RUN_LOST, ROOM_STATE_RUN_LOST)
    );
    assert_eq!(d.team_deaths, 7);

    // Players still connected keep syncing (they see the summary); a new
    // join is turned away, also after a relay restart.
    report(&mut lobby, 0, ChallengeUp::default());
    assert!(lobby.is_run_lost(), "a lost run is never undone");
    // A game that shows the end and saves it reports so, and is let go.
    let mut seen = packet(0, SessionOut::default(), true);
    seen.challenge = Some(ChallengeUp {
        state: CHALLENGE_STATE_SPIRIT | CHALLENGE_STATE_RUN_LOST,
        rules,
        ..ChallengeUp::default()
    });
    assert_eq!(
        lobby.handle_client_packet(addr(0), seen),
        Err(ROOM_STATUS_ERR_RUN_LOST)
    );
    assert_eq!(
        lobby.handle_client_packet(addr(0), packet(0, SessionOut::default(), true)),
        Err(ROOM_STATUS_ERR_RUN_LOST),
        "its reconnect from the title screen too"
    );
    let rejoin = packet(1, SessionOut::default(), true);
    assert_eq!(
        lobby.handle_client_packet(addr(4), rejoin.clone()),
        Err(ROOM_STATUS_ERR_RUN_LOST)
    );
    let mut restarted = LobbyState::with_room(
        *b"TEST01",
        HOST_TOKEN,
        None,
        Duration::from_secs(60),
        2,
        false,
        Some(dir.clone()),
    )
    .unwrap();
    assert!(restarted.is_run_lost());
    assert_eq!(
        restarted.handle_client_packet(addr(1), rejoin),
        Err(ROOM_STATUS_ERR_RUN_LOST)
    );

    // Without Nuzlocke the flag means nothing.
    let mut plain = room_with_rules(2, 0, true, None);
    spirit(&mut plain, 0, 0);
    spirit(&mut plain, 1, 0);
    assert!(!plain.is_run_lost());
    let _ = fs::remove_dir_all(&dir);
}
