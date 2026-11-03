//! Trading between two players (docs/MILESTONE3_PLAN.md, Phase 2).
//!
//! The relay is the escrow: it holds both offers (encrypted party Pokemon,
//! as the games store them), and commits only when each side confirmed the
//! other's *current* offer. Before saying "commit" it writes the trade (both
//! Pokemon) to the campaign file; each game then swaps, saves and reports
//! DONE. A game that crashes or disconnects before saving gets the same
//! commit again when it comes back, until it reports DONE. The ROM applies a
//! commit idempotently (it looks the Pokemon up by personality value).
//!
//! Milestone 5 Ghost Mode adds the one-sided **revive** (docs/MILESTONE5_PLAN.md
//! §5.5.4): side 0 (the donor) offers, side 1 (the spirit) only confirms that
//! offer, and the commit gives the donor's Pokemon to the spirit with nothing
//! coming back. The same journal makes it crash-safe.

use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use tracing::{info, warn};

use super::{LobbyState, remote_slot_of};
use crate::protocol::{
    MAX_PLAYERS, NO_SLOT, TRADE_ABORT_CANCELLED, TRADE_ABORT_LEFT, TRADE_ABORT_TIMEOUT,
    TRADE_FLAG_RECEIVER, TRADE_FLAG_REVIVE, TRADE_IN_ABORTED, TRADE_IN_COMMIT, TRADE_IN_OFFERED,
    TRADE_IN_WAITING, TRADE_MON_SIZE, TRADE_OUT_CANCEL, TRADE_OUT_CONFIRMED, TRADE_OUT_DONE,
    TRADE_OUT_OFFERED, TradeIn, TradeOut, mon_pid, room_code_to_string,
};

/// An open trade nobody finishes is cancelled after this long.
pub const TRADE_IDLE_TIMEOUT: Duration = Duration::from_secs(10 * 60);
/// A side whose game stops sending its trade (crash, closed menu, lost
/// packets) cancels the trade after this long.
pub const TRADE_SILENCE_TIMEOUT: Duration = Duration::from_secs(15);
/// How long an abort notice waits for its acknowledgment.
pub const TRADE_NOTICE_TTL: Duration = Duration::from_secs(15);

#[derive(Debug, Clone, Copy)]
pub(super) struct Offer {
    id: u8,
    mon: [u8; TRADE_MON_SIZE],
}

/// A trade between two online players, before the commit.
#[derive(Debug, Clone)]
pub(super) struct ActiveTrade {
    slots: [usize; 2],
    trainers: [u32; 2],
    offers: [Option<Offer>; 2],
    /// The partner offer id each side confirmed.
    confirmed: [Option<u8>; 2],
    last_heard: [Instant; 2],
    last_change: Instant,
    /// A revive: side 0 gives, side 1 (the spirit) receives.
    revive: bool,
}

impl ActiveTrade {
    fn side_of(&self, slot: usize) -> Option<usize> {
        self.slots.iter().position(|&s| s == slot)
    }

    /// Both confirmed the other's current offer (a revive: the spirit
    /// confirmed the donor's).
    fn agreed(&self) -> bool {
        if self.revive {
            return matches!((&self.offers[0], self.confirmed[1]), (Some(a), Some(c)) if a.id == c);
        }
        match (&self.offers, &self.confirmed) {
            ([Some(a), Some(b)], [Some(conf_a), Some(conf_b)]) => {
                *conf_a == b.id && *conf_b == a.id
            }
            _ => false,
        }
    }
}

/// A committed trade, persisted until both games saved it. `mons[i]` is what
/// `trainers[i]` gives (hex of the encrypted party Pokemon).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PendingTrade {
    pub id: u32,
    pub trainers: [u32; 2],
    pub mons: [String; 2],
    pub done: [bool; 2],
    /// Milestone 5: a revive; `mons[1]` is empty (zeros), the spirit gives nothing.
    #[serde(default)]
    pub revive: bool,
}

impl PendingTrade {
    fn side_of(&self, trainer: u32) -> Option<usize> {
        self.trainers.iter().position(|&t| t == trainer)
    }
}

/// "Your trade was cancelled", until the game acknowledges it.
#[derive(Debug, Clone, Copy)]
pub(super) struct TradeNotice {
    trainer: u32,
    reason: u8,
    created: Instant,
}

fn revive_flags(revive: bool, side: usize) -> u8 {
    match (revive, side) {
        (false, _) => 0,
        (true, 0) => TRADE_FLAG_REVIVE,
        (true, _) => TRADE_FLAG_REVIVE | TRADE_FLAG_RECEIVER,
    }
}

fn to_hex(mon: &[u8; TRADE_MON_SIZE]) -> String {
    mon.iter().map(|b| format!("{b:02x}")).collect()
}

fn from_hex(text: &str) -> Option<[u8; TRADE_MON_SIZE]> {
    if text.len() != TRADE_MON_SIZE * 2 {
        return None;
    }
    let mut mon = [0u8; TRADE_MON_SIZE];
    for (i, byte) in mon.iter_mut().enumerate() {
        *byte = u8::from_str_radix(text.get(i * 2..i * 2 + 2)?, 16).ok()?;
    }
    Some(mon)
}

impl LobbyState {
    pub fn pending_trades(&self) -> &[PendingTrade] {
        &self.pending_trades
    }

    pub fn active_trade_count(&self) -> usize {
        self.trades.len()
    }

    /// Busy for new requests: an open trade, a commit to apply, or an abort
    /// notice to read.
    pub(super) fn trade_busy(&self, slot: usize) -> bool {
        let trainer = self.trainer_at(slot).unwrap_or(0);
        self.trades.iter().any(|t| t.side_of(slot).is_some())
            || (trainer != 0
                && (self.pending_side(trainer).is_some()
                    || self.trade_notices.iter().any(|n| n.trainer == trainer)))
    }

    /// The pending commit this trainer has not saved yet: (index, side).
    fn pending_side(&self, trainer: u32) -> Option<(usize, usize)> {
        self.pending_trades.iter().enumerate().find_map(|(i, p)| {
            p.side_of(trainer)
                .filter(|&side| !p.done[side])
                .map(|side| (i, side))
        })
    }

    /// Opens a trade between `a` and `b`; a revive has `a` give to the spirit `b`.
    pub(super) fn start_trade(&mut self, a: usize, b: usize, revive: bool, now: Instant) {
        let (Some(ta), Some(tb)) = (self.trainer_at(a), self.trainer_at(b)) else {
            return;
        };
        info!(
            "[Room {} | TRADE] P{a} and P{b} opened a {}",
            room_code_to_string(&self.room_code),
            if revive { "revive" } else { "trade" }
        );
        self.trades.push(ActiveTrade {
            slots: [a, b],
            trainers: [ta, tb],
            offers: [None, None],
            confirmed: [None, None],
            last_heard: [now, now],
            last_change: now,
            revive,
        });
    }

    fn notify_abort(&mut self, trainer: u32, reason: u8, now: Instant) {
        self.trade_notices.retain(|n| n.trainer != trainer);
        self.trade_notices.push(TradeNotice {
            trainer,
            reason,
            created: now,
        });
    }

    /// Ends open trade `index`: each side reads `reasons[side]` (None: no notice).
    fn abort_trade(&mut self, index: usize, reasons: [Option<u8>; 2], now: Instant) {
        let trade = self.trades.remove(index);
        info!(
            "[Room {} | TRADE] trade between P{} and P{} cancelled",
            room_code_to_string(&self.room_code),
            trade.slots[0],
            trade.slots[1]
        );
        for (trainer, reason) in trade.trainers.into_iter().zip(reasons) {
            if let Some(reason) = reason {
                self.notify_abort(trainer, reason, now);
            }
        }
    }

    /// Applies the trade trailer of one client packet (or its absence).
    pub(super) fn apply_trade_out(&mut self, slot: usize, out: Option<TradeOut>, now: Instant) {
        let Some(out) = out else {
            return;
        };
        let Some(trainer) = self.trainer_at(slot) else {
            return;
        };

        if out.state == TRADE_OUT_DONE {
            self.finish_pending(trainer, out.trade_id);
            return;
        }
        if out.state == TRADE_OUT_CANCEL {
            // Acknowledges a notice, or withdraws from an open trade.
            self.trade_notices.retain(|n| n.trainer != trainer);
            if let Some(i) = self.trades.iter().position(|t| t.side_of(slot).is_some()) {
                let side = self.trades[i].side_of(slot).unwrap_or(0);
                let mut reasons = [None, None];
                reasons[1 - side] = Some(TRADE_ABORT_CANCELLED);
                self.abort_trade(i, reasons, now);
            }
            return;
        }

        let Some(i) = self.trades.iter().position(|t| t.side_of(slot).is_some()) else {
            return;
        };
        let trade = &mut self.trades[i];
        let side = trade.side_of(slot).unwrap_or(0);
        trade.last_heard[side] = now;

        // In a revive only the donor offers.
        if (out.state == TRADE_OUT_OFFERED || out.state == TRADE_OUT_CONFIRMED)
            && !(trade.revive && side == 1)
            && out.offer_id != 0
            && mon_pid(&out.mon) != 0
            && trade.offers[side].is_none_or(|o| o.id != out.offer_id)
        {
            // A new offer: every earlier confirmation is void.
            trade.offers[side] = Some(Offer {
                id: out.offer_id,
                mon: out.mon,
            });
            trade.confirmed = [None, None];
            trade.last_change = now;
        }
        if out.state == TRADE_OUT_CONFIRMED
            && trade.offers[1 - side].is_some_and(|o| o.id == out.confirm_id)
            && trade.confirmed[side] != Some(out.confirm_id)
        {
            trade.confirmed[side] = Some(out.confirm_id);
            trade.last_change = now;
        }
        if trade.agreed() {
            self.commit_trade(i, now);
        }
    }

    /// Journals the trade on disk, then tells both games to swap.
    fn commit_trade(&mut self, index: usize, now: Instant) {
        let trade = self.trades[index].clone();
        let room = room_code_to_string(&self.room_code);
        let nothing = Offer {
            id: 0,
            mon: [0; TRADE_MON_SIZE],
        };
        let (a, b) = match (trade.offers, trade.revive) {
            ([Some(a), _], true) => (a, nothing),
            ([Some(a), Some(b)], false) => (a, b),
            _ => return,
        };
        let id = loop {
            let id = super::new_room_uid(self.room_code) ^ (a.id as u32) << 8 ^ (b.id as u32);
            if id != 0 && !self.pending_trades.iter().any(|p| p.id == id) {
                break id;
            }
        };
        self.pending_trades.push(PendingTrade {
            id,
            trainers: trade.trainers,
            mons: [to_hex(&a.mon), to_hex(&b.mon)],
            done: [false, false],
            revive: trade.revive,
        });
        if !self.save_campaign_to_disk() {
            // Without the journal a crash could duplicate or lose a Pokemon:
            // refuse the trade instead.
            warn!("[Room {room} | TRADE] cannot journal trade 0x{id:08X}; cancelled");
            self.pending_trades.pop();
            self.abort_trade(
                index,
                [Some(TRADE_ABORT_CANCELLED), Some(TRADE_ABORT_CANCELLED)],
                now,
            );
            return;
        }
        self.trades.remove(index);
        info!(
            "[Room {room} | TRADE] committed 0x{id:08X}: 0x{:08X} gives PID 0x{:08X}, 0x{:08X} gives PID 0x{:08X}",
            trade.trainers[0],
            mon_pid(&a.mon),
            trade.trainers[1],
            mon_pid(&b.mon)
        );
    }

    fn finish_pending(&mut self, trainer: u32, trade_id: u32) {
        let Some(i) = self.pending_trades.iter().position(|p| p.id == trade_id) else {
            return;
        };
        let Some(side) = self.pending_trades[i].side_of(trainer) else {
            return;
        };
        if self.pending_trades[i].done[side] {
            return;
        }
        self.pending_trades[i].done[side] = true;
        info!(
            "[Room {} | TRADE] 0x{trainer:08X} saved trade 0x{trade_id:08X}",
            room_code_to_string(&self.room_code)
        );
        if self.pending_trades[i].done.iter().all(|&d| d) {
            self.pending_trades.remove(i);
        }
        self.save_campaign_to_disk();
    }

    /// Open trades lose a side that went offline; the other side is told.
    pub(super) fn abort_trades_of_missing_players(&mut self) {
        let now = Instant::now();
        while let Some(i) = self
            .trades
            .iter()
            .position(|t| t.slots.iter().any(|&s| self.slots[s].is_none()))
        {
            let gone = self.trades[i].slots.map(|s| self.slots[s].is_none());
            let reasons = gone.map(|g| (!g).then_some(TRADE_ABORT_LEFT));
            // The leaving side sees nothing (it is gone); the other one is told.
            self.abort_trade(i, [reasons[0], reasons[1]], now);
        }
    }

    pub fn expire_trades(&mut self, now: Instant) {
        while let Some(i) = self.trades.iter().position(|t| {
            now.duration_since(t.last_change) >= TRADE_IDLE_TIMEOUT
                || t.last_heard
                    .iter()
                    .any(|&h| now.duration_since(h) >= TRADE_SILENCE_TIMEOUT)
        }) {
            let trade = &self.trades[i];
            let silent = trade
                .last_heard
                .map(|h| now.duration_since(h) >= TRADE_SILENCE_TIMEOUT);
            // A silent game is gone as far as its partner can tell.
            let reasons = if now.duration_since(trade.last_change) >= TRADE_IDLE_TIMEOUT {
                [Some(TRADE_ABORT_TIMEOUT), Some(TRADE_ABORT_TIMEOUT)]
            } else {
                [
                    Some(if silent[1] {
                        TRADE_ABORT_LEFT
                    } else {
                        TRADE_ABORT_TIMEOUT
                    }),
                    Some(if silent[0] {
                        TRADE_ABORT_LEFT
                    } else {
                        TRADE_ABORT_TIMEOUT
                    }),
                ]
            };
            self.abort_trade(i, reasons, now);
        }
        self.trade_notices
            .retain(|n| now.duration_since(n.created) < TRADE_NOTICE_TTL);
    }

    /// The trailer for one recipient, or None (then trade_pending is 0).
    pub(super) fn trade_in_for(&self, slot: usize) -> Option<TradeIn> {
        let trainer = self.trainer_at(slot)?;
        let peer_slot = |other: u32| {
            (0..MAX_PLAYERS)
                .find(|&j| j != slot && self.trainer_at(j) == Some(other))
                .map_or(NO_SLOT, |j| remote_slot_of(slot, j))
        };

        if let Some((i, side)) = self.pending_side(trainer) {
            let pending = &self.pending_trades[i];
            let (Some(give), Some(receive)) = (
                from_hex(&pending.mons[side]),
                from_hex(&pending.mons[1 - side]),
            ) else {
                return None;
            };
            let mut t = TradeIn::empty(TRADE_IN_COMMIT);
            t.trade_id = pending.id;
            t.give_pid = mon_pid(&give);
            t.mon = receive;
            t.peer_slot = peer_slot(pending.trainers[1 - side]);
            t.flags = revive_flags(pending.revive, side);
            return Some(t);
        }
        if let Some(trade) = self.trades.iter().find(|t| t.side_of(slot).is_some()) {
            let side = trade.side_of(slot).unwrap_or(0);
            let partner = trade.offers[1 - side];
            let mine = trade.offers[side];
            let mut t = TradeIn::empty(if partner.is_some() {
                TRADE_IN_OFFERED
            } else {
                TRADE_IN_WAITING
            });
            if let Some(p) = partner {
                t.partner_offer_id = p.id;
                t.mon = p.mon;
            }
            t.my_offer_id = mine.map_or(0, |o| o.id);
            t.partner_confirmed =
                u8::from(mine.is_some() && trade.confirmed[1 - side] == mine.map(|o| o.id));
            t.peer_slot = remote_slot_of(slot, trade.slots[1 - side]);
            t.flags = revive_flags(trade.revive, side);
            return Some(t);
        }
        self.trade_notices
            .iter()
            .find(|n| n.trainer == trainer)
            .map(|n| {
                let mut t = TradeIn::empty(TRADE_IN_ABORTED);
                t.reason = n.reason;
                t
            })
    }
}
