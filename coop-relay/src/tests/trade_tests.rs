//! Trading (docs/MILESTONE3_PLAN.md, Phase 2): escrow, commit journal,
//! cancellation and recovery.

use std::fs;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use crate::lobby::{LobbyState, TRADE_NOTICE_TTL, TRADE_SILENCE_TIMEOUT};
use crate::protocol::{
    CHALLENGE_DOWN_SIZE, ChallengeDown, PLAYER_STATE_SIZE, REMOTE_SLOTS, SERVER_PACKET_SIZE,
    SESSION_KIND_TRADE, SESSION_OP_ACCEPT, SESSION_OP_REQUEST, SessionOut, TRADE_ABORT_CANCELLED,
    TRADE_ABORT_LEFT, TRADE_IN_ABORTED, TRADE_IN_COMMIT, TRADE_IN_OFFERED, TRADE_IN_SIZE,
    TRADE_IN_WAITING, TRADE_MON_SIZE, TRADE_OUT_CANCEL, TRADE_OUT_CONFIRMED, TRADE_OUT_DONE,
    TRADE_OUT_OFFERED, TradeIn, TradeOut, mon_pid,
};
use crate::session_tests::{addr, op, packet, room_in};

const PID_A: u32 = 0xAAAA_0001;
const PID_B: u32 = 0xBBBB_0002;

fn mon(pid: u32, fill: u8) -> [u8; TRADE_MON_SIZE] {
    let mut m = [fill; TRADE_MON_SIZE];
    m[0..4].copy_from_slice(&pid.to_le_bytes());
    m
}

/// Two players (0 asks 1) with an open trade.
fn open_trade(dir: Option<PathBuf>) -> LobbyState {
    let mut lobby = room_in(2, true, dir);
    op(&mut lobby, 0, 1, SESSION_OP_REQUEST, SESSION_KIND_TRADE, 1);
    op(&mut lobby, 1, 1, SESSION_OP_ACCEPT, 0, 0);
    lobby
}

/// One packet from player i carrying its trade trailer (the session block
/// repeats the last operation, so nothing is re-applied).
fn send(
    lobby: &mut LobbyState,
    i: usize,
    state: u8,
    offer_id: u8,
    confirm_id: u8,
    trade_id: u32,
    m: [u8; TRADE_MON_SIZE],
) {
    let mut pkt = packet(
        i,
        SessionOut {
            seq: 1,
            ..SessionOut::default()
        },
        true,
    );
    pkt.trade = Some(TradeOut {
        state,
        offer_id,
        confirm_id,
        trade_id,
        mon: m,
    });
    lobby.handle_client_packet(addr(i), pkt).unwrap();
}

/// (gates.trade_pending, the trailer) for player i.
fn view(lobby: &mut LobbyState, i: usize) -> (u8, Option<TradeIn>) {
    let batch = lobby.build_broadcast_batch();
    let (_, buf) = batch
        .iter()
        .find(|(a, _)| *a == addr(i))
        .expect("player in the batch");
    let pending = buf[16 + REMOTE_SLOTS * PLAYER_STATE_SIZE + 23];
    // Milestone 5: the challenge trailer always ends the packet.
    let tail = buf.len() - CHALLENGE_DOWN_SIZE;
    assert!(
        ChallengeDown::decode(&buf[tail..]).is_some(),
        "challenge trailer last"
    );
    let buf = &buf[..tail];
    let trailer = (buf.len() == SERVER_PACKET_SIZE + TRADE_IN_SIZE)
        .then(|| TradeIn::decode_le(&buf[SERVER_PACKET_SIZE..]));
    assert!(
        buf.len() == SERVER_PACKET_SIZE || trailer.is_some(),
        "bad packet length {}",
        buf.len()
    );
    assert_eq!(
        pending,
        u8::from(trailer.is_some()),
        "trade_pending follows the trailer"
    );
    (pending, trailer)
}

fn trailer(lobby: &mut LobbyState, i: usize) -> Option<TradeIn> {
    view(lobby, i).1
}

#[test]
fn test_offers_confirmations_commit_and_journal() {
    let dir = PathBuf::from("target/test_saves_trade_commit");
    let _ = fs::remove_dir_all(&dir);
    let mut lobby = open_trade(Some(dir.clone()));
    assert_eq!(trailer(&mut lobby, 0).unwrap().state, TRADE_IN_WAITING);
    assert_eq!(trailer(&mut lobby, 1).unwrap().state, TRADE_IN_WAITING);

    // Offers cross through the escrow.
    send(&mut lobby, 0, TRADE_OUT_OFFERED, 1, 0, 0, mon(PID_A, 0x11));
    let t1 = trailer(&mut lobby, 1).unwrap();
    assert_eq!(
        (t1.state, t1.partner_offer_id, t1.peer_slot),
        (TRADE_IN_OFFERED, 1, 0)
    );
    assert_eq!(t1.mon, mon(PID_A, 0x11));
    let t0 = trailer(&mut lobby, 0).unwrap();
    assert_eq!((t0.state, t0.my_offer_id), (TRADE_IN_WAITING, 1));
    send(&mut lobby, 1, TRADE_OUT_OFFERED, 1, 0, 0, mon(PID_B, 0x22));
    assert_eq!(trailer(&mut lobby, 0).unwrap().mon, mon(PID_B, 0x22));

    // One confirmation is not enough; the other side sees it.
    send(
        &mut lobby,
        0,
        TRADE_OUT_CONFIRMED,
        1,
        1,
        0,
        mon(PID_A, 0x11),
    );
    let t1 = trailer(&mut lobby, 1).unwrap();
    assert_eq!((t1.state, t1.partner_confirmed), (TRADE_IN_OFFERED, 1));
    assert!(lobby.pending_trades().is_empty());

    // Both: committed, journaled on disk with both Pokemon.
    send(
        &mut lobby,
        1,
        TRADE_OUT_CONFIRMED,
        1,
        1,
        0,
        mon(PID_B, 0x22),
    );
    let c0 = trailer(&mut lobby, 0).unwrap();
    let c1 = trailer(&mut lobby, 1).unwrap();
    assert_eq!(
        (c0.state, c0.give_pid, c0.mon),
        (TRADE_IN_COMMIT, PID_A, mon(PID_B, 0x22))
    );
    assert_eq!(
        (c1.state, c1.give_pid, c1.mon),
        (TRADE_IN_COMMIT, PID_B, mon(PID_A, 0x11))
    );
    assert!(c0.trade_id != 0 && c0.trade_id == c1.trade_id);
    assert_eq!(lobby.pending_trades().len(), 1);
    assert_eq!(lobby.active_trade_count(), 0);
    let file = fs::read_to_string(dir.join("TEST01.json")).unwrap();
    assert!(file.contains("pending_trades") && file.contains(&format!("{}", c0.trade_id)));

    // A relay restart keeps the commit; each game gets it again on rejoin.
    drop(lobby);
    let mut lobby = LobbyState::with_room(
        *b"TEST01",
        0,
        None,
        Duration::from_secs(60),
        2,
        false,
        Some(dir.clone()),
    )
    .unwrap();
    for i in 0..2 {
        lobby
            .handle_client_packet(addr(i), packet(i, SessionOut::default(), true))
            .unwrap();
    }
    let r0 = trailer(&mut lobby, 0).unwrap();
    assert_eq!(
        (r0.state, r0.trade_id, r0.give_pid),
        (TRADE_IN_COMMIT, c0.trade_id, PID_A)
    );

    // DONE after each save; the journal goes once both saved.
    send(
        &mut lobby,
        0,
        TRADE_OUT_DONE,
        0,
        0,
        c0.trade_id,
        [0; TRADE_MON_SIZE],
    );
    assert_eq!(view(&mut lobby, 0), (0, None));
    assert_eq!(trailer(&mut lobby, 1).unwrap().state, TRADE_IN_COMMIT);
    send(
        &mut lobby,
        1,
        TRADE_OUT_DONE,
        0,
        0,
        0xDEAD,
        [0; TRADE_MON_SIZE],
    ); // wrong id: ignored
    assert_eq!(lobby.pending_trades().len(), 1);
    send(
        &mut lobby,
        1,
        TRADE_OUT_DONE,
        0,
        0,
        c1.trade_id,
        [0; TRADE_MON_SIZE],
    );
    assert!(lobby.pending_trades().is_empty());
    assert_eq!(view(&mut lobby, 1), (0, None));
    let file = fs::read_to_string(dir.join("TEST01.json")).unwrap();
    assert!(!file.contains(&format!("{}", c0.trade_id)));
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn test_a_new_offer_voids_confirmations() {
    let mut lobby = open_trade(None);
    send(&mut lobby, 0, TRADE_OUT_OFFERED, 1, 0, 0, mon(PID_A, 1));
    send(&mut lobby, 1, TRADE_OUT_OFFERED, 1, 0, 0, mon(PID_B, 1));
    send(&mut lobby, 0, TRADE_OUT_CONFIRMED, 1, 1, 0, mon(PID_A, 1));
    // B swaps its offer: A's yes was for the old one.
    send(&mut lobby, 1, TRADE_OUT_OFFERED, 2, 0, 0, mon(PID_B + 1, 2));
    send(
        &mut lobby,
        1,
        TRADE_OUT_CONFIRMED,
        2,
        1,
        0,
        mon(PID_B + 1, 2),
    );
    assert!(lobby.pending_trades().is_empty());
    // A confirmation for an offer that does not exist counts for nothing.
    send(&mut lobby, 0, TRADE_OUT_CONFIRMED, 1, 9, 0, mon(PID_A, 1));
    assert!(lobby.pending_trades().is_empty());
    send(&mut lobby, 0, TRADE_OUT_CONFIRMED, 1, 2, 0, mon(PID_A, 1));
    let c0 = trailer(&mut lobby, 0).unwrap();
    assert_eq!((c0.state, c0.mon), (TRADE_IN_COMMIT, mon(PID_B + 1, 2)));
    assert_eq!(mon_pid(&c0.mon), PID_B + 1);
}

#[test]
fn test_cancel_tells_the_partner_until_acknowledged() {
    let mut lobby = open_trade(None);
    send(&mut lobby, 0, TRADE_OUT_OFFERED, 1, 0, 0, mon(PID_A, 1));
    send(
        &mut lobby,
        1,
        TRADE_OUT_CANCEL,
        0,
        0,
        0,
        [0; TRADE_MON_SIZE],
    );
    assert_eq!(lobby.active_trade_count(), 0);
    let t0 = trailer(&mut lobby, 0).unwrap();
    assert_eq!(
        (t0.state, t0.reason),
        (TRADE_IN_ABORTED, TRADE_ABORT_CANCELLED)
    );
    assert_eq!(
        view(&mut lobby, 1),
        (0, None),
        "the canceller reads nothing"
    );
    // Still busy until acknowledged.
    op(&mut lobby, 0, 2, SESSION_OP_REQUEST, SESSION_KIND_TRADE, 1);
    assert_eq!(
        crate::session_tests::gates(&mut lobby, 0)[6],
        crate::protocol::SESSION_REASON_BUSY
    );
    send(
        &mut lobby,
        0,
        TRADE_OUT_CANCEL,
        1,
        0,
        0,
        [0; TRADE_MON_SIZE],
    );
    assert_eq!(view(&mut lobby, 0), (0, None));
}

#[test]
fn test_disconnect_silence_and_notice_expiry() {
    let mut lobby = open_trade(None);
    lobby.remove_client_by_addr(addr(1));
    let t0 = trailer(&mut lobby, 0).unwrap();
    assert_eq!((t0.state, t0.reason), (TRADE_IN_ABORTED, TRADE_ABORT_LEFT));

    // A game that stops sending its trade (crash, lost packets) ends it.
    let mut lobby = open_trade(None);
    send(&mut lobby, 0, TRADE_OUT_OFFERED, 1, 0, 0, mon(PID_A, 1));
    let later = Instant::now() + TRADE_SILENCE_TIMEOUT;
    lobby.expire_trades(later);
    assert_eq!(lobby.active_trade_count(), 0);
    assert_eq!(trailer(&mut lobby, 0).unwrap().state, TRADE_IN_ABORTED);
    // Unacknowledged notices fade.
    lobby.expire_trades(later + TRADE_NOTICE_TTL + Duration::from_secs(1));
    assert_eq!(view(&mut lobby, 0), (0, None));
    assert_eq!(view(&mut lobby, 1), (0, None));
}

#[test]
fn test_offer_without_personality_is_ignored() {
    let mut lobby = open_trade(None);
    send(
        &mut lobby,
        0,
        TRADE_OUT_OFFERED,
        1,
        0,
        0,
        [0; TRADE_MON_SIZE],
    );
    assert_eq!(trailer(&mut lobby, 1).unwrap().state, TRADE_IN_WAITING);
}
