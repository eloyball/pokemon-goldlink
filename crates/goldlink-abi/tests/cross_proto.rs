//! The relay's wire constants (goldlink-proto, hand-copied from the ROM over the milestones)
//! against the ROM's own (goldlink-abi, generated from its headers). When one side changes and
//! the other doesn't, this fails, instead of a "Must match ..." comment going stale.

use core::mem::size_of;

use goldlink_abi as rom;
use goldlink_proto as wire;

/// `rom == wire`, whatever integer types each side uses.
macro_rules! same {
    ($($rom:expr => $wire:expr),+ $(,)?) => {
        $(assert_eq!(
            u64::try_from($rom).unwrap(),
            u64::try_from($wire).unwrap(),
            "{} != {}",
            stringify!($rom),
            stringify!($wire)
        );)+
    };
}

#[test]
fn packet_payloads_are_the_mailbox_blocks() {
    same! {
        rom::COOP_MAGIC => wire::COOP_MAGIC,
        rom::COOP_MAX_PLAYERS => wire::MAX_PLAYERS,
        rom::COOP_REMOTE_SLOTS => wire::REMOTE_SLOTS,
        size_of::<rom::CoopPlayerState>() => wire::PLAYER_STATE_SIZE,
        size_of::<rom::CoopProgressionGates>() => wire::GATES_SIZE,
        size_of::<rom::CoopSessionOut>() => wire::SESSION_BLOCK_SIZE,
        size_of::<rom::CoopTradeOut>() => wire::TRADE_OUT_SIZE,
        size_of::<rom::CoopTradeIn>() => wire::TRADE_IN_SIZE,
        size_of::<[u8; 236]>() => wire::TRADE_MON_SIZE,
    }
    // The server packet's payload is copied verbatim to mailbox 0x050..0x1A8: the five remote
    // players, then the gates.
    same! {
        size_of::<[rom::CoopPlayerState; 5]>() + size_of::<rom::CoopProgressionGates>()
            => wire::SERVER_PACKET_SIZE - 16,
    }
}

#[test]
fn sessions() {
    use rom::{
        CoopSessionKind as K, CoopSessionOp as O, CoopSessionReason as R, CoopSessionState as S,
    };
    same! {
        O::COOP_SESSION_OP_NONE => wire::SESSION_OP_NONE,
        O::COOP_SESSION_OP_REQUEST => wire::SESSION_OP_REQUEST,
        O::COOP_SESSION_OP_ACCEPT => wire::SESSION_OP_ACCEPT,
        O::COOP_SESSION_OP_DECLINE => wire::SESSION_OP_DECLINE,
        O::COOP_SESSION_OP_CANCEL => wire::SESSION_OP_CANCEL,
        K::COOP_SESSION_KIND_TRADE => wire::SESSION_KIND_TRADE,
        K::COOP_SESSION_KIND_BATTLE => wire::SESSION_KIND_BATTLE,
        K::COOP_SESSION_KIND_TEAM_UP => wire::SESSION_KIND_TEAM_UP,
        K::COOP_SESSION_KIND_LEAVE_TEAM => wire::SESSION_KIND_LEAVE_TEAM,
        K::COOP_SESSION_KIND_TEAM_BATTLE => wire::SESSION_KIND_TEAM_BATTLE,
        K::COOP_SESSION_KIND_REVIVE => wire::SESSION_KIND_REVIVE,
        S::COOP_SESSION_NONE => wire::SESSION_STATE_NONE,
        S::COOP_SESSION_OUTGOING => wire::SESSION_STATE_OUTGOING,
        S::COOP_SESSION_INCOMING => wire::SESSION_STATE_INCOMING,
        S::COOP_SESSION_ACCEPTED => wire::SESSION_STATE_ACCEPTED,
        S::COOP_SESSION_DECLINED => wire::SESSION_STATE_DECLINED,
        S::COOP_SESSION_CANCELLED => wire::SESSION_STATE_CANCELLED,
        S::COOP_SESSION_TIMEOUT => wire::SESSION_STATE_TIMEOUT,
        S::COOP_SESSION_REFUSED => wire::SESSION_STATE_REFUSED,
        S::COOP_SESSION_NOTICE => wire::SESSION_STATE_NOTICE,
        R::COOP_SESSION_REASON_NONE => wire::SESSION_REASON_NONE,
        R::COOP_SESSION_REASON_NOT_HERE => wire::SESSION_REASON_NOT_HERE,
        R::COOP_SESSION_REASON_BUSY => wire::SESSION_REASON_BUSY,
        R::COOP_SESSION_REASON_TEAM_RULE => wire::SESSION_REASON_TEAM_RULE,
        R::COOP_SESSION_REASON_ALREADY_TEAMED => wire::SESSION_REASON_ALREADY_TEAMED,
        R::COOP_SESSION_REASON_INVALID => wire::SESSION_REASON_INVALID,
        R::COOP_SESSION_REASON_TEAMMATE_AWAY => wire::SESSION_REASON_TEAMMATE_AWAY,
    }
}

#[test]
fn player_state_values() {
    use rom::{CoopActivity as A, CoopSaveStatus as V};
    same! {
        A::COOP_ACTIVITY_BATTLE => wire::ACTIVITY_BATTLE,
        V::COOP_SAVE_UNKNOWN => wire::SAVE_STATUS_UNKNOWN,
        V::COOP_SAVE_FRESH => wire::SAVE_STATUS_FRESH,
        V::COOP_SAVE_IN_PROGRESS => wire::SAVE_STATUS_IN_PROGRESS,
        rom::COOP_PLAYER_FLAG_SPIRIT => wire::CHALLENGE_FLAG_SPIRIT,
    }
}

#[test]
fn trades() {
    use rom::{CoopTradeAbortReason as X, CoopTradeInState as I, CoopTradeOutState as O};
    same! {
        O::COOP_TRADE_OUT_NONE => wire::TRADE_OUT_NONE,
        O::COOP_TRADE_OUT_PICKING => wire::TRADE_OUT_PICKING,
        O::COOP_TRADE_OUT_OFFERED => wire::TRADE_OUT_OFFERED,
        O::COOP_TRADE_OUT_CONFIRMED => wire::TRADE_OUT_CONFIRMED,
        O::COOP_TRADE_OUT_CANCEL => wire::TRADE_OUT_CANCEL,
        O::COOP_TRADE_OUT_DONE => wire::TRADE_OUT_DONE,
        I::COOP_TRADE_IN_WAITING => wire::TRADE_IN_WAITING,
        I::COOP_TRADE_IN_OFFERED => wire::TRADE_IN_OFFERED,
        I::COOP_TRADE_IN_COMMIT => wire::TRADE_IN_COMMIT,
        I::COOP_TRADE_IN_ABORTED => wire::TRADE_IN_ABORTED,
        X::COOP_TRADE_ABORT_CANCELLED => wire::TRADE_ABORT_CANCELLED,
        X::COOP_TRADE_ABORT_LEFT => wire::TRADE_ABORT_LEFT,
        X::COOP_TRADE_ABORT_TIMEOUT => wire::TRADE_ABORT_TIMEOUT,
        rom::COOP_TRADE_FLAG_REVIVE => wire::TRADE_FLAG_REVIVE,
        rom::COOP_TRADE_FLAG_RECEIVER => wire::TRADE_FLAG_RECEIVER,
    }
}

#[test]
fn virtual_link() {
    use rom::{CoopLinkMsgType as M, CoopLinkRole as R};
    same! {
        M::COOP_LINK_MSG_DATA => wire::LINK_MSG_DATA,
        M::COOP_LINK_MSG_CONNECT => wire::LINK_MSG_CONNECT,
        M::COOP_LINK_MSG_ACCEPT => wire::LINK_MSG_ACCEPT,
        M::COOP_LINK_MSG_REFUSE => wire::LINK_MSG_REFUSE,
        M::COOP_LINK_MSG_BYE => wire::LINK_MSG_BYE,
        R::COOP_LINK_ROLE_NONE => wire::LINK_ROLE_NONE,
        R::COOP_LINK_ROLE_PARENT => wire::LINK_ROLE_PARENT,
        R::COOP_LINK_ROLE_CHILD => wire::LINK_ROLE_CHILD,
        rom::COOP_LINK_DATA_MAX => wire::LINK_DATA_MAX,
        rom::COOP_LINK_BEACON_MAX => wire::LINK_BEACON_MAX,
    }
}

#[test]
fn challenge_rules_and_trailers() {
    same! {
        rom::COOP_RULE_RANDOM_MASK => wire::CHALLENGE_RULE_RANDOM_MASK,
        rom::COOP_RULE_LEGENDS_WILD => wire::CHALLENGE_RULE_LEGENDS_WILD,
        rom::COOP_RULE_RANDOM_ABILITIES => wire::CHALLENGE_RULE_RANDOM_ABILITIES,
        rom::COOP_RULE_NUZLOCKE => wire::CHALLENGE_RULE_NUZLOCKE,
        rom::COOP_RULE_SOUL_LINK_SHIFT => wire::CHALLENGE_RULE_SOUL_LINK_SHIFT,
        rom::COOP_RULE_SOUL_LINK_MASK => wire::CHALLENGE_RULE_SOUL_LINK_MASK,
        rom::COOP_RULE_MONOLOCKE => wire::CHALLENGE_RULE_MONOLOCKE,
        rom::COOP_RULE_LEVEL_CAP => wire::CHALLENGE_RULE_LEVEL_CAP,
        rom::COOP_RULE_RANDOM_ITEMS => wire::CHALLENGE_RULE_RANDOM_ITEMS,
        rom::COOP_RULES_VALID => wire::CHALLENGE_RULES_VALID,
        rom::COOP_SOUL_LINK_OFF => wire::SOUL_LINK_OFF,
        rom::COOP_SOUL_LINK_ROOM_WIDE => wire::SOUL_LINK_ROOM_WIDE,
        rom::COOP_SOUL_LINK_DUO => wire::SOUL_LINK_DUO,
        rom::COOP_MONO_TYPE_COUNT => wire::MONO_TYPE_COUNT,
        rom::COOP_MONO_TYPE_NONE => wire::MONO_TYPE_NONE,
        rom::COOP_ROOM_RUN_LOST => wire::CHALLENGE_ROOM_RUN_LOST,
        rom::COOP_ROOM_RUN_LOST => wire::ROOM_STATE_RUN_LOST,
        rom::COOP_LOCATION_BYTES => wire::CHALLENGE_LOCATION_BYTES,
        rom::COOP_CHALLENGE_WIRE_MAGIC => wire::CHALLENGE_MAGIC,
        rom::COOP_CHALLENGE_WIRE_VERSION => wire::CHALLENGE_VERSION,
        rom::COOP_CHALLENGE_UP_SIZE => wire::CHALLENGE_UP_SIZE,
        rom::COOP_CHALLENGE_DOWN_SIZE => wire::CHALLENGE_DOWN_SIZE,
        size_of::<rom::CoopChallengeUp>() => wire::CHALLENGE_UP_SIZE,
        size_of::<rom::CoopChallengeDown>() => wire::CHALLENGE_DOWN_SIZE,
        rom::COOP_CHALLENGE_LATCHED => wire::CHALLENGE_STATE_LATCHED,
        rom::COOP_CHALLENGE_SPIRIT => wire::CHALLENGE_STATE_SPIRIT,
        rom::COOP_CHALLENGE_RUN_LOST => wire::CHALLENGE_STATE_RUN_LOST,
        rom::COOP_CHALLENGE_HAD_POKEMON => wire::CHALLENGE_STATE_HAD_POKEMON,
    }
}
