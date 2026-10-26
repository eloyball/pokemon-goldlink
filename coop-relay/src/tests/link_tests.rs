//! Virtual link (docs/MILESTONE3_PLAN.md §15): trailers, routing, the
//! reliable streams, and the group ending.

use std::time::{Duration, Instant};

use crate::lobby::{LINK_RESEND, LINK_SILENCE, LobbyState};
use crate::protocol::{
    CLIENT_PACKET_SIZE, ClientToServerPacket, LINK_BEACON_MAX, LINK_DATA_MAX, LINK_MSG_ACCEPT,
    LINK_MSG_BYE, LINK_MSG_CONNECT, LINK_MSG_DATA, LINK_MSG_REFUSE, LINK_ROLE_CHILD,
    LINK_ROLE_PARENT, LinkBeacon, LinkDown, LinkMsg, LinkUp, SERVER_PACKET_SIZE, SessionOut,
    TRADE_MON_SIZE, TRADE_OUT_SIZE, TradeOut,
};
use crate::session_tests::{addr, packet, room_in};

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
        beacons: vec![(2, beacon())],
        msgs: vec![msg(LINK_MSG_ACCEPT, 1, &[])],
    };
    let mut buf = Vec::new();
    down.encode(&mut buf);
    assert_eq!(LinkDown::decode(&buf), Some(down));

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
