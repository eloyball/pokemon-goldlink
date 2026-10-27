//! Virtual link routing (docs/MILESTONE3_PLAN.md §15).
//!
//! While a ROM runs a local-wireless session, its bridge sends a link trailer
//! every packet: its role, its beacon when it is a parent, and the messages of
//! its link block's out ring. The relay is the "air" between the games:
//!
//! - beacons are shared with the whole room (the Colosseum lists them);
//! - a child's CONNECT names the parent (the player id the fake scan put in
//!   the parent's address): the relay assigns the child an aid (1..=3) and
//!   forms the group;
//! - DATA from a parent goes to its accepted children (aid 0), DATA from a
//!   child to its parent (the child's aid);
//! - BYE, a new link block (epoch), silence or a disconnect dissolves a
//!   member's part of the group, and the others get BYE.
//!
//! Each direction of each hop is a reliable, ordered stream (go-back-N):
//! consecutive u16 sequence numbers, a cumulative ack (the next one
//! expected), out-of-order messages dropped, unacked ones resent.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

use tracing::{debug, info, warn};

use super::LobbyState;
use crate::protocol::{
    LINK_DATA_MAX, LINK_MAX_MSGS, LINK_SEND_BUDGET, LINK_MSG_ACCEPT, LINK_MSG_BYE, LINK_MSG_CONNECT, LINK_MSG_DATA,
    LINK_MSG_REFUSE, LINK_ROLE_PARENT, LinkBeacon, LinkDown, LinkMsg, LinkUp, MAX_PLAYERS,
    room_code_to_string,
};

/// A message not acked this long after it was sent goes again.
pub const LINK_RESEND: Duration = Duration::from_millis(150);
/// A link member whose bridge stopped sending link trailers this long left it.
pub const LINK_SILENCE: Duration = Duration::from_secs(2);
/// Messages waiting for one recipient; more means it stopped acking.
const LINK_QUEUE_MAX: usize = 256;
/// aids a parent hands out (vanilla: up to 4 players, the parent is 0).
const LINK_MAX_AID: u8 = 3;

struct Outgoing {
    seq: u16,
    msg: LinkMsg,
    sent: Option<Instant>,
}

/// One player's side of the virtual link.
#[derive(Default)]
pub(crate) struct LinkPeer {
    active: bool,
    epoch: u8,
    role: u8,
    in_next: u16,
    out: VecDeque<Outgoing>,
    out_next: u16,
    beacon: Option<LinkBeacon>,
    /// Child: the parent's slot and my aid under it.
    parent: Option<usize>,
    aid: u8,
    accepted: bool,
    last_trailer: Option<Instant>,
}

/// `a` comes before `b` in wrapping u16 order.
fn seq_before(a: u16, b: u16) -> bool {
    (b.wrapping_sub(a) as i16) > 0
}

impl LobbyState {
    /// The link trailer of a client packet (or its absence).
    pub(super) fn apply_link_up(&mut self, slot: usize, up: Option<LinkUp>, now: Instant) {
        let Some(up) = up else {
            return; // silence is handled by expire_links
        };
        let peer = &self.links[slot];
        if !peer.active || peer.epoch != up.epoch {
            if peer.active {
                self.leave_link(slot); // a new link block: the old session is over
            }
            self.links[slot] = LinkPeer {
                active: true,
                epoch: up.epoch,
                ..LinkPeer::default()
            };
        }
        let peer = &mut self.links[slot];
        peer.role = up.role;
        peer.last_trailer = Some(now);
        peer.beacon = if up.role == LINK_ROLE_PARENT {
            up.beacon
        } else {
            None
        };
        while let Some(front) = peer.out.front() {
            if !seq_before(front.seq, up.ack) {
                break;
            }
            peer.out.pop_front();
        }
        for (i, msg) in up.msgs.into_iter().enumerate() {
            let seq = up.first_seq.wrapping_add(i as u16);
            if seq != self.links[slot].in_next {
                debug!(
                    "link P{slot} up seq {seq} kind {} skipped (expecting {})",
                    msg.kind, self.links[slot].in_next
                );
                continue; // a duplicate, or after a gap: resent later
            }
            debug!("link P{slot} up seq {seq} kind {} len {}", msg.kind, msg.data.len());
            self.links[slot].in_next = seq.wrapping_add(1);
            self.route_link_msg(slot, msg);
        }
    }

    fn push_link_msg(&mut self, to: usize, kind: u8, aid: u8, data: Vec<u8>) {
        let peer = &mut self.links[to];
        if !peer.active {
            return;
        }
        if peer.out.len() >= LINK_QUEUE_MAX {
            let room_str = room_code_to_string(&self.room_code);
            warn!("[Room {room_str} | LINK] P{to} stopped acking; dropping it from the link");
            self.leave_link(to);
            self.links[to] = LinkPeer::default();
            return;
        }
        let seq = peer.out_next;
        peer.out_next = seq.wrapping_add(1);
        debug!("link P{to} queue seq {seq} kind {kind} len {}", data.len());
        peer.out.push_back(Outgoing {
            seq,
            msg: LinkMsg { kind, aid, data },
            sent: None,
        });
    }

    fn children_of(&self, parent: usize) -> Vec<usize> {
        (0..MAX_PLAYERS)
            .filter(|&c| self.links[c].active && self.links[c].parent == Some(parent))
            .collect()
    }

    fn route_link_msg(&mut self, from: usize, msg: LinkMsg) {
        let room_str = room_code_to_string(&self.room_code);
        match msg.kind {
            LINK_MSG_DATA => {
                if self.links[from].role == LINK_ROLE_PARENT {
                    for c in self.children_of(from) {
                        if self.links[c].accepted {
                            self.push_link_msg(c, LINK_MSG_DATA, 0, msg.data.clone());
                        }
                    }
                } else if let Some(p) = self.links[from].parent
                    && self.links[from].accepted
                {
                    let aid = self.links[from].aid;
                    self.push_link_msg(p, LINK_MSG_DATA, aid, msg.data);
                }
            }
            LINK_MSG_CONNECT => {
                // data: the parent's player id, then the connect data.
                let target = msg.data.first().map(|&t| t as usize);
                if self.links[from].parent.is_some() {
                    self.leave_link(from);
                }
                let free_aid = target
                    .filter(|&t| {
                        t < MAX_PLAYERS
                            && t != from
                            && self.links[t].active
                            && self.links[t].role == LINK_ROLE_PARENT
                    })
                    .and_then(|t| {
                        let used: Vec<u8> = self
                            .children_of(t)
                            .iter()
                            .map(|&c| self.links[c].aid)
                            .collect();
                        (1..=LINK_MAX_AID)
                            .find(|a| !used.contains(a))
                            .map(|a| (t, a))
                    });
                match free_aid {
                    Some((parent, aid)) => {
                        info!(
                            "[Room {room_str} | LINK] P{from} joins P{parent}'s link as aid {aid}"
                        );
                        let peer = &mut self.links[from];
                        peer.parent = Some(parent);
                        peer.aid = aid;
                        peer.accepted = false;
                        self.push_link_msg(parent, LINK_MSG_CONNECT, aid, msg.data[1..].to_vec());
                    }
                    None => self.push_link_msg(from, LINK_MSG_REFUSE, 0, Vec::new()),
                }
            }
            LINK_MSG_ACCEPT | LINK_MSG_REFUSE => {
                let Some(child) = self
                    .children_of(from)
                    .into_iter()
                    .find(|&c| self.links[c].aid == msg.aid)
                else {
                    return;
                };
                if msg.kind == LINK_MSG_ACCEPT {
                    self.links[child].accepted = true;
                } else {
                    self.links[child].parent = None;
                }
                self.push_link_msg(child, msg.kind, msg.aid, Vec::new());
            }
            LINK_MSG_BYE => self.leave_link(from),
            _ => {}
        }
    }

    /// `slot` leaves its link group: its children or its parent get BYE.
    fn leave_link(&mut self, slot: usize) {
        for c in self.children_of(slot) {
            self.links[c].parent = None;
            self.links[c].accepted = false;
            self.push_link_msg(c, LINK_MSG_BYE, 0, Vec::new());
        }
        if let Some(p) = self.links[slot].parent.take() {
            let aid = self.links[slot].aid;
            self.links[slot].accepted = false;
            self.push_link_msg(p, LINK_MSG_BYE, aid, Vec::new());
        }
    }

    /// Link members whose bridge went quiet (block gone without a BYE, or a
    /// stuck game) leave their group.
    pub(crate) fn expire_links(&mut self, now: Instant) {
        for slot in 0..MAX_PLAYERS {
            let peer = &self.links[slot];
            if peer.active
                && peer
                    .last_trailer
                    .is_some_and(|t| now.duration_since(t) > LINK_SILENCE)
            {
                self.leave_link(slot);
                self.links[slot] = LinkPeer::default();
            }
        }
    }

    /// Players who went offline leave their link groups.
    pub(super) fn drop_links_of_missing_players(&mut self) {
        for slot in 0..MAX_PLAYERS {
            if self.slots[slot].is_none() && self.links[slot].active {
                self.leave_link(slot);
                self.links[slot] = LinkPeer::default();
            }
        }
    }

    /// The link trailer for a recipient: its ack, the room's beacons, and the
    /// messages due (new, or unacked for LINK_RESEND), oldest first.
    pub(crate) fn link_down_for(&mut self, slot: usize, now: Instant) -> Option<LinkDown> {
        if !self.links[slot].active {
            return None;
        }
        let beacons = (0..MAX_PLAYERS)
            .filter(|&o| {
                o != slot && self.links[o].active && self.links[o].role == LINK_ROLE_PARENT
            })
            .filter_map(|o| self.links[o].beacon.clone().map(|b| (o as u8, b)))
            .collect();
        let peer = &mut self.links[slot];
        let due = peer
            .out
            .iter()
            .position(|o| o.sent.is_none_or(|t| now.duration_since(t) >= LINK_RESEND));
        let mut msgs = Vec::new();
        let mut first_seq = peer.out_next;
        if let Some(start) = due {
            first_seq = peer.out[start].seq;
            let mut budget = LINK_SEND_BUDGET;
            for o in peer.out.iter_mut().skip(start) {
                let size = 4 + o.msg.data.len().min(LINK_DATA_MAX);
                if msgs.len() >= LINK_MAX_MSGS || size > budget {
                    break;
                }
                budget -= size;
                o.sent = Some(now);
                msgs.push(o.msg.clone());
            }
        }
        if !msgs.is_empty() {
            debug!("link P{slot} down seq {first_seq}+{} (queued {})", msgs.len(), peer.out.len());
        }
        Some(LinkDown {
            ack: peer.in_next,
            first_seq,
            beacons,
            msgs,
        })
    }

    /// Tests: as if `slot`'s last link trailer came `by` earlier.
    #[cfg(test)]
    pub(crate) fn backdate_link(&mut self, slot: usize, by: Duration) {
        if let Some(t) = self.links[slot].last_trailer {
            self.links[slot].last_trailer = t.checked_sub(by);
        }
    }

    pub fn link_role(&self, slot: usize) -> u8 {
        if self.links[slot].active {
            self.links[slot].role
        } else {
            0
        }
    }

    pub fn link_parent(&self, slot: usize) -> Option<(usize, u8, bool)> {
        let p = &self.links[slot];
        p.parent.map(|parent| (parent, p.aid, p.accepted))
    }
}
