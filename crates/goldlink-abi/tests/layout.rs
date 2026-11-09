//! The co-op interface's frozen layout, as numbers. `generated.rs` already asserts every size
//! and offset clang computed for the ARM9; these tests pin the contract itself, so a header change
//! that moves the mailbox (and its own `_Static_assert` with it) still fails here after a
//! regeneration, instead of silently changing what the client reads.

use core::mem::{offset_of, size_of};

use goldlink_abi::*;

#[test]
fn mailbox_block_sizes() {
    assert_eq!(size_of::<CoopCommBlock>(), 512);
    assert_eq!(size_of::<CoopPlayerState>(), 64);
    assert_eq!(size_of::<CoopProgressionGates>(), 24);
    assert_eq!(size_of::<CoopDiagnostics>(), 24);
    assert_eq!(size_of::<CoopSessionOut>(), 8);
}

#[test]
fn mailbox_offsets_the_bridge_and_probes_use() {
    // coop_sync.h's offset comments; the bridge and every probe address these directly.
    assert_eq!(offset_of!(CoopCommBlock, heartbeat_rom), 0x008);
    assert_eq!(offset_of!(CoopCommBlock, local_player), 0x010);
    assert_eq!(offset_of!(CoopCommBlock, remote_players), 0x050);
    assert_eq!(offset_of!(CoopCommBlock, gates), 0x190);
    assert_eq!(offset_of!(CoopCommBlock, is_connected_to_room), 0x1A8);
    assert_eq!(offset_of!(CoopCommBlock, room_rejection), 0x1AE);
    assert_eq!(offset_of!(CoopCommBlock, heartbeat_bridge), 0x1B0);
    assert_eq!(offset_of!(CoopCommBlock, diag), 0x1C4);
    assert_eq!(offset_of!(CoopCommBlock, debug_cmd), 0x1DC);
    assert_eq!(offset_of!(CoopCommBlock, save_slot_ready), 0x1E4);
    assert_eq!(offset_of!(CoopCommBlock, exchange_ptr), 0x1FC);
    assert_eq!(offset_of!(CoopPlayerState, activity), 0x13);
    assert_eq!(offset_of!(CoopPlayerState, trainer_id), 0x14);
    assert_eq!(offset_of!(CoopPlayerState, challenge_flags), 0x3F);
}

#[test]
fn trade_and_link_block_sizes() {
    assert_eq!(size_of::<CoopTradeOut>(), 244);
    assert_eq!(size_of::<CoopTradeIn>(), 252);
    assert_eq!(size_of::<CoopTradeExchange>(), 0x1F8);
    assert_eq!(size_of::<CoopLinkMsg>(), 0xC4);
    assert_eq!(size_of::<CoopLinkBeacon>(), 0x78);
    assert_eq!(size_of::<CoopLinkBlock>(), 0xDB0);
    // The message link (link battle rework phase 1) retired the send callbacks: 0xD9C is unused.
    assert_eq!(offset_of!(CoopLinkBlock, unused_d9c), 0xD9C);
}

#[test]
fn challenge_sync_layout() {
    assert_eq!(
        size_of::<CoopChallengeUp>(),
        COOP_CHALLENGE_UP_SIZE as usize
    );
    assert_eq!(
        size_of::<CoopChallengeDown>(),
        COOP_CHALLENGE_DOWN_SIZE as usize
    );
    assert_eq!(offset_of!(CoopChallengeUp, failed), 0x1C);
    assert_eq!(offset_of!(CoopChallengeUp, checksum), 0x2C);
    assert_eq!(offset_of!(CoopChallengeDown, link_union), 0x0C);
    assert_eq!(offset_of!(CoopChallengeDown, failed), 0x2C);
    assert_eq!(offset_of!(CoopChallengeDown, checksum), 0x3C);
    assert_eq!(size_of::<CoopChallengeSync>(), 0x78);
    assert_eq!(offset_of!(CoopChallengeSync, out), 0x08);
    assert_eq!(offset_of!(CoopChallengeSync, in_), 0x38);
    // Overlay 131's pinned header: the bridge follows the pointer at +0x0C.
    assert_eq!(offset_of!(CoopFieldEntryHeader, challenge), 0x0C);
    assert_eq!(COOP_FIELD_ENTRY_ADDR, 0x023C_4000);
}

#[test]
fn magics_and_versions() {
    assert_eq!(&COOP_MAGIC.to_le_bytes(), b"POOC"); // "COOP" read as a big-endian word
    assert_eq!(COOP_MAILBOX_VERSION, 7);
    assert_eq!(&COOP_FIELD_ENTRY_MAGIC.to_le_bytes(), b"COF1");
    assert_eq!(&COOP_CHALLENGE_SYNC_MAGIC.to_le_bytes(), b"CHS1");
    assert_eq!(&COOP_LINK_MAGIC.to_le_bytes(), b"CLK1");
}

/// The build's mailbox symbol (scripts/extract_coop_sym.py) against this crate. Only with
/// `--features offsets-json`: the file is a build output of the ROM, not of this crate.
#[cfg(feature = "offsets-json")]
#[test]
fn coop_offsets_json_matches() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../coop_offsets.json");
    let text = std::fs::read_to_string(path).expect("coop_offsets.json");
    let field = |key: &str| -> u64 {
        let at = text
            .find(&format!("\"{key}\""))
            .unwrap_or_else(|| panic!("{key} in coop_offsets.json"));
        let rest = &text[at + key.len() + 2..];
        let digits: String = rest
            .chars()
            .skip_while(|c| !c.is_ascii_digit())
            .take_while(|c| c.is_ascii_digit())
            .collect();
        digits.parse().unwrap()
    };
    assert_eq!(field("mailbox_version"), u64::from(COOP_MAILBOX_VERSION));
    assert_eq!(field("mailbox_size"), size_of::<CoopCommBlock>() as u64);
}
