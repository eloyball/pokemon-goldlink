//! Virtual link (docs/MILESTONE3_PLAN.md §15): trailers, routing, the
//! reliable streams, and the group ending.

use std::time::{Duration, Instant};

use crate::lobby::{LINK_DEADLINE, LINK_RESEND, LINK_SILENCE, LINK_STALL, LobbyState};
use crate::protocol::{
    CLIENT_PACKET_SIZE, ClientToServerPacket, LINK_BEACON_MAX, LINK_DATA_MAX, LINK_MSG_ACCEPT,
    LINK_MSG_BYE, LINK_MSG_CONNECT, LINK_MSG_DATA, LINK_MSG_REFUSE, LINK_NOBODY, LINK_ROLE_CHILD,
    LINK_ROLE_PARENT, LinkBeacon, LinkDown, LinkMsg, LinkUp, SERVER_PACKET_SIZE, SessionOut,
    TRADE_MON_SIZE, TRADE_OUT_SIZE, TradeOut,
};
use crate::session_tests::{addr, packet, room_in, teamed_room};

fn msg(kind: u8, aid: u8, data: &[u8]) -> LinkMsg {
    LinkMsg {
        kind,
        aid,
        data: data.to_vec(),
    }
}

fn trade_out() -> TradeOut {
    TradeOut {
        state: 1,
        offer_id: 0,
        confirm_id: 0,
        trade_id: 0,
        mon: [0x11; TRADE_MON_SIZE],
    }
}

fn beacon() -> LinkBeacon {
    LinkBeacon {
        ggid: 0x333,
        entry: 1,
        seq: 7,
        data: vec![0x5A; 92],
    }
}

/// A client's link stream: what a bridge keeps between packets.
#[derive(Default)]
struct Client {
    epoch: u8,
    next_seq: u16,  // sequence number of the next message I send
    recv_next: u16, // next relay->me sequence number expected (my ack)
}

impl Client {
    /// One packet from player i with its link trailer: `msgs` are new
    /// messages (numbered from next_seq), `resend_from` repeats from there.
    fn send(
        &mut self,
        lobby: &mut LobbyState,
        i: usize,
        role: u8,
        b: Option<LinkBeacon>,
        msgs: Vec<LinkMsg>,
    ) {
        let first_seq = self.next_seq;
        self.next_seq = self.next_seq.wrapping_add(msgs.len() as u16);
        self.send_raw(lobby, i, role, b, first_seq, msgs);
    }

    fn send_raw(
        &mut self,
        lobby: &mut LobbyState,
        i: usize,
        role: u8,
        b: Option<LinkBeacon>,
        first_seq: u16,
        msgs: Vec<LinkMsg>,
    ) {
        let mut pkt = packet(i, SessionOut::default(), true);
        pkt.link = Some(LinkUp {
            epoch: self.epoch,
            role,
            ack: self.recv_next,
            first_seq,
            beacon: b,
            msgs,
        });
        lobby.handle_client_packet(addr(i), pkt).unwrap();
    }

    /// The relay's link trailer for player i, with the in-order messages
    /// accepted (and acked on the next send).
    /// Every batch carries every recipient's trailer, so reading one with a
    /// future "now" includes all unacked messages (the client keeps only
    /// the next in-order ones, as a bridge does).
    fn recv(&mut self, lobby: &mut LobbyState, i: usize) -> (Option<LinkDown>, Vec<LinkMsg>) {
        let down = lobby.link_down_for(i, Instant::now() + Duration::from_secs(60));
        let mut fresh = Vec::new();
        if let Some(d) = &down {
            for (k, m) in d.msgs.iter().enumerate() {
                if d.first_seq.wrapping_add(k as u16) == self.recv_next {
                    self.recv_next = self.recv_next.wrapping_add(1);
                    fresh.push(m.clone());
                }
            }
        }
        (down, fresh)
    }
}

fn down(lobby: &mut LobbyState, i: usize) -> Option<LinkDown> {
    let batch = lobby.build_broadcast_batch();
    let (_, buf) = batch
        .iter()
        .find(|(a, _)| *a == addr(i))
        .expect("player in the batch");
    buf.get(SERVER_PACKET_SIZE..).and_then(LinkDown::decode)
}

#[test]
fn test_link_trailers_round_trip_and_reject_bad_input() {
    let up = LinkUp {
        epoch: 3,
        role: LINK_ROLE_PARENT,
        ack: 0xFFFE,
        first_seq: 0xFFFF,
        beacon: Some(beacon()),
        msgs: vec![
            msg(LINK_MSG_DATA, 0, &[1; LINK_DATA_MAX]),
            msg(LINK_MSG_BYE, 0, &[]),
        ],
    };
    let mut buf = Vec::new();
    up.encode(&mut buf);
    assert_eq!(LinkUp::decode(&buf), Some(up.clone()));
    assert_eq!(LinkUp::decode(&buf[..buf.len() - 1]), None, "truncated");
    let mut bad = buf.clone();
    bad[12 + 4] = (LINK_BEACON_MAX + 1) as u8; // beacon length (after the header and GGID)
    assert_eq!(LinkUp::decode(&bad), None, "beacon too long");

    let down = LinkDown {
        ack: 5,
        first_seq: 9,
        epoch: Some(3),
        waiting: 0b10,
        called_off_by: Some(1),
        beacons: vec![(2, beacon())],
        msgs: vec![msg(LINK_MSG_ACCEPT, 1, &[])],
    };
    let mut buf = Vec::new();
    down.encode(&mut buf);
    assert_eq!(LinkDown::decode(&buf), Some(down.clone()));
    // A relay before the epoch wrote 0, 0 there: no epoch.
    let mut old = buf.clone();
    old[10] = 0;
    old[11] = 0;
    assert_eq!(LinkDown::decode(&old).unwrap().epoch, None);
    let nobody = LinkDown {
        called_off_by: None,
        ..down.clone()
    };
    let mut buf = Vec::new();
    nobody.encode(&mut buf);
    assert_eq!(buf[13], LINK_NOBODY);
    assert_eq!(LinkDown::decode(&buf), Some(nobody));

    // In a client packet: alone at byte 120, or after a trade trailer.
    let mut pkt: ClientToServerPacket = packet(0, SessionOut::default(), true);
    pkt.link = Some(up.clone());
    let decoded = ClientToServerPacket::decode_le(&pkt.encode_full()).unwrap();
    assert_eq!((decoded.trade, decoded.link.as_ref()), (None, Some(&up)));
    pkt.trade = Some(trade_out());
    let bytes = pkt.encode_full();
    assert!(bytes.len() > CLIENT_PACKET_SIZE + TRADE_OUT_SIZE);
    let decoded = ClientToServerPacket::decode_le(&bytes).unwrap();
    assert_eq!((decoded.trade, decoded.link), (Some(trade_out()), Some(up)));
}

/// Parent 0 (beaconing), child 1 joined and accepted, bystander 2 linked.
fn linked_room() -> (LobbyState, Client, Client, Client) {
    let mut lobby = room_in(3, true, None);
    let (mut p, mut c, mut s) = (Client::default(), Client::default(), Client::default());
    p.send(&mut lobby, 0, LINK_ROLE_PARENT, Some(beacon()), vec![]);
    s.send(&mut lobby, 2, LINK_ROLE_CHILD, None, vec![]);
    c.send(&mut lobby, 1, LINK_ROLE_CHILD, None, vec![]);

    // Everyone else with a link block sees the parent's beacon.
    let (d, _) = c.recv(&mut lobby, 1);
    assert_eq!(d.unwrap().beacons, vec![(0, beacon())]);
    let (d, _) = s.recv(&mut lobby, 2);
    assert_eq!(d.unwrap().beacons.len(), 1);
    assert!(
        p.recv(&mut lobby, 0).0.unwrap().beacons.is_empty(),
        "not its own"
    );

    // CONNECT names the parent; the relay strips that and assigns aid 1.
    c.send(
        &mut lobby,
        1,
        LINK_ROLE_CHILD,
        None,
        vec![msg(LINK_MSG_CONNECT, 0, &[0, 1, b'D', b'P', 0])],
    );
    let (_, got) = p.recv(&mut lobby, 0);
    assert_eq!(got, vec![msg(LINK_MSG_CONNECT, 1, &[1, b'D', b'P', 0])]);
    p.send(
        &mut lobby,
        0,
        LINK_ROLE_PARENT,
        Some(beacon()),
        vec![msg(LINK_MSG_ACCEPT, 1, &[])],
    );
    let (_, got) = c.recv(&mut lobby, 1);
    assert_eq!(got, vec![msg(LINK_MSG_ACCEPT, 1, &[])]);
    assert_eq!(lobby.link_parent(1), Some((0, 1, true)));
    (lobby, p, c, s)
}

#[test]
fn test_link_connect_and_data_routing() {
    let (mut lobby, mut p, mut c, mut s) = linked_room();

    // Parent frames reach the child (aid 0), child frames the parent (aid 1).
    p.send(
        &mut lobby,
        0,
        LINK_ROLE_PARENT,
        None,
        vec![msg(LINK_MSG_DATA, 0, &[0xAA; 192])],
    );
    c.send(
        &mut lobby,
        1,
        LINK_ROLE_CHILD,
        None,
        vec![msg(LINK_MSG_DATA, 1, &[0xBB; 38])],
    );
    assert_eq!(
        c.recv(&mut lobby, 1).1,
        vec![msg(LINK_MSG_DATA, 0, &[0xAA; 192])]
    );
    assert_eq!(
        p.recv(&mut lobby, 0).1,
        vec![msg(LINK_MSG_DATA, 1, &[0xBB; 38])]
    );
    assert!(
        s.recv(&mut lobby, 2).1.is_empty(),
        "the bystander gets nothing"
    );

    // A repeated packet (same sequence numbers) is applied once.
    c.send_raw(
        &mut lobby,
        1,
        LINK_ROLE_CHILD,
        None,
        c.next_seq - 1,
        vec![msg(LINK_MSG_DATA, 1, &[0xBB; 38])],
    );
    assert!(p.recv(&mut lobby, 0).1.is_empty());

    // Acked messages are not sent again; unacked ones only after LINK_RESEND.
    p.send(&mut lobby, 0, LINK_ROLE_PARENT, None, vec![]); // acks
    assert!(p.recv(&mut lobby, 0).0.unwrap().msgs.is_empty());
    p.send(
        &mut lobby,
        0,
        LINK_ROLE_PARENT,
        None,
        vec![msg(LINK_MSG_DATA, 0, &[1])],
    );
    let first = down(&mut lobby, 1).unwrap();
    assert_eq!(first.msgs.len(), 1);
    assert!(
        down(&mut lobby, 1).unwrap().msgs.is_empty(),
        "sent, not due yet"
    );
    let later = lobby
        .link_down_for(1, Instant::now() + LINK_RESEND)
        .unwrap();
    assert_eq!(
        (later.first_seq, later.msgs.len()),
        (first.first_seq, 1),
        "resent with its sequence number"
    );
}

#[test]
fn test_link_child_data_reaches_every_member() {
    let (mut lobby, mut p, mut c, mut s) = linked_room();
    s.send(
        &mut lobby,
        2,
        LINK_ROLE_CHILD,
        None,
        vec![msg(LINK_MSG_CONNECT, 0, &[0, 1, b'D', b'P', 0])],
    );
    p.recv(&mut lobby, 0);
    p.send(
        &mut lobby,
        0,
        LINK_ROLE_PARENT,
        None,
        vec![msg(LINK_MSG_ACCEPT, 2, &[])],
    );
    s.recv(&mut lobby, 2);

    // The message link: a child's commands reach the parent and the other child,
    // tagged with its aid; not the child itself.
    c.send(
        &mut lobby,
        1,
        LINK_ROLE_CHILD,
        None,
        vec![msg(LINK_MSG_DATA, 1, &[0xCC; 20])],
    );
    assert_eq!(
        p.recv(&mut lobby, 0).1,
        vec![msg(LINK_MSG_DATA, 1, &[0xCC; 20])]
    );
    assert_eq!(
        s.recv(&mut lobby, 2).1,
        vec![msg(LINK_MSG_DATA, 1, &[0xCC; 20])]
    );
    assert!(c.recv(&mut lobby, 1).1.is_empty(), "not its own");
}

#[test]
fn test_link_refusals_and_second_child() {
    let (mut lobby, mut p, _c, mut s) = linked_room();

    // A second child gets aid 2; the parent may refuse it.
    s.send(
        &mut lobby,
        2,
        LINK_ROLE_CHILD,
        None,
        vec![msg(LINK_MSG_CONNECT, 0, &[0, 1, b'D', b'P', 0])],
    );
    assert_eq!(
        p.recv(&mut lobby, 0).1,
        vec![msg(LINK_MSG_CONNECT, 2, &[1, b'D', b'P', 0])]
    );
    p.send(
        &mut lobby,
        0,
        LINK_ROLE_PARENT,
        None,
        vec![msg(LINK_MSG_REFUSE, 2, &[])],
    );
    assert_eq!(s.recv(&mut lobby, 2).1, vec![msg(LINK_MSG_REFUSE, 2, &[])]);
    assert_eq!(lobby.link_parent(2), None);

    // A CONNECT to a player who is not a parent is refused by the relay.
    s.send(
        &mut lobby,
        2,
        LINK_ROLE_CHILD,
        None,
        vec![msg(LINK_MSG_CONNECT, 0, &[1, 1, b'D', b'P', 0])],
    );
    assert_eq!(s.recv(&mut lobby, 2).1, vec![msg(LINK_MSG_REFUSE, 0, &[])]);
}

#[test]
fn test_link_bye_epoch_silence_and_disconnect() {
    // The parent leaves: the child gets BYE and its frames go nowhere.
    let (mut lobby, mut p, mut c, _) = linked_room();
    p.send(
        &mut lobby,
        0,
        LINK_ROLE_PARENT,
        None,
        vec![msg(LINK_MSG_BYE, 0, &[])],
    );
    assert_eq!(c.recv(&mut lobby, 1).1, vec![msg(LINK_MSG_BYE, 0, &[])]);
    assert_eq!(lobby.link_parent(1), None);
    c.send(
        &mut lobby,
        1,
        LINK_ROLE_CHILD,
        None,
        vec![msg(LINK_MSG_DATA, 1, &[1])],
    );
    assert!(p.recv(&mut lobby, 0).1.is_empty());

    // The child's bridge saw a new link block (epoch): the parent gets BYE (aid 1).
    let (mut lobby, mut p, mut c, _) = linked_room();
    c.epoch = c.epoch.wrapping_add(1);
    c.next_seq = 0;
    c.recv_next = 0;
    c.send(&mut lobby, 1, LINK_ROLE_CHILD, None, vec![]);
    assert_eq!(p.recv(&mut lobby, 0).1, vec![msg(LINK_MSG_BYE, 1, &[])]);

    // Milestone 6: a late trailer of the child's previous block, arriving after the new
    // block started (the internet reorders), is dropped: no BYE, the new block goes on.
    let (mut lobby, mut p, mut c, _) = linked_room();
    let old_epoch = c.epoch;
    c.epoch = c.epoch.wrapping_add(1);
    c.next_seq = 0;
    c.recv_next = 0;
    c.send(&mut lobby, 1, LINK_ROLE_CHILD, None, vec![]);
    assert_eq!(p.recv(&mut lobby, 0).1, vec![msg(LINK_MSG_BYE, 1, &[])]);
    c.send(
        &mut lobby,
        1,
        LINK_ROLE_CHILD,
        None,
        vec![msg(LINK_MSG_CONNECT, 0, &[0, 1, b'D', b'P', 0])],
    );
    assert_eq!(p.recv(&mut lobby, 0).1, vec![msg(LINK_MSG_CONNECT, 1, &[1, b'D', b'P', 0])]);
    p.send(&mut lobby, 0, LINK_ROLE_PARENT, Some(beacon()), vec![msg(LINK_MSG_ACCEPT, 1, &[])]);
    assert_eq!(c.recv(&mut lobby, 1).1, vec![msg(LINK_MSG_ACCEPT, 1, &[])]);
    let mut late = Client { epoch: old_epoch, next_seq: 7, recv_next: 9 };
    late.send(&mut lobby, 1, LINK_ROLE_CHILD, None, vec![msg(LINK_MSG_DATA, 1, &[9])]);
    assert!(p.recv(&mut lobby, 0).1.is_empty(), "no BYE, no stale data");
    let (d, _) = c.recv(&mut lobby, 1);
    assert_eq!(d.unwrap().epoch, Some(c.epoch), "the down stream names the current block");
    c.send(&mut lobby, 1, LINK_ROLE_CHILD, None, vec![msg(LINK_MSG_DATA, 1, &[5])]);
    assert_eq!(p.recv(&mut lobby, 0).1, vec![msg(LINK_MSG_DATA, 1, &[5])], "the new block still routes");

    // The parent's bridge went quiet: the child gets BYE after LINK_SILENCE.
    let (mut lobby, _p, mut c, _) = linked_room();
    lobby.backdate_link(0, LINK_SILENCE + Duration::from_millis(10));
    lobby.expire_links(Instant::now());
    assert_eq!(lobby.link_parent(1), None);
    c.send(&mut lobby, 1, LINK_ROLE_CHILD, None, vec![]);
    assert!(c.recv(&mut lobby, 1).1.contains(&msg(LINK_MSG_BYE, 0, &[])));

    // The parent disconnects: the child gets BYE.
    let (mut lobby, _p, mut c, _) = linked_room();
    lobby.remove_client_by_addr(addr(0));
    assert_eq!(c.recv(&mut lobby, 1).1, vec![msg(LINK_MSG_BYE, 0, &[])]);
}

#[test]
fn test_link_multi_battle_seats_teammates_together() {
    // Players 0 and 1 are a team; 0 hosts a 4-player multi (link mode 4).
    let mut lobby = teamed_room(4, None);
    let mut clients: Vec<Client> = (0..4).map(|_| Client::default()).collect();
    clients[0].send(&mut lobby, 0, LINK_ROLE_PARENT, Some(beacon()), vec![]);
    // An opponent connects first, then the teammate, then the other opponent.
    for (i, aid) in [(2, 1), (1, 2), (3, 3)] {
        clients[i].send(
            &mut lobby,
            i,
            LINK_ROLE_CHILD,
            None,
            vec![msg(LINK_MSG_CONNECT, 0, &[0, 4, b'D', b'P', 0])],
        );
        let (_, got) = clients[0].recv(&mut lobby, 0);
        assert_eq!(
            got,
            vec![msg(LINK_MSG_CONNECT, aid, &[4, b'D', b'P', 0])],
            "player {i}"
        );
    }

    // Outside a multi, children get aids in order.
    let mut lobby = teamed_room(3, None);
    let mut clients: Vec<Client> = (0..3).map(|_| Client::default()).collect();
    clients[0].send(&mut lobby, 0, LINK_ROLE_PARENT, Some(beacon()), vec![]);
    clients[1].send(
        &mut lobby,
        1,
        LINK_ROLE_CHILD,
        None,
        vec![msg(LINK_MSG_CONNECT, 0, &[0, 1, b'D', b'P', 0])],
    );
    let (_, got) = clients[0].recv(&mut lobby, 0);
    assert_eq!(got, vec![msg(LINK_MSG_CONNECT, 1, &[1, b'D', b'P', 0])]);
}

/// Link latency (rebalance plan §25): a link message goes out at once in a
/// flush packet to its recipient only, and the next tick does not send it
/// again (it is resent only after LINK_RESEND without an ack).
#[test]
fn test_link_flush_sends_new_messages_at_once() {
    let (mut lobby, mut p, _c, _s) = linked_room();
    // Nothing new: no flush packets.
    let _ = lobby.build_broadcast_batch();
    assert!(lobby.build_link_flush().is_empty(), "nothing unsent");

    p.send(
        &mut lobby,
        0,
        LINK_ROLE_PARENT,
        Some(beacon()),
        vec![msg(LINK_MSG_DATA, 0, &[7; 8])],
    );
    let flush = lobby.build_link_flush();
    let to: Vec<_> = flush.iter().map(|(a, _)| *a).collect();
    assert_eq!(to, vec![addr(1)], "only the child has the new message");
    let d = flush[0]
        .1
        .get(SERVER_PACKET_SIZE..)
        .and_then(LinkDown::decode)
        .unwrap();
    assert_eq!(d.msgs, vec![msg(LINK_MSG_DATA, 0, &[7; 8])]);

    // The tick right after: the child's trailer has no messages to repeat.
    assert!(down(&mut lobby, 1).unwrap().msgs.is_empty(), "already sent");
    assert!(lobby.build_link_flush().is_empty());
}

/// The referee (dev/docs/LINK_BATTLE_REWORK.md part 3): a quiet client is
/// waited for, then called off at the deadline; it gets its BYE when it comes back.
#[test]
fn test_link_referee_waits_then_calls_off() {
    // The child's whole client goes quiet (a paused game): the battle waits, and the
    // parent is told who it waits for.
    let (mut lobby, mut p, mut c, _) = linked_room();
    lobby.set_client_timeout(Duration::from_secs(5)); // the relay's default
    c.send(
        &mut lobby,
        1,
        LINK_ROLE_CHILD,
        None,
        vec![msg(LINK_MSG_DATA, 1, &[1])],
    );
    assert_eq!(p.recv(&mut lobby, 0).1, vec![msg(LINK_MSG_DATA, 1, &[1])]);
    let parent_waits_for = |lobby: &mut LobbyState| {
        lobby
            .link_down_for(0, Instant::now())
            .map(|d| (d.waiting, d.called_off_by))
    };
    assert_eq!(parent_waits_for(&mut lobby), Some((0, None)));
    lobby.backdate_client(1, LINK_STALL + Duration::from_millis(10));
    assert_eq!(parent_waits_for(&mut lobby), Some((0b10, None)));
    lobby.backdate_client(1, LINK_DEADLINE - LINK_STALL - Duration::from_secs(1));
    lobby.expire_links(Instant::now());
    lobby.prune_stale_clients();
    assert_eq!(
        lobby.link_parent(1),
        Some((0, 1, true)),
        "still waiting before the deadline"
    );
    assert!(
        lobby.is_slot_occupied(1),
        "the room keeps a quiet link member's slot"
    );

    // Back in time: the stream goes on where it stopped.
    c.send(
        &mut lobby,
        1,
        LINK_ROLE_CHILD,
        None,
        vec![msg(LINK_MSG_DATA, 1, &[2])],
    );
    assert_eq!(p.recv(&mut lobby, 0).1, vec![msg(LINK_MSG_DATA, 1, &[2])]);
    assert_eq!(parent_waits_for(&mut lobby), Some((0, None)));

    // Past the deadline: called off. The parent gets BYE and who; the child, still
    // quiet, keeps its slot for its BYE.
    lobby.backdate_client(1, LINK_DEADLINE + Duration::from_secs(1));
    lobby.expire_links(Instant::now());
    lobby.prune_stale_clients();
    assert_eq!(lobby.link_parent(1), None);
    assert!(lobby.is_slot_occupied(1));
    let (d, fresh) = p.recv(&mut lobby, 0);
    assert_eq!(fresh, vec![msg(LINK_MSG_BYE, 1, &[])]);
    assert_eq!(d.unwrap().called_off_by, Some(1));

    // The child comes back with the same link block and numbers the relay forgot:
    // it gets BYE (the parent is gone) on its own stream, and its stale data goes nowhere.
    c.send(
        &mut lobby,
        1,
        LINK_ROLE_CHILD,
        None,
        vec![msg(LINK_MSG_DATA, 1, &[3])],
    );
    let (d, fresh) = c.recv(&mut lobby, 1);
    assert_eq!(fresh, vec![msg(LINK_MSG_BYE, 0, &[])]);
    assert_eq!(d.unwrap().called_off_by, Some(1));
    assert!(p.recv(&mut lobby, 0).1.is_empty());

    // The same, but the client read relay packets queued from before the call-off after
    // the trailer the relay picked its stream up from: its ack runs past the BYE's number.
    let (mut lobby, mut p, mut c, _) = linked_room();
    lobby.set_client_timeout(Duration::from_secs(5));
    p.send(
        &mut lobby,
        0,
        LINK_ROLE_PARENT,
        None,
        vec![msg(LINK_MSG_DATA, 0, &[7]), msg(LINK_MSG_DATA, 0, &[8])],
    );
    let stale = lobby.link_down_for(1, Instant::now()).unwrap(); // still in flight
    lobby.backdate_client(1, LINK_DEADLINE + Duration::from_secs(1));
    lobby.expire_links(Instant::now());
    c.send(&mut lobby, 1, LINK_ROLE_CHILD, None, vec![]); // back: the relay picks it up
    c.recv_next = c.recv_next.wrapping_add(stale.msgs.len() as u16); // then reads the stale ones
    c.send(&mut lobby, 1, LINK_ROLE_CHILD, None, vec![]);
    let (_, fresh) = c.recv(&mut lobby, 1);
    assert_eq!(
        fresh,
        vec![msg(LINK_MSG_BYE, 0, &[])],
        "the BYE is renumbered, not lost"
    );

    // Any link block holds the slot past the usual timeout (the bystander child too);
    // a player without one is pruned at it.
    lobby.backdate_client(2, Duration::from_secs(6));
    lobby.prune_stale_clients();
    assert!(lobby.is_slot_occupied(2));
    let no_trailer = packet(2, SessionOut::default(), true); // its link block closed
    lobby.handle_client_packet(addr(2), no_trailer).unwrap();
    lobby.backdate_link(2, LINK_SILENCE + Duration::from_secs(1));
    lobby.expire_links(Instant::now());
    lobby.backdate_client(2, Duration::from_secs(6));
    lobby.prune_stale_clients();
    assert!(!lobby.is_slot_occupied(2));

    // The parent goes quiet instead: every child gets BYE at the deadline, and the
    // parent its children's BYEs when it comes back.
    let (mut lobby, mut p, mut c, _) = linked_room();
    lobby.backdate_client(0, LINK_DEADLINE + Duration::from_secs(1));
    lobby.expire_links(Instant::now());
    let (d, fresh) = c.recv(&mut lobby, 1);
    assert_eq!(fresh, vec![msg(LINK_MSG_BYE, 0, &[])]);
    assert_eq!(d.unwrap().called_off_by, Some(0));
    p.send(&mut lobby, 0, LINK_ROLE_PARENT, None, vec![]);
    assert_eq!(p.recv(&mut lobby, 0).1, vec![msg(LINK_MSG_BYE, 1, &[])]);

    // A quiet member that left the room for good is called off the same way.
    let (mut lobby, _p, mut c, _) = linked_room();
    lobby.remove_client_by_addr(addr(0));
    let (d, fresh) = c.recv(&mut lobby, 1);
    assert_eq!(fresh, vec![msg(LINK_MSG_BYE, 0, &[])]);
    assert_eq!(d.unwrap().called_off_by, Some(0));

    // A new link block (another epoch) is a new session: no stale BYE.
    let (mut lobby, mut p, _c, _) = linked_room();
    lobby.backdate_client(0, LINK_DEADLINE + Duration::from_secs(1));
    lobby.expire_links(Instant::now());
    p.epoch = p.epoch.wrapping_add(1);
    p.next_seq = 0;
    p.recv_next = 0;
    p.send(&mut lobby, 0, LINK_ROLE_PARENT, None, vec![]);
    let (d, fresh) = p.recv(&mut lobby, 0);
    assert!(fresh.is_empty());
    assert_eq!(d.unwrap().called_off_by, None);
}

/// Protocol 9 (rework plan §7.1): a QUIC member's link messages travel on its link stream;
/// a UDP member in the same group keeps the trailer's go-back-N.
#[test]
fn test_link_streams_beside_the_go_back_n() {
    use crate::lobby::{LinkStreamEvent, LinkStreamOut};
    let out = |lobby: &mut LobbyState| -> Vec<(usize, LinkMsg)> {
        lobby
            .take_link_stream_out()
            .into_iter()
            .map(|LinkStreamOut { addr: a, msg, .. }| {
                let slot = (0..6).find(|&i| addr(i) == a).expect("a player's address");
                (slot, msg)
            })
            .collect()
    };
    let mut lobby = room_in(2, true, None);
    let (mut p, mut c) = (Client::default(), Client::default());
    // The parent's trailer comes first (the stream opens a moment later); the child's CONNECT
    // is queued for it until then, and goes on the stream when it opens.
    p.send(&mut lobby, 0, LINK_ROLE_PARENT, Some(beacon()), vec![]);
    c.send(&mut lobby, 1, LINK_ROLE_CHILD, None, vec![]);
    c.send(
        &mut lobby,
        1,
        LINK_ROLE_CHILD,
        None,
        vec![msg(LINK_MSG_CONNECT, 0, &[0, 1, b'D', b'P', 0])],
    );
    assert!(out(&mut lobby).is_empty());
    lobby.link_stream_event(addr(0), LinkStreamEvent::Open(p.epoch));
    assert_eq!(
        out(&mut lobby),
        vec![(0, msg(LINK_MSG_CONNECT, 1, &[1, b'D', b'P', 0]))]
    );
    // Its trailers carry no messages any more.
    assert!(p.recv(&mut lobby, 0).1.is_empty());

    // The parent's ACCEPT and DATA from its stream reach the UDP child's trailer, the child's
    // DATA the parent's stream.
    lobby.link_stream_event(
        addr(0),
        LinkStreamEvent::Msg(p.epoch, msg(LINK_MSG_ACCEPT, 1, &[])),
    );
    lobby.link_stream_event(
        addr(0),
        LinkStreamEvent::Msg(p.epoch, msg(LINK_MSG_DATA, 0, &[7])),
    );
    assert_eq!(
        c.recv(&mut lobby, 1).1,
        vec![msg(LINK_MSG_ACCEPT, 1, &[]), msg(LINK_MSG_DATA, 0, &[7])]
    );
    assert_eq!(lobby.link_parent(1), Some((0, 1, true)));
    c.send(
        &mut lobby,
        1,
        LINK_ROLE_CHILD,
        None,
        vec![msg(LINK_MSG_DATA, 1, &[8])],
    );
    assert_eq!(out(&mut lobby), vec![(0, msg(LINK_MSG_DATA, 1, &[8]))]);

    // An earlier block's message is dropped; so is one for a stream that isn't open.
    lobby.link_stream_event(
        addr(0),
        LinkStreamEvent::Msg(p.epoch.wrapping_sub(1), msg(LINK_MSG_DATA, 0, &[9])),
    );
    assert!(c.recv(&mut lobby, 1).1.is_empty());

    // Finishing the stream is the parent's BYE.
    lobby.link_stream_event(addr(0), LinkStreamEvent::Closed(p.epoch));
    assert_eq!(c.recv(&mut lobby, 1).1, vec![msg(LINK_MSG_BYE, 0, &[])]);
    assert_eq!(lobby.link_parent(1), None);

    // The referee: a quiet QUIC child gets its BYE on its stream at once (QUIC delivers it
    // when it comes back); its status line still learns who called it off.
    let mut lobby = room_in(2, true, None);
    let (mut p, mut c) = (Client::default(), Client::default());
    p.send(&mut lobby, 0, LINK_ROLE_PARENT, Some(beacon()), vec![]);
    c.send(&mut lobby, 1, LINK_ROLE_CHILD, None, vec![]);
    lobby.link_stream_event(addr(1), LinkStreamEvent::Open(c.epoch));
    lobby.link_stream_event(
        addr(1),
        LinkStreamEvent::Msg(c.epoch, msg(LINK_MSG_CONNECT, 0, &[0, 1, b'D', b'P', 0])),
    );
    assert_eq!(
        p.recv(&mut lobby, 0).1,
        vec![msg(LINK_MSG_CONNECT, 1, &[1, b'D', b'P', 0])]
    );
    p.send(
        &mut lobby,
        0,
        LINK_ROLE_PARENT,
        Some(beacon()),
        vec![msg(LINK_MSG_ACCEPT, 1, &[])],
    );
    assert_eq!(out(&mut lobby), vec![(1, msg(LINK_MSG_ACCEPT, 1, &[]))]);
    lobby.backdate_client(1, LINK_DEADLINE + Duration::from_secs(1));
    lobby.expire_links(Instant::now());
    assert_eq!(out(&mut lobby), vec![(1, msg(LINK_MSG_BYE, 0, &[]))]);
    assert_eq!(p.recv(&mut lobby, 0).1, vec![msg(LINK_MSG_BYE, 1, &[])]);
    c.send(&mut lobby, 1, LINK_ROLE_CHILD, None, vec![]); // back: no BYE twice
    let (d, fresh) = c.recv(&mut lobby, 1);
    assert!(fresh.is_empty());
    assert_eq!(d.unwrap().called_off_by, Some(1));
}
