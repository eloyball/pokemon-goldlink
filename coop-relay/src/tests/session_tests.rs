//! Player-to-player sessions and teams (docs/MILESTONE3_PLAN.md, Phase 1).

use std::fs;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use crate::lobby::{LobbyState, SESSION_TIMEOUT};
use crate::lobby_tests::make_packet;
use crate::protocol::{
    ClientToServerPacket, NO_SLOT, PLAYER_STATE_SIZE, REMOTE_SLOTS, ROOM_ACTION_SYNC,
    SAVE_STATUS_IN_PROGRESS, SESSION_KIND_BATTLE, SESSION_KIND_LEAVE_TEAM,
    SESSION_KIND_TEAM_BATTLE, SESSION_KIND_TEAM_UP, SESSION_KIND_TRADE, SESSION_OP_ACCEPT,
    SESSION_OP_CANCEL, SESSION_OP_DECLINE, SESSION_OP_REQUEST, SESSION_REASON_ALREADY_TEAMED,
    SESSION_REASON_BUSY, SESSION_REASON_INVALID, SESSION_REASON_NOT_HERE, SESSION_REASON_TEAM_RULE,
    SESSION_REASON_TEAMMATE_AWAY, SESSION_STATE_ACCEPTED, SESSION_STATE_CANCELLED,
    SESSION_STATE_DECLINED, SESSION_STATE_INCOMING, SESSION_STATE_NOTICE, SESSION_STATE_OUTGOING,
    SESSION_STATE_REFUSED, SESSION_STATE_TIMEOUT, SessionOut, TEAMED_SELF_BIT,
};

pub(crate) const TRAINERS: [u32; 4] = [0x1111, 0x2222, 0x3333, 0x4444];

pub(crate) fn addr(i: usize) -> SocketAddr {
    format!("127.0.0.1:{}", 53000 + i).parse().unwrap()
}

/// One player's packet in the field, carrying a session operation.
pub(crate) fn packet(i: usize, session: SessionOut, started: bool) -> ClientToServerPacket {
    let mut pkt = make_packet(
        *b"TEST01",
        ROOM_ACTION_SYNC,
        0,
        i as u8,
        TRAINERS[i],
        0,
        64,
        3,
        3,
        0,
        0,
    );
    if started {
        pkt.local_player.save_status = SAVE_STATUS_IN_PROGRESS;
        pkt.local_player.client_ready = 0;
    }
    pkt.session = session;
    pkt
}

/// A room of `n` players; `start` locks the roster first (all ready in the bedroom).
pub(crate) fn room(n: usize, start: bool) -> LobbyState {
    room_in(n, start, None)
}

pub(crate) fn room_in(n: usize, start: bool, save_dir: Option<PathBuf>) -> LobbyState {
    let mut lobby = LobbyState::with_room(
        *b"TEST01",
        0,
        None,
        Duration::from_secs(60),
        n,
        false,
        save_dir,
    )
    .unwrap();
    for i in 0..n {
        let mut pkt = packet(i, SessionOut::default(), false);
        if !start {
            pkt.local_player.client_ready = 0;
        }
        lobby.handle_client_packet(addr(i), pkt).unwrap();
    }
    assert_eq!(lobby.is_campaign_started(), start);
    lobby
}

pub(crate) fn op(lobby: &mut LobbyState, i: usize, seq: u8, op: u8, kind: u8, target: usize) {
    let session = SessionOut {
        seq,
        op,
        kind,
        target_trainer: TRAINERS.get(target).copied().unwrap_or(0xDEAD),
    };
    lobby
        .handle_client_packet(addr(i), packet(i, session, lobby.is_campaign_started()))
        .unwrap();
}

/// Gates bytes 16..24 for player i: partner slot, teamed mask, state, kind,
/// peer slot, seq, reason, pad.
pub(crate) fn gates(lobby: &mut LobbyState, i: usize) -> [u8; 8] {
    let batch = lobby.build_broadcast_batch();
    let (_, buf) = batch
        .iter()
        .find(|(a, _)| *a == addr(i))
        .expect("player in the batch");
    let base = 16 + REMOTE_SLOTS * PLAYER_STATE_SIZE + 16;
    buf[base..base + 8].try_into().unwrap()
}

#[test]
fn test_trade_request_accept_and_decline() {
    let mut lobby = room(2, true);
    op(&mut lobby, 0, 1, SESSION_OP_REQUEST, SESSION_KIND_TRADE, 1);
    let g0 = gates(&mut lobby, 0);
    let g1 = gates(&mut lobby, 1);
    assert_eq!(
        (g0[2], g0[3], g0[4]),
        (SESSION_STATE_OUTGOING, SESSION_KIND_TRADE, 0)
    );
    assert_eq!(
        (g1[2], g1[3], g1[4]),
        (SESSION_STATE_INCOMING, SESSION_KIND_TRADE, 0)
    );
    assert_ne!(g1[5], 0, "a change bumps the sequence");

    // The client repeats its latched operation every packet: applied once.
    op(&mut lobby, 0, 1, SESSION_OP_REQUEST, SESSION_KIND_TRADE, 1);
    assert_eq!(gates(&mut lobby, 1)[5], g1[5]);

    op(&mut lobby, 1, 1, SESSION_OP_ACCEPT, 0, 0);
    assert_eq!(gates(&mut lobby, 0)[2], SESSION_STATE_ACCEPTED);
    assert_eq!(gates(&mut lobby, 1)[2], SESSION_STATE_ACCEPTED);
    // The accepted trade is open (gates.trade_pending), so both are busy.
    assert_eq!(gates(&mut lobby, 0)[7], 1);
    op(&mut lobby, 0, 2, SESSION_OP_REQUEST, SESSION_KIND_BATTLE, 1);
    assert_eq!(gates(&mut lobby, 0)[6], SESSION_REASON_BUSY);

    // Declining, in a room without an open trade.
    let mut lobby = room(2, true);
    op(&mut lobby, 0, 1, SESSION_OP_REQUEST, SESSION_KIND_BATTLE, 1);
    op(&mut lobby, 1, 1, SESSION_OP_DECLINE, 0, 0);
    assert_eq!(gates(&mut lobby, 0)[2], SESSION_STATE_DECLINED);
    assert_eq!(gates(&mut lobby, 1)[2], SESSION_STATE_DECLINED);
    assert_eq!(gates(&mut lobby, 0)[7], 0, "no trade for a battle request");
}

#[test]
fn test_cancel_timeout_and_disconnect() {
    let mut lobby = room(2, true);
    op(&mut lobby, 0, 1, SESSION_OP_REQUEST, SESSION_KIND_TRADE, 1);
    op(&mut lobby, 0, 2, SESSION_OP_CANCEL, 0, 0);
    assert_eq!(gates(&mut lobby, 0)[2], SESSION_STATE_CANCELLED);
    assert_eq!(gates(&mut lobby, 1)[2], SESSION_STATE_CANCELLED);

    op(&mut lobby, 0, 3, SESSION_OP_REQUEST, SESSION_KIND_TRADE, 1);
    lobby.expire_sessions(Instant::now() + SESSION_TIMEOUT);
    assert_eq!(gates(&mut lobby, 0)[2], SESSION_STATE_TIMEOUT);
    assert_eq!(gates(&mut lobby, 1)[2], SESSION_STATE_TIMEOUT);

    op(&mut lobby, 0, 4, SESSION_OP_REQUEST, SESSION_KIND_TRADE, 1);
    lobby.remove_client_by_addr(addr(1));
    let g0 = gates(&mut lobby, 0);
    assert_eq!(g0[2], SESSION_STATE_CANCELLED);
    assert_eq!(g0[4], NO_SLOT, "the peer is gone");
}

#[test]
fn test_refusals() {
    let mut lobby = room(3, true);
    // Unknown target.
    op(&mut lobby, 0, 1, SESSION_OP_REQUEST, SESSION_KIND_TRADE, 9);
    assert_eq!(
        gates(&mut lobby, 0)[2..7],
        [
            SESSION_STATE_REFUSED,
            SESSION_KIND_TRADE,
            NO_SLOT,
            1,
            SESSION_REASON_NOT_HERE
        ]
    );
    // Oneself.
    op(&mut lobby, 0, 2, SESSION_OP_REQUEST, SESSION_KIND_TRADE, 0);
    assert_eq!(gates(&mut lobby, 0)[6], SESSION_REASON_INVALID);
    // Busy: player 1 is already being asked.
    op(&mut lobby, 0, 3, SESSION_OP_REQUEST, SESSION_KIND_TRADE, 1);
    op(&mut lobby, 2, 1, SESSION_OP_REQUEST, SESSION_KIND_TRADE, 1);
    let g2 = gates(&mut lobby, 2);
    assert_eq!((g2[2], g2[6]), (SESSION_STATE_REFUSED, SESSION_REASON_BUSY));
}

#[test]
fn test_team_up_rules_battles() {
    let mut lobby = room(3, true);
    op(
        &mut lobby,
        0,
        1,
        SESSION_OP_REQUEST,
        SESSION_KIND_TEAM_UP,
        1,
    );
    op(&mut lobby, 1, 1, SESSION_OP_ACCEPT, 0, 0);
    assert_eq!(lobby.teams(), &[[TRAINERS[0], TRAINERS[1]]]);
    assert_eq!(lobby.team_partner_of(TRAINERS[1]), Some(TRAINERS[0]));

    // Player 0 sees its partner (absolute 1) in remote slot 0; player 1 sees
    // player 0 in remote slot 0 too; player 2 sees both teamed.
    let g0 = gates(&mut lobby, 0);
    assert_eq!(g0[0], 0);
    assert_eq!(g0[1], TEAMED_SELF_BIT | 0b01);
    let g2 = gates(&mut lobby, 2);
    assert_eq!(g2[0], NO_SLOT);
    assert_eq!(
        g2[1], 0b11,
        "remote slots 0 and 1 are teamed; player 2 is not"
    );

    // A teamed player cannot join another team (leaving comes first).
    op(
        &mut lobby,
        2,
        1,
        SESSION_OP_REQUEST,
        SESSION_KIND_TEAM_UP,
        0,
    );
    assert_eq!(gates(&mut lobby, 2)[6], SESSION_REASON_ALREADY_TEAMED);
    // A team does not battle a solo player (backlog) nor its own teammate.
    op(&mut lobby, 2, 2, SESSION_OP_REQUEST, SESSION_KIND_BATTLE, 0);
    assert_eq!(gates(&mut lobby, 2)[6], SESSION_REASON_TEAM_RULE);
    op(&mut lobby, 0, 2, SESSION_OP_REQUEST, SESSION_KIND_BATTLE, 1);
    assert_eq!(gates(&mut lobby, 0)[6], SESSION_REASON_TEAM_RULE);
    // Trading is open to everyone.
    op(&mut lobby, 2, 3, SESSION_OP_REQUEST, SESSION_KIND_TRADE, 0);
    assert_eq!(gates(&mut lobby, 2)[2], SESSION_STATE_OUTGOING);
}

#[test]
fn test_team_up_needs_a_started_adventure() {
    let mut lobby = room(2, false);
    op(
        &mut lobby,
        0,
        1,
        SESSION_OP_REQUEST,
        SESSION_KIND_TEAM_UP,
        1,
    );
    assert_eq!(gates(&mut lobby, 0)[6], SESSION_REASON_INVALID);
}

#[test]
fn test_teams_survive_a_relay_restart() {
    let dir = PathBuf::from("target/test_saves_teams");
    let _ = fs::remove_dir_all(&dir);
    let mut lobby = room_in(2, true, Some(dir.clone()));
    op(
        &mut lobby,
        0,
        1,
        SESSION_OP_REQUEST,
        SESSION_KIND_TEAM_UP,
        1,
    );
    op(&mut lobby, 1, 1, SESSION_OP_ACCEPT, 0, 0);
    drop(lobby);

    let restored = LobbyState::with_room(
        *b"TEST01",
        0,
        None,
        Duration::from_secs(60),
        2,
        false,
        Some(dir.clone()),
    )
    .unwrap();
    assert!(restored.is_campaign_started());
    assert_eq!(restored.teams(), &[[TRAINERS[0], TRAINERS[1]]]);
    let _ = fs::remove_dir_all(&dir);
}

/// Teams 0 + 1 in a started room of `n`.
pub(crate) fn teamed_room(n: usize, save_dir: Option<PathBuf>) -> LobbyState {
    let mut lobby = room_in(n, true, save_dir);
    op(
        &mut lobby,
        0,
        1,
        SESSION_OP_REQUEST,
        SESSION_KIND_TEAM_UP,
        1,
    );
    op(&mut lobby, 1, 1, SESSION_OP_ACCEPT, 0, 0);
    assert_eq!(lobby.teams(), &[[TRAINERS[0], TRAINERS[1]]]);
    lobby
}

#[test]
fn test_leave_team() {
    let mut lobby = teamed_room(3, None);

    // Not in a team: nothing to leave.
    op(
        &mut lobby,
        2,
        1,
        SESSION_OP_REQUEST,
        SESSION_KIND_LEAVE_TEAM,
        0,
    );
    let g2 = gates(&mut lobby, 2);
    assert_eq!(
        (g2[2], g2[3], g2[6]),
        (
            SESSION_STATE_REFUSED,
            SESSION_KIND_LEAVE_TEAM,
            SESSION_REASON_INVALID
        )
    );

    // Player 0 leaves at once: no answer from player 1, who reads a notice.
    op(
        &mut lobby,
        0,
        2,
        SESSION_OP_REQUEST,
        SESSION_KIND_LEAVE_TEAM,
        1,
    );
    assert!(lobby.teams().is_empty());
    let g0 = gates(&mut lobby, 0);
    assert_eq!((g0[0], g0[1]), (NO_SLOT, 0), "no partner, nobody teamed");
    assert_eq!(
        (g0[2], g0[3], g0[4]),
        (SESSION_STATE_ACCEPTED, SESSION_KIND_LEAVE_TEAM, 0)
    );
    let g1 = gates(&mut lobby, 1);
    assert_eq!((g1[0], g1[1]), (NO_SLOT, 0));
    assert_eq!(
        (g1[2], g1[3], g1[4]),
        (SESSION_STATE_NOTICE, SESSION_KIND_LEAVE_TEAM, 0)
    );

    // Either can team up again, with anyone.
    op(
        &mut lobby,
        0,
        3,
        SESSION_OP_REQUEST,
        SESSION_KIND_TEAM_UP,
        2,
    );
    op(&mut lobby, 2, 2, SESSION_OP_ACCEPT, 0, 0);
    assert_eq!(lobby.teams(), &[[TRAINERS[0], TRAINERS[2]]]);
}

#[test]
fn test_leave_team_while_busy() {
    let mut lobby = teamed_room(3, None);

    // The partner is asking someone else: the team is dropped, the partner's
    // request is left alone (no notice over it).
    op(&mut lobby, 1, 2, SESSION_OP_REQUEST, SESSION_KIND_TRADE, 2);
    op(
        &mut lobby,
        0,
        2,
        SESSION_OP_REQUEST,
        SESSION_KIND_LEAVE_TEAM,
        1,
    );
    assert!(lobby.teams().is_empty());
    assert_eq!(gates(&mut lobby, 1)[2], SESSION_STATE_OUTGOING);
    op(&mut lobby, 1, 3, SESSION_OP_CANCEL, 0, 0);

    // The leaver is in a session of its own: refused, the team stays.
    let mut lobby = teamed_room(3, None);
    op(&mut lobby, 2, 1, SESSION_OP_REQUEST, SESSION_KIND_TRADE, 0);
    op(
        &mut lobby,
        0,
        2,
        SESSION_OP_REQUEST,
        SESSION_KIND_LEAVE_TEAM,
        1,
    );
    assert_eq!(gates(&mut lobby, 0)[6], SESSION_REASON_BUSY);
    assert_eq!(lobby.teams(), &[[TRAINERS[0], TRAINERS[1]]]);
}

#[test]
fn test_leaving_is_saved() {
    let dir = PathBuf::from("target/test_saves_leave_team");
    let _ = fs::remove_dir_all(&dir);
    let mut lobby = teamed_room(2, Some(dir.clone()));
    op(
        &mut lobby,
        1,
        2,
        SESSION_OP_REQUEST,
        SESSION_KIND_LEAVE_TEAM,
        0,
    );
    drop(lobby);

    let restored = LobbyState::with_room(
        *b"TEST01",
        0,
        None,
        Duration::from_secs(60),
        2,
        false,
        Some(dir.clone()),
    )
    .unwrap();
    assert!(restored.teams().is_empty());
    let _ = fs::remove_dir_all(&dir);
}

/// Players 0+1 and 2+3 in two teams.
fn two_teams() -> LobbyState {
    let mut lobby = teamed_room(4, None);
    op(
        &mut lobby,
        2,
        1,
        SESSION_OP_REQUEST,
        SESSION_KIND_TEAM_UP,
        3,
    );
    op(&mut lobby, 3, 1, SESSION_OP_ACCEPT, 0, 0);
    assert_eq!(lobby.teams().len(), 2);
    lobby
}

#[test]
fn test_team_battle_needs_all_four() {
    let mut lobby = two_teams();
    op(&mut lobby, 0, 2, SESSION_OP_REQUEST, SESSION_KIND_BATTLE, 2);

    // The three others are asked; my teammate also learns the opponent
    // (player 2 is remote slot 1 for player 1) through the reason byte.
    let g0 = gates(&mut lobby, 0);
    assert_eq!(
        (g0[2], g0[3], g0[4]),
        (SESSION_STATE_OUTGOING, SESSION_KIND_TEAM_BATTLE, 1)
    );
    let g1 = gates(&mut lobby, 1);
    assert_eq!(
        (g1[2], g1[3], g1[4], g1[6]),
        (SESSION_STATE_INCOMING, SESSION_KIND_TEAM_BATTLE, 0, 1)
    );
    for i in [2, 3] {
        let g = gates(&mut lobby, i);
        assert_eq!(
            (g[2], g[3], g[4]),
            (SESSION_STATE_INCOMING, SESSION_KIND_TEAM_BATTLE, 0)
        );
    }

    // Two yeses are not enough; nobody hears anything until the third.
    op(&mut lobby, 2, 2, SESSION_OP_ACCEPT, 0, 0);
    op(&mut lobby, 1, 2, SESSION_OP_ACCEPT, 0, 0);
    op(&mut lobby, 1, 3, SESSION_OP_ACCEPT, 0, 0); // a repeat changes nothing
    for i in 0..4 {
        assert_ne!(
            gates(&mut lobby, i)[2],
            SESSION_STATE_ACCEPTED,
            "player {i}"
        );
    }
    // Everyone is busy meanwhile.
    op(&mut lobby, 1, 4, SESSION_OP_REQUEST, SESSION_KIND_TRADE, 3);
    assert_eq!(gates(&mut lobby, 1)[6], SESSION_REASON_BUSY);

    op(&mut lobby, 3, 2, SESSION_OP_ACCEPT, 0, 0);
    for i in 0..4 {
        let g = gates(&mut lobby, i);
        assert_eq!(
            (g[2], g[3]),
            (SESSION_STATE_ACCEPTED, SESSION_KIND_TEAM_BATTLE),
            "player {i}"
        );
    }
}

#[test]
fn test_team_battle_one_no_ends_it() {
    let mut lobby = two_teams();
    op(&mut lobby, 2, 2, SESSION_OP_REQUEST, SESSION_KIND_BATTLE, 1);
    op(&mut lobby, 1, 2, SESSION_OP_ACCEPT, 0, 0);
    op(&mut lobby, 3, 2, SESSION_OP_DECLINE, 0, 0);
    // The requester sees who said no (player 3: its remote slot 2).
    let g2 = gates(&mut lobby, 2);
    assert_eq!((g2[2], g2[4]), (SESSION_STATE_DECLINED, 2));
    for i in [0, 1, 3] {
        assert_eq!(
            gates(&mut lobby, i)[2],
            SESSION_STATE_DECLINED,
            "player {i}"
        );
    }
    // A late answer finds nothing; a new request is possible at once.
    op(&mut lobby, 0, 2, SESSION_OP_ACCEPT, 0, 0);
    assert_eq!(gates(&mut lobby, 0)[2], SESSION_STATE_DECLINED);
    op(&mut lobby, 0, 3, SESSION_OP_REQUEST, SESSION_KIND_BATTLE, 3);
    assert_eq!(gates(&mut lobby, 0)[2], SESSION_STATE_OUTGOING);
}

#[test]
fn test_team_battle_timeout_cancel_and_leaving() {
    let mut lobby = two_teams();
    op(&mut lobby, 0, 2, SESSION_OP_REQUEST, SESSION_KIND_BATTLE, 2);
    op(&mut lobby, 1, 2, SESSION_OP_ACCEPT, 0, 0);
    lobby.expire_sessions(Instant::now() + SESSION_TIMEOUT);
    for i in 0..4 {
        assert_eq!(gates(&mut lobby, i)[2], SESSION_STATE_TIMEOUT, "player {i}");
    }

    op(&mut lobby, 0, 3, SESSION_OP_REQUEST, SESSION_KIND_BATTLE, 2);
    op(&mut lobby, 0, 4, SESSION_OP_CANCEL, 0, 0);
    for i in 0..4 {
        assert_eq!(
            gates(&mut lobby, i)[2],
            SESSION_STATE_CANCELLED,
            "player {i}"
        );
    }

    // A teammate who goes offline cancels it for everyone.
    op(&mut lobby, 0, 5, SESSION_OP_REQUEST, SESSION_KIND_BATTLE, 2);
    lobby.remove_client_by_addr(addr(3));
    for i in 0..3 {
        assert_eq!(
            gates(&mut lobby, i)[2],
            SESSION_STATE_CANCELLED,
            "player {i}"
        );
    }
}

#[test]
fn test_team_battle_needs_both_teammates_here() {
    let mut lobby = two_teams();
    lobby.remove_client_by_addr(addr(3));
    op(&mut lobby, 0, 2, SESSION_OP_REQUEST, SESSION_KIND_BATTLE, 2);
    assert_eq!(gates(&mut lobby, 0)[2], SESSION_STATE_REFUSED);
    assert_eq!(gates(&mut lobby, 0)[6], SESSION_REASON_TEAMMATE_AWAY);
    // A busy teammate (here: in a trade request) blocks it too.
    let mut lobby = two_teams();
    op(&mut lobby, 1, 2, SESSION_OP_REQUEST, SESSION_KIND_TRADE, 3);
    op(&mut lobby, 0, 2, SESSION_OP_REQUEST, SESSION_KIND_BATTLE, 2);
    assert_eq!(gates(&mut lobby, 0)[6], SESSION_REASON_BUSY);
}

/// Player i's packet with a changed player state, and no new session op.
fn state_packet(lobby: &mut LobbyState, i: usize, edit: impl Fn(&mut ClientToServerPacket)) {
    let mut pkt = packet(i, SessionOut::default(), true);
    edit(&mut pkt);
    lobby.handle_client_packet(addr(i), pkt).unwrap();
}

#[test]
fn test_busy_while_waiting_at_a_boss_or_battling() {
    let mut lobby = room(3, true);
    // Player 1 waits at a team boss for its teammate: busy.
    state_packet(&mut lobby, 1, |p| p.local_player.boss_wait = 1);
    op(&mut lobby, 2, 1, SESSION_OP_REQUEST, SESSION_KIND_TRADE, 1);
    assert_eq!(
        (gates(&mut lobby, 2)[2], gates(&mut lobby, 2)[6]),
        (SESSION_STATE_REFUSED, SESSION_REASON_BUSY)
    );
    // In a battle of its own: busy too.
    state_packet(&mut lobby, 1, |p| {
        p.local_player.boss_wait = 0;
        p.local_player.activity = 2;
    });
    op(&mut lobby, 2, 2, SESSION_OP_REQUEST, SESSION_KIND_BATTLE, 1);
    assert_eq!(gates(&mut lobby, 2)[6], SESSION_REASON_BUSY);
    // Back in the field: free.
    state_packet(&mut lobby, 1, |p| p.local_player.activity = 1);
    op(&mut lobby, 2, 3, SESSION_OP_REQUEST, SESSION_KIND_TRADE, 1);
    assert_eq!(gates(&mut lobby, 2)[2], SESSION_STATE_OUTGOING);
}

#[test]
fn test_busy_while_linked() {
    use crate::protocol::{LINK_ROLE_PARENT, LinkUp};
    let mut lobby = room(3, true);
    // Player 1 hosts a co-op link session (a battle starting): busy.
    state_packet(&mut lobby, 1, |p| {
        p.link = Some(LinkUp {
            epoch: 1,
            role: LINK_ROLE_PARENT,
            ack: 0,
            first_seq: 0,
            beacon: None,
            msgs: vec![],
        })
    });
    op(&mut lobby, 2, 1, SESSION_OP_REQUEST, SESSION_KIND_TRADE, 1);
    assert_eq!(gates(&mut lobby, 2)[6], SESSION_REASON_BUSY);
    // The link went quiet: free again.
    lobby.expire_links(Instant::now() + crate::lobby::LINK_SILENCE + Duration::from_secs(1));
    op(&mut lobby, 2, 2, SESSION_OP_REQUEST, SESSION_KIND_TRADE, 1);
    assert_eq!(gates(&mut lobby, 2)[2], SESSION_STATE_OUTGOING);
}
