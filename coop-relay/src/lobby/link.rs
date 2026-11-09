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
//!   child to its parent and the other accepted children (the child's aid):
//!   every member gets every member's comm commands (the ROM's message link);
//! - BYE, a new link block (epoch), silence or a disconnect dissolves a
//!   member's part of the group, and the others get BYE.
//!
//! The referee (dev/docs/LINK_BATTLE_REWORK.md part 3): a member whose whole
//! client went quiet (a paused game, its network down) is waited for, and the
//! others are told who they wait for; past `LINK_DEADLINE`, or when it leaves
//! the room, its battle is called off. The quiet member learns it too when it
//! comes back with the same link block: the relay forgot its stream, so it
//! picks the stream up at the client's numbers and sends the BYEs then.
//!
//! Each direction of each hop is a reliable, ordered stream (go-back-N):
//! consecutive u16 sequence numbers, a cumulative ack (the next one
//! expected), out-of-order messages dropped, unacked ones resent. Each block
//! restarts its streams at 0, so both ends tag them with the block's epoch and
//! drop a late trailer of an earlier block.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

use tracing::{debug, info, warn};

use super::LobbyState;
use crate::protocol::{
    LINK_DATA_MAX, LINK_MAX_MSGS, LINK_MODE_MULTI_BATTLE, LINK_MSG_ACCEPT, LINK_MSG_BYE,
    LINK_MSG_CONNECT, LINK_MSG_DATA, LINK_MSG_REFUSE, LINK_ROLE_PARENT, LINK_SEND_BUDGET,
    LinkBeacon, LinkDown, LinkMsg, LinkUp, MAX_PLAYERS, room_code_to_string,
};

/// A message not acked this long after it was sent goes again.
pub const LINK_RESEND: Duration = Duration::from_millis(150);
/// A link member whose client still talks but sent no link trailer this long
/// closed its link block (its session ended without a BYE reaching us).
pub const LINK_SILENCE: Duration = Duration::from_secs(2);
/// A link member whose whole client is quiet this long is gone: the referee
/// calls its battle off. Until then the battle waits for it.
pub const LINK_DEADLINE: Duration = Duration::from_secs(30);
/// A member quiet this long is named to the others ("Waiting for ...").
pub const LINK_STALL: Duration = Duration::from_secs(1);
/// How long a called-off member's BYEs wait for it to come back.
const LINK_CALLED_OFF_KEEP: Duration = Duration::from_secs(120);
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
    /// The player whose loss called this member's battle off (sent in every
    /// trailer until the link block ends, for the bridge's status line).
    called_off_by: Option<u8>,
}

/// A member that was quiet when the referee called its battle off: the BYEs it
/// gets if it comes back with the same link block (`epoch`).
pub(crate) struct CalledOff {
    epoch: u8,
    byes: Vec<u8>,
    by: u8,
    at: Instant,
}

/// `a` comes before `b` in wrapping u16 order.
fn seq_before(a: u16, b: u16) -> bool {
    (b.wrapping_sub(a) as i16) > 0
}

impl LobbyState {
    /// In a co-op link session (a battle starting or running).
    pub(super) fn link_busy(&self, slot: usize) -> bool {
        self.links[slot].active
    }

    /// The link trailer of a client packet (or its absence).
    pub(super) fn apply_link_up(&mut self, slot: usize, up: Option<LinkUp>, now: Instant) {
        let Some(up) = up else {
            return; // silence is handled by expire_links
        };
        let peer = &self.links[slot];
        // The bridge counts its link blocks (epochs) up. A trailer of an earlier block
        // arriving late (the internet reorders) is stale: dropping it, not restarting the
        // session on it, keeps the current one alive (Milestone 6: the PvP freeze over the
        // VPS; locally nothing ever arrived late).
        if peer.active && peer.epoch != up.epoch && (up.epoch.wrapping_sub(peer.epoch) as i8) < 0 {
            debug!("link P{slot} stale trailer of epoch {} (now {})", up.epoch, peer.epoch);
            return;
        }
        if !peer.active || peer.epoch != up.epoch {
            if peer.active {
                self.leave_link(slot); // a new link block: the old session is over
            }
            self.links[slot] = LinkPeer {
                active: true,
                epoch: up.epoch,
                ..LinkPeer::default()
            };
            if let Some(off) = self.link_called_off[slot].take()
                && off.epoch == up.epoch
            {
                // Back after its battle was called off: go on from the client's own
                // numbers (what it sends now was for a group that is gone), then BYE.
                let room_str = room_code_to_string(&self.room_code);
                info!("[Room {room_str} | LINK] P{slot} is back: its battle was called off");
                let peer = &mut self.links[slot];
                peer.in_next = up.first_seq.wrapping_add(up.msgs.len() as u16);
                peer.out_next = up.ack;
                peer.called_off_by = Some(off.by);
                for aid in off.byes {
                    self.push_link_msg(slot, LINK_MSG_BYE, aid, Vec::new());
                }
            }
        }
        let peer = &mut self.links[slot];
        peer.role = up.role;
        peer.last_trailer = Some(now);
        // An ack past anything numbered here: after a called-off member came back, the
        // client went on reading relay packets queued from before (its numbers moved on
        // after the trailer we picked its stream up from). Go on at its number, so the
        // queued BYEs aren't taken as read.
        if seq_before(peer.out_next, up.ack) {
            for (i, o) in peer.out.iter_mut().enumerate() {
                o.seq = up.ack.wrapping_add(i as u16);
                o.sent = None;
            }
            peer.out_next = up.ack.wrapping_add(peer.out.len() as u16);
        }
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
            debug!(
                "link P{slot} up seq {seq} kind {} len {}",
                msg.kind,
                msg.data.len()
            );
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
            self.call_off(to, Instant::now());
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
                    // Every member gets every member's commands (the ROM's message link,
                    // like transfer mode 1's parent frames): the parent and the other
                    // children, tagged with the sender's aid.
                    let aid = self.links[from].aid;
                    for c in self.children_of(p) {
                        if c != from && self.links[c].accepted {
                            self.push_link_msg(c, LINK_MSG_DATA, aid, msg.data.clone());
                        }
                    }
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
                        // A 4-player multi battle seats aids 0 and 2 on one
                        // side: the parent's teammate gets 2, the others 1, 3.
                        let multi = msg
                            .data
                            .get(1)
                            .is_some_and(|m| LINK_MODE_MULTI_BATTLE.contains(m));
                        let teammate = self
                            .trainer_at(from)
                            .is_some_and(|me| self.team_partner_of(me) == self.trainer_at(t));
                        let order: [u8; 3] = match (multi, teammate) {
                            (true, true) => [2, 1, 3],
                            (true, false) => [1, 3, 2],
                            _ => [1, 2, 3],
                        };
                        order
                            .into_iter()
                            .filter(|&a| a <= LINK_MAX_AID)
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
        let by = self.links[slot].called_off_by;
        for c in self.children_of(slot) {
            self.links[c].parent = None;
            self.links[c].accepted = false;
            // A parent whose battle was called off ends its session: say why to the rest.
            self.links[c].called_off_by = self.links[c].called_off_by.or(by);
            self.push_link_msg(c, LINK_MSG_BYE, 0, Vec::new());
        }
        if let Some(p) = self.links[slot].parent.take() {
            let aid = self.links[slot].aid;
            self.links[slot].accepted = false;
            self.push_link_msg(p, LINK_MSG_BYE, aid, Vec::new());
        }
    }

    /// Link members that stopped sending link trailers: a client that still
    /// talks closed its block (it leaves now); a quiet client is waited for
    /// until `LINK_DEADLINE`, then its battle is called off.
    pub(crate) fn expire_links(&mut self, now: Instant) {
        for slot in 0..MAX_PLAYERS {
            let peer = &self.links[slot];
            let Some(quiet) = peer
                .last_trailer
                .filter(|_| peer.active)
                .map(|t| now.duration_since(t))
            else {
                continue;
            };
            let client_talks = self.slots[slot]
                .as_ref()
                .is_some_and(|p| now.duration_since(p.last_seen) <= LINK_SILENCE);
            if client_talks && quiet > LINK_SILENCE {
                self.leave_link(slot);
                self.links[slot] = LinkPeer::default();
            } else if quiet > LINK_DEADLINE {
                self.call_off(slot, now);
            }
        }
        for off in &mut self.link_called_off {
            if off
                .as_ref()
                .is_some_and(|o| now.duration_since(o.at) > LINK_CALLED_OFF_KEEP)
            {
                *off = None;
            }
        }
    }

    /// The referee: `slot` is gone (quiet past the deadline, or out of the
    /// room), so its battle can't go on (the user's rule: no one carries on
    /// alone). The others get BYE now, told who; `slot` gets its BYEs if it
    /// comes back with the same link block.
    fn call_off(&mut self, slot: usize, now: Instant) {
        let room_str = room_code_to_string(&self.room_code);
        // Its group: the parent once accepted, or the accepted children (a CONNECT
        // still pending is no battle yet; leave_link tells those as before).
        let peer = &self.links[slot];
        let others: Vec<usize> = match peer.parent {
            Some(p) if peer.accepted => vec![p],
            Some(_) => Vec::new(),
            None => self
                .children_of(slot)
                .into_iter()
                .filter(|&c| self.links[c].accepted)
                .collect(),
        };
        let byes: Vec<u8> = match peer.parent {
            Some(_) if !others.is_empty() => vec![0],
            Some(_) => Vec::new(),
            None => others.iter().map(|&c| self.links[c].aid).collect(),
        };
        if !others.is_empty() {
            warn!("[Room {room_str} | LINK] P{slot} is gone: its link battle is called off");
        }
        for &o in &others {
            self.links[o].called_off_by = Some(slot as u8);
        }
        self.link_called_off[slot] = (!byes.is_empty()).then(|| CalledOff {
            epoch: self.links[slot].epoch,
            byes,
            by: slot as u8,
            at: now,
        });
        self.leave_link(slot);
        self.links[slot] = LinkPeer::default();
    }

    /// Players who went offline: the referee calls their battles off.
    pub(super) fn drop_links_of_missing_players(&mut self) {
        let now = Instant::now();
        for slot in 0..MAX_PLAYERS {
            if self.slots[slot].is_none() && self.links[slot].active {
                self.call_off(slot, now);
            }
        }
    }

    /// A player in a link session, or one whose called-off BYEs wait: the room
    /// keeps its slot through a long pause (`prune_stale_clients`).
    pub(super) fn link_holds_slot(&self, slot: usize) -> bool {
        self.links[slot].active || self.link_called_off[slot].is_some()
    }

    /// The members of `slot`'s link group the battle waits for (quiet for
    /// `LINK_STALL`), as a player bitmap.
    fn link_waiting_for(&self, slot: usize, now: Instant) -> u8 {
        let peer = &self.links[slot];
        let group: Vec<usize> = match peer.parent.filter(|_| peer.accepted) {
            Some(p) => std::iter::once(p)
                .chain(self.children_of(p).into_iter().filter(|&c| c != slot))
                .collect(),
            None => self.children_of(slot),
        };
        group
            .into_iter()
            .filter(|&m| m == peer.parent.unwrap_or(usize::MAX) || self.links[m].accepted)
            .filter(|&m| {
                self.links[m]
                    .last_trailer
                    .is_some_and(|t| now.duration_since(t) > LINK_STALL)
            })
            .fold(0u8, |bits, m| bits | 1 << m)
    }

    /// The link trailer for a recipient: its ack, the room's beacons, and the
    /// messages due (new, or unacked for LINK_RESEND), oldest first.
    /// A link message for `slot` that was never sent.
    pub(crate) fn link_has_unsent(&self, slot: usize) -> bool {
        let peer = &self.links[slot];
        peer.active && peer.out.iter().any(|o| o.sent.is_none())
    }

    pub(crate) fn link_down_for(&mut self, slot: usize, now: Instant) -> Option<LinkDown> {
        if !self.links[slot].active {
            return None;
        }
        let waiting = self.link_waiting_for(slot, now);
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
            debug!(
                "link P{slot} down seq {first_seq}+{} (queued {})",
                msgs.len(),
                peer.out.len()
            );
        }
        Some(LinkDown {
            ack: peer.in_next,
            first_seq,
            epoch: Some(peer.epoch),
            waiting,
            called_off_by: peer.called_off_by,
            beacons,
            msgs,
        })
    }

    /// Tests: as if `slot`'s whole client went quiet `by` ago (a paused game).
    #[cfg(test)]
    pub(crate) fn backdate_client(&mut self, slot: usize, by: Duration) {
        self.backdate_link(slot, by);
        if let Some(p) = &mut self.slots[slot]
            && let Some(t) = p.last_seen.checked_sub(by)
        {
            p.last_seen = t;
        }
    }

    /// Tests: whether the room still holds `slot`, and its client timeout.
    #[cfg(test)]
    pub(crate) fn is_slot_occupied(&self, slot: usize) -> bool {
        self.slots[slot].is_some()
    }

    #[cfg(test)]
    pub(crate) fn set_client_timeout(&mut self, t: Duration) {
        self.client_timeout = t;
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
