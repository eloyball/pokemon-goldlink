//! The virtual link carrier (the C# bridge's CoopLink.cs, docs/MILESTONE3_PLAN.md §15). While
//! the ROM runs a local-wireless session it publishes a link block (`CoopLinkBlock`, "CLK1")
//! through the mailbox's exchange pointer. Each frame this drains the block's out ring into a
//! reliable stream to the relay, and writes the relay's routed messages into its in ring and the
//! world's beacons into its "seen" slots.
//!
//! Streams (both directions, as coop-relay's lobby/link.rs): consecutive u16 sequence numbers, a
//! cumulative ack (the next one expected), unacked messages resent after [`RESEND`], out-of-order
//! ones dropped. The epoch changes with every new link block, which resets both streams; the
//! relay names the block its down stream belongs to (RFC_DELTA §2), and another block's acks and
//! messages are skipped.

use std::collections::VecDeque;
use std::mem::{offset_of as of, size_of};
use std::time::{Duration, Instant};

use goldlink_abi::{CoopLinkBeacon, CoopLinkBlock, CoopLinkMsg};
use goldlink_proto as wire;

use crate::layout::{MAIN_RAM_BASE, MAIN_RAM_SIZE, u16_at, u32_at};

const BLOCK_MAGIC: u32 = goldlink_abi::COOP_LINK_MAGIC;
const BLOCK_VERSION: u16 = goldlink_abi::COOP_LINK_VERSION as u16;
const BLOCK_SIZE: usize = size_of::<CoopLinkBlock>();
const OFF_ROLE: usize = of!(CoopLinkBlock, role);
const OFF_OUT_HEAD: usize = of!(CoopLinkBlock, out_head);
const OFF_OUT_TAIL: usize = of!(CoopLinkBlock, out_tail);
const OFF_IN_HEAD: usize = of!(CoopLinkBlock, in_head);
const OFF_IN_TAIL: usize = of!(CoopLinkBlock, in_tail);
const OFF_MINE: usize = of!(CoopLinkBlock, mine);
const OFF_SEEN: usize = of!(CoopLinkBlock, seen);
const OFF_OUT: usize = of!(CoopLinkBlock, out);
const OFF_IN: usize = of!(CoopLinkBlock, in_);
const BEACON_SIZE: usize = size_of::<CoopLinkBeacon>();
const BEACON_MAX: usize = goldlink_abi::COOP_LINK_BEACON_MAX as usize;
const MSG_SIZE: usize = size_of::<CoopLinkMsg>();
const DATA_MAX: usize = goldlink_abi::COOP_LINK_DATA_MAX as usize;
const OUT_SLOTS: u8 = goldlink_abi::COOP_LINK_OUT_SLOTS as u8;
const IN_SLOTS: u8 = goldlink_abi::COOP_LINK_IN_SLOTS as u8;
const SEEN_SLOTS: usize = goldlink_abi::COOP_LINK_SEEN_SLOTS as usize;
const MSG_KIND_MAX: u8 = wire::LINK_MSG_BYE;

pub const RESEND: Duration = Duration::from_millis(150);
/// Relay -> client trailer header: magic, counts, ack, first seq, epoch and 1, then (protocol 9,
/// the referee) the players the battle waits for and who called it off.
const DOWN_HEADER: usize = 14;

struct Pending {
    seq: u16,
    kind: u8,
    aid: u8,
    data: Vec<u8>,
    sent: Option<Instant>,
}

/// One routed message for the in ring: kind, aid, data.
type InMsg = (u8, u8, Vec<u8>);

#[derive(Default)]
pub struct LinkCarrier {
    active: bool,
    block: usize,
    epoch: u8,
    role: u8,
    /// Encoded: ggid u32, len, entry, seq, pad, data.
    beacon: Option<Vec<u8>>,
    out_next: u16,
    out: Vec<Pending>,
    in_next: u16,
    inbox: VecDeque<InMsg>,
    /// Raw trailers from the relay, waiting for the next pump.
    received: VecDeque<Vec<u8>>,
    /// The relay's referee (dev/docs/LINK_BATTLE_REWORK.md part 3): the players of my link group
    /// the battle waits for (a bitmap), and the player whose loss called it off.
    waiting: u8,
    called_off_by: Option<u8>,
}

/// Main-RAM offset of a valid link block at ARM9 address `ptr`, or None.
pub fn find(ram: &[u8], ptr: u32) -> Option<usize> {
    if ptr < MAIN_RAM_BASE
        || ptr >= MAIN_RAM_BASE + MAIN_RAM_SIZE - BLOCK_SIZE as u32
        || ptr & 3 != 0
    {
        return None;
    }
    let at = (ptr - MAIN_RAM_BASE) as usize;
    (u32_at(ram, at) == BLOCK_MAGIC
        && u16_at(ram, at + 4) == BLOCK_VERSION
        && u16_at(ram, at + 6) as usize == BLOCK_SIZE)
        .then_some(at)
}

/// A link trailer ("LNK1") starts at `at`.
pub fn has_magic(buf: &[u8], at: usize) -> bool {
    buf.len() >= at + 4 && u32_at(buf, at) == wire::LINK_MAGIC
}

fn seq_before(a: u16, b: u16) -> bool {
    (b.wrapping_sub(a) as i16) > 0
}

impl LinkCarrier {
    pub fn new() -> Self {
        Self::default()
    }

    /// Keeps a server packet's link trailer for the next pump (every OK packet counts: the
    /// latest alone could skip one). It follows the 360-byte packet, after the trade trailer if
    /// there is one.
    pub fn on_server_packet(&mut self, pkt: &[u8]) {
        let mut at = wire::SERVER_PACKET_SIZE;
        if !has_magic(pkt, at) {
            at += wire::TRADE_IN_SIZE;
            if !has_magic(pkt, at) {
                return;
            }
        }
        if self.received.len() < 64 {
            self.received.push_back(pkt[at..].to_vec());
        }
    }

    fn reset(&mut self) {
        self.active = false;
        self.out.clear();
        self.inbox.clear();
        self.out_next = 0;
        self.in_next = 0;
        self.beacon = None;
        self.role = 0;
        self.waiting = 0;
        self.called_off_by = None;
    }

    /// The relay's referee for my link session: who the battle waits for (a player bitmap) and
    /// who its being called off is down to. Nothing outside a session.
    pub fn referee(&self) -> (u8, Option<u8>) {
        if self.active {
            (self.waiting, self.called_off_by)
        } else {
            (0, None)
        }
    }

    /// After a frame. `block`: the link block's main-RAM offset (None: no link session).
    pub fn pump(&mut self, ram: &mut [u8], block: Option<usize>, my_player: u8) {
        let Some(blk) = block else {
            if self.active {
                self.reset();
            }
            self.received.clear();
            return;
        };
        if !self.active || blk != self.block {
            self.reset();
            self.active = true;
            self.block = blk;
            self.epoch = self.epoch.wrapping_add(1);
        }

        // My messages and beacon.
        self.role = ram[blk + OFF_ROLE];
        let head = ram[blk + OFF_OUT_HEAD];
        let mut tail = ram[blk + OFF_OUT_TAIL];
        while tail != head && self.out.len() < 256 {
            let m = blk + OFF_OUT + (tail % OUT_SLOTS) as usize * MSG_SIZE;
            let len = (u16_at(ram, m + 2) as usize).min(DATA_MAX);
            self.out.push(Pending {
                seq: self.out_next,
                kind: ram[m],
                aid: ram[m + 1],
                data: ram[m + 4..m + 4 + len].to_vec(),
                sent: None,
            });
            self.out_next = self.out_next.wrapping_add(1);
            tail = tail.wrapping_add(1);
        }
        ram[blk + OFF_OUT_TAIL] = tail;
        let mine = &ram[blk + OFF_MINE..blk + OFF_MINE + 8];
        let beacon_len = mine[4] as usize;
        self.beacon = (beacon_len > 0 && beacon_len <= BEACON_MAX).then(|| {
            let mut b = Vec::with_capacity(8 + beacon_len);
            b.extend_from_slice(&mine[0..4]);
            b.push(beacon_len as u8);
            b.push(mine[5]);
            b.push(mine[7]);
            b.push(0);
            b.extend_from_slice(&ram[blk + OFF_MINE + 8..blk + OFF_MINE + 8 + beacon_len]);
            b
        });

        self.deliver_in(ram, blk, my_player);
    }

    /// Before a frame (link latency, docs/GAME_REBALANCE_UPDATE.md §25): what the relay sent
    /// since the last frame goes into the in ring now, so the game handles it this frame. Only
    /// the receiving half, on the block the last pump validated (the game hasn't run since).
    pub fn pump_in(&mut self, ram: &mut [u8], my_player: u8) {
        if self.active && !self.received.is_empty() {
            let blk = self.block;
            self.deliver_in(ram, blk, my_player);
        }
    }

    fn deliver_in(&mut self, ram: &mut [u8], blk: usize, my_player: u8) {
        let mut last_beacons = None;
        while let Some(t) = self.received.pop_front() {
            if let Some(b) = self.apply(&t) {
                last_beacons = Some(b);
            }
        }
        if let Some(b) = last_beacons {
            write_seen(ram, blk, &b, my_player);
        }
        // Into the in ring, as far as it has room.
        let mut head = ram[blk + OFF_IN_HEAD];
        let tail = ram[blk + OFF_IN_TAIL];
        while head.wrapping_sub(tail) < IN_SLOTS {
            let Some((kind, aid, data)) = self.inbox.pop_front() else {
                break;
            };
            let m = blk + OFF_IN + (head % IN_SLOTS) as usize * MSG_SIZE;
            ram[m] = kind;
            ram[m + 1] = aid;
            ram[m + 2..m + 4].copy_from_slice(&(data.len() as u16).to_le_bytes());
            ram[m + 4..m + 4 + data.len()].copy_from_slice(&data);
            head = head.wrapping_add(1);
        }
        ram[blk + OFF_IN_HEAD] = head; // last: the ROM reads up to in_head
    }

    /// Applies one relay trailer; returns its beacon section (count, then the raw beacons), or
    /// None when malformed.
    fn apply(&mut self, t: &[u8]) -> Option<Vec<u8>> {
        if t.len() < DOWN_HEADER || u32_at(t, 0) != wire::LINK_MAGIC {
            return None;
        }
        let (beacon_count, count) = (t[4] as usize, t[5] as usize);
        let (ack, first_seq) = (u16_at(t, 6), u16_at(t, 8));
        if beacon_count > 6 || count > wire::LINK_MAX_MSGS {
            return None;
        }
        let mut pos = DOWN_HEADER;
        let beacons_start = pos;
        for _ in 0..beacon_count {
            if pos + 8 > t.len() || t[pos + 1] as usize > BEACON_MAX {
                return None;
            }
            pos += 8 + t[pos + 1] as usize;
        }
        if pos > t.len() {
            return None;
        }
        let mut beacons = Vec::with_capacity(1 + pos - beacons_start);
        beacons.push(beacon_count as u8);
        beacons.extend_from_slice(&t[beacons_start..pos]);

        // Another block's ack and messages are skipped (its beacons are the world's, still
        // fine); so is an ack past anything ever sent.
        let other_block = t[11] == 1 && t[10] != self.epoch;
        if other_block || seq_before(self.out_next, ack) {
            return Some(beacons);
        }
        self.waiting = t[12];
        self.called_off_by = (t[13] < wire::MAX_PLAYERS as u8).then_some(t[13]);
        self.out.retain(|p| !seq_before(p.seq, ack));
        for i in 0..count {
            if pos + 4 > t.len() {
                return None;
            }
            let (kind, aid) = (t[pos], t[pos + 1]);
            let len = u16_at(t, pos + 2) as usize;
            if len > DATA_MAX || pos + 4 + len > t.len() {
                return None;
            }
            let seq = first_seq.wrapping_add(i as u16);
            if seq == self.in_next
                && (1..=MSG_KIND_MAX).contains(&kind)
                && aid <= 15
                && self.inbox.len() < 256
            {
                self.inbox
                    .push_back((kind, aid, t[pos + 4..pos + 4 + len].to_vec()));
                self.in_next = self.in_next.wrapping_add(1);
            }
            pos += 4 + len;
        }
        Some(beacons)
    }

    /// The link trailer for the next client packet, or None outside a link session.
    pub fn build_trailer(&mut self, now: Instant) -> Option<Vec<u8>> {
        if !self.active {
            return None;
        }
        let start = self
            .out
            .iter()
            .position(|p| p.sent.is_none_or(|s| now.duration_since(s) >= RESEND));
        let mut msgs = Vec::new();
        if let Some(start) = start {
            let mut budget = wire::LINK_SEND_BUDGET;
            for p in self.out[start..].iter_mut() {
                if msgs.len() >= wire::LINK_MAX_MSGS {
                    break;
                }
                let size = 4 + p.data.len();
                if size > budget {
                    break;
                }
                budget -= size;
                p.sent = Some(now);
                msgs.push((p.seq, p.kind, p.aid, p.data.clone()));
            }
        }
        let mut buf = Vec::with_capacity(64);
        buf.extend_from_slice(&wire::LINK_MAGIC.to_le_bytes());
        buf.push(self.epoch);
        buf.push(self.role);
        buf.push(u8::from(self.beacon.is_some()));
        buf.push(msgs.len() as u8);
        buf.extend_from_slice(&self.in_next.to_le_bytes());
        let first = msgs.first().map_or(self.out_next, |m| m.0);
        buf.extend_from_slice(&first.to_le_bytes());
        if let Some(b) = &self.beacon {
            buf.extend_from_slice(b);
        }
        for (_, kind, aid, data) in msgs {
            buf.push(kind);
            buf.push(aid);
            buf.extend_from_slice(&(data.len() as u16).to_le_bytes());
            buf.extend_from_slice(&data);
        }
        Some(buf)
    }
}

/// The world's parents into the block's seen slots, by remote slot; the rest emptied.
fn write_seen(ram: &mut [u8], blk: usize, beacons: &[u8], my_player: u8) {
    let mut seen: [Option<Vec<u8>>; SEEN_SLOTS] = Default::default();
    let mut pos = 1;
    for _ in 0..beacons[0] {
        let (player, len) = (beacons[pos], beacons[pos + 1] as usize);
        if player < 6 && player != my_player && my_player < 6 {
            let remote = if player < my_player {
                player
            } else {
                player - 1
            } as usize;
            let mut b = Vec::with_capacity(8 + len);
            b.extend_from_slice(&beacons[pos + 4..pos + 8]); // ggid
            b.push(len as u8);
            b.push(beacons[pos + 2]); // entry
            b.push(player);
            b.push(beacons[pos + 3]); // seq
            b.extend_from_slice(&beacons[pos + 8..pos + 8 + len]);
            seen[remote] = Some(b);
        }
        pos += 8 + len;
    }
    for (k, s) in seen.iter().enumerate() {
        let at = blk + OFF_SEEN + k * BEACON_SIZE;
        match s {
            Some(b) => ram[at..at + b.len()].copy_from_slice(b),
            None => ram[at + 4] = 0, // no beacon
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The C# carrier's constants.
    #[test]
    fn layout_matches_the_csharp_carrier() {
        assert_eq!(
            (BLOCK_MAGIC, BLOCK_VERSION, BLOCK_SIZE),
            (0x314B4C43, 1, 0xDB0)
        );
        assert_eq!(
            (
                OFF_ROLE,
                OFF_OUT_HEAD,
                OFF_OUT_TAIL,
                OFF_IN_HEAD,
                OFF_IN_TAIL
            ),
            (0x08, 0x0C, 0x0D, 0x0E, 0x0F)
        );
        assert_eq!(
            (OFF_MINE, OFF_SEEN, OFF_OUT, OFF_IN),
            (0x010, 0x088, 0x2E0, 0x5F0)
        );
        assert_eq!(
            (BEACON_SIZE, BEACON_MAX, MSG_SIZE, DATA_MAX),
            (0x78, 0x70, 0xC4, 0xC0)
        );
        assert_eq!(
            (OUT_SLOTS, IN_SLOTS, SEEN_SLOTS, MSG_KIND_MAX),
            (4, 10, 5, 5)
        );
    }

    fn ram_with_block() -> (Vec<u8>, usize) {
        let mut ram = vec![0u8; 0x40_0000];
        let blk = 0x10_0000;
        ram[blk..blk + 4].copy_from_slice(&BLOCK_MAGIC.to_le_bytes());
        ram[blk + 4..blk + 6].copy_from_slice(&BLOCK_VERSION.to_le_bytes());
        ram[blk + 6..blk + 8].copy_from_slice(&(BLOCK_SIZE as u16).to_le_bytes());
        (ram, blk)
    }

    fn put_out(ram: &mut [u8], blk: usize, slot: u8, kind: u8, data: &[u8]) {
        let m = blk + OFF_OUT + (slot % OUT_SLOTS) as usize * MSG_SIZE;
        ram[m] = kind;
        ram[m + 1] = 0;
        ram[m + 2..m + 4].copy_from_slice(&(data.len() as u16).to_le_bytes());
        ram[m + 4..m + 4 + data.len()].copy_from_slice(data);
    }

    /// A relay trailer with these messages (from `first`), acking `ack`, for block `epoch`.
    fn down(epoch: u8, ack: u16, first: u16, msgs: &[&[u8]]) -> Vec<u8> {
        let mut t = Vec::new();
        t.extend_from_slice(&wire::LINK_MAGIC.to_le_bytes());
        t.push(0);
        t.push(msgs.len() as u8);
        t.extend_from_slice(&ack.to_le_bytes());
        t.extend_from_slice(&first.to_le_bytes());
        t.push(epoch);
        t.push(1);
        t.push(0); // nobody waited for
        t.push(wire::LINK_NOBODY);
        for m in msgs {
            t.push(wire::LINK_MSG_DATA);
            t.push(0);
            t.extend_from_slice(&(m.len() as u16).to_le_bytes());
            t.extend_from_slice(m);
        }
        t
    }

    #[test]
    fn out_ring_to_trailer_with_resend_and_ack() {
        let (mut ram, blk) = ram_with_block();
        assert_eq!(find(&ram, MAIN_RAM_BASE + blk as u32), Some(blk));
        let mut c = LinkCarrier::new();
        put_out(&mut ram, blk, 0, wire::LINK_MSG_DATA, b"hello");
        put_out(&mut ram, blk, 1, wire::LINK_MSG_DATA, b"world");
        ram[blk + OFF_OUT_HEAD] = 2;
        c.pump(&mut ram, Some(blk), 0);
        assert_eq!(ram[blk + OFF_OUT_TAIL], 2, "the ring is drained");
        let t0 = Instant::now();
        let t = c.build_trailer(t0).unwrap();
        assert_eq!(
            (t[4], t[7], u16_at(&t, 10)),
            (1, 2, 0),
            "epoch 1, two messages from seq 0"
        );
        assert_eq!(&t[16..21], b"hello");
        // Not resent before 150 ms; resent after.
        assert_eq!(
            c.build_trailer(t0 + Duration::from_millis(10)).unwrap()[7],
            0
        );
        assert_eq!(c.build_trailer(t0 + RESEND).unwrap()[7], 2);
        // The relay acks the first: only the second is resent.
        c.on_server_packet(&[vec![0u8; wire::SERVER_PACKET_SIZE], down(1, 1, 0, &[])].concat());
        c.pump(&mut ram, Some(blk), 0);
        let t = c.build_trailer(t0 + RESEND * 3).unwrap();
        assert_eq!((t[7], u16_at(&t, 10)), (1, 1));
    }

    #[test]
    fn in_messages_in_order_into_the_ring() {
        let (mut ram, blk) = ram_with_block();
        let mut c = LinkCarrier::new();
        c.pump(&mut ram, Some(blk), 1);
        let pkt = |t: Vec<u8>| [vec![0u8; wire::SERVER_PACKET_SIZE], t].concat();
        // Seq 1 before seq 0: dropped (go-back-N); then 0 and 1 arrive.
        c.on_server_packet(&pkt(down(1, 0, 1, &[b"b"])));
        c.on_server_packet(&pkt(down(1, 0, 0, &[b"a", b"b"])));
        c.pump_in(&mut ram, 1);
        assert_eq!(ram[blk + OFF_IN_HEAD], 2);
        assert_eq!(ram[blk + OFF_IN + 4], b'a');
        assert_eq!(ram[blk + OFF_IN + MSG_SIZE + 4], b'b');
        assert_eq!(
            u16_at(&c.build_trailer(Instant::now()).unwrap(), 8),
            2,
            "acks 2"
        );
        // Another block's trailer (epoch 9): skipped.
        c.on_server_packet(&pkt(down(9, 0, 2, &[b"x"])));
        c.pump_in(&mut ram, 1);
        assert_eq!(ram[blk + OFF_IN_HEAD], 2);
        // The game reads one; a full ring waits.
        ram[blk + OFF_IN_TAIL] = 2;
        let many: Vec<Vec<u8>> = (0..12).map(|i| vec![i]).collect();
        let refs: Vec<&[u8]> = many.iter().map(Vec::as_slice).collect();
        c.on_server_packet(&pkt(down(1, 0, 2, &refs)));
        c.pump_in(&mut ram, 1);
        assert_eq!(
            ram[blk + OFF_IN_HEAD],
            12,
            "ten slots: 10 delivered, 2 wait"
        );
        ram[blk + OFF_IN_TAIL] = 12;
        c.pump_in(&mut ram, 1);
        c.pump(&mut ram, Some(blk), 1);
        assert_eq!(ram[blk + OFF_IN_HEAD], 14);
    }

    #[test]
    fn a_new_block_resets_the_streams() {
        let (mut ram, blk) = ram_with_block();
        let mut c = LinkCarrier::new();
        c.pump(&mut ram, Some(blk), 0);
        put_out(&mut ram, blk, 0, wire::LINK_MSG_DATA, b"x");
        ram[blk + OFF_OUT_HEAD] = 1;
        c.pump(&mut ram, Some(blk), 0);
        c.pump(&mut ram, None, 0);
        assert!(c.build_trailer(Instant::now()).is_none());
        c.pump(&mut ram, Some(blk), 0);
        let t = c.build_trailer(Instant::now()).unwrap();
        assert_eq!(
            (t[4], t[7], u16_at(&t, 10)),
            (2, 0, 0),
            "epoch 2, nothing pending"
        );
    }

    /// Protocol 9: the relay's referee bytes (header bytes 12 and 13).
    #[test]
    fn the_referee_bytes() {
        let (mut ram, blk) = ram_with_block();
        let mut c = LinkCarrier::new();
        c.pump(&mut ram, Some(blk), 0);
        assert_eq!(c.referee(), (0, None));
        let mut t = down(c.epoch, 0, 0, &[]);
        t[12] = 0b10;
        t[13] = 1;
        c.on_server_packet(&[vec![0; wire::SERVER_PACKET_SIZE], t].concat());
        c.pump(&mut ram, Some(blk), 0);
        assert_eq!(c.referee(), (0b10, Some(1)));
        c.pump(&mut ram, None, 0);
        assert_eq!(c.referee(), (0, None), "cleared with the link block");
    }
}
