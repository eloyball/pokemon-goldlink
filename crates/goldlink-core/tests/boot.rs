//! UC1's exit criteria with the real game. They need your HeartGold (USA) ROM and the built
//! co-op ROM, never in the repository, so they're ignored by default:
//!
//!     cargo test -p goldlink-core -- --ignored
//!
//! with `hg-engine/rom.nds` and `hg-engine/test.nds` in place (or `GOLDLINK_BASE_ROM` /
//! `GOLDLINK_TEST_ROM` pointing at them).

use std::path::PathBuf;

use chrono::NaiveDate;
use goldlink_core::{FirmwareUser, Nds};

/// coop_offsets.json `main_ram_offset` (pinned by hg-engine's linker script).
const MAILBOX: usize = 0x3D8060;

fn repo() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn read(var: &str, default: &str) -> Vec<u8> {
    let path = std::env::var_os(var)
        .map(PathBuf::from)
        .unwrap_or_else(|| repo().join(default));
    std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e} (set {var})", path.display()))
}

fn fixed_clock(nds: &mut Nds) {
    nds.set_rtc(
        NaiveDate::from_ymd_opt(2026, 11, 7)
            .unwrap()
            .and_hms_opt(12, 0, 0)
            .unwrap(),
    );
}

/// Exit criterion 1: the co-op ROM boots on FreeBIOS + generated firmware, publishes the mailbox
/// (v7) and its heartbeat advances (the co-op SysTask runs every frame from the title screen on).
fn boot_and_check(game: &[u8]) -> Nds {
    let mut nds = Nds::new(&FirmwareUser::default(), None).expect("melonDS starts");
    nds.load_rom(game, None).expect("melonDS reads the ROM");
    fixed_clock(&mut nds);
    for _ in 0..600 {
        nds.run_frame();
    }
    let first = nds
        .mailbox(MAILBOX)
        .expect("the mailbox ('COOP') is published after 600 frames");
    assert_eq!(first.version, goldlink_abi::COOP_MAILBOX_VERSION as u16);
    for _ in 0..300 {
        nds.run_frame();
    }
    let second = nds.mailbox(MAILBOX).unwrap();
    assert!(
        second.heartbeat_rom > first.heartbeat_rom,
        "heartbeat_rom advances: {} -> {}",
        first.heartbeat_rom,
        second.heartbeat_rom
    );
    nds
}

#[test]
#[ignore = "needs hg-engine/test.nds"]
fn boots_and_publishes_the_mailbox() {
    let game = read("GOLDLINK_TEST_ROM", "hg-engine/test.nds");
    boot_and_check(&game);
}

/// Exit criterion 2: two DSes in one process, each with its own state.
#[test]
#[ignore = "needs hg-engine/test.nds"]
fn two_instances_in_one_process() {
    let game = read("GOLDLINK_TEST_ROM", "hg-engine/test.nds");
    let mut a = boot_and_check(&game);
    let mut b = Nds::new(&FirmwareUser::default(), None).unwrap();
    b.load_rom(&game, None).unwrap();
    fixed_clock(&mut b);
    // A runs on while B boots: their heartbeats differ, then both advance.
    for _ in 0..600 {
        a.run_frame();
        b.run_frame();
    }
    let (ha, hb) = (
        a.mailbox(MAILBOX).unwrap().heartbeat_rom,
        b.mailbox(MAILBOX).unwrap().heartbeat_rom,
    );
    assert!(ha > hb, "A has run 900 frames longer than B ({ha} vs {hb})");
    for _ in 0..60 {
        a.run_frame();
        b.run_frame();
    }
    assert!(a.mailbox(MAILBOX).unwrap().heartbeat_rom > ha);
    assert!(b.mailbox(MAILBOX).unwrap().heartbeat_rom > hb);
    // Writes to one don't show in the other.
    a.main_ram_mut()[MAILBOX + 0x1DC] = 0x5A;
    assert_ne!(b.main_ram()[MAILBOX + 0x1DC], 0x5A);
}

/// Exit criterion 3: the patch applied in memory gives exactly the co-op ROM.
#[test]
#[ignore = "needs hg-engine/rom.nds and hg-engine/test.nds"]
fn patches_the_base_rom_in_memory() {
    let base = read("GOLDLINK_BASE_ROM", "hg-engine/rom.nds");
    let game = read("GOLDLINK_TEST_ROM", "hg-engine/test.nds");
    let patch = goldlink_rom::bps::create(&base, &game);
    let release = goldlink_rom::Release {
        version: "test".into(),
        rom_sha1: goldlink_rom::sha1_hex(&base),
        patched_sha1: goldlink_rom::sha1_hex(&game),
        patch: "goldlink.bps".into(),
        repository: String::new(),
    };
    let patched = goldlink_rom::prepare(&base, &patch, &release).expect("the base ROM patches");
    assert!(patched == game, "the patched bytes are the co-op ROM");
    boot_and_check(&patched);
}

/// Determinism (UC4's in-process tests need it): the same ROM, clock and inputs give the same RAM.
#[test]
#[ignore = "needs hg-engine/test.nds"]
fn same_inputs_same_state() {
    let game = read("GOLDLINK_TEST_ROM", "hg-engine/test.nds");
    let run = || {
        let mut nds = Nds::new(&FirmwareUser::default(), None).unwrap();
        nds.load_rom(&game, None).unwrap();
        fixed_clock(&mut nds);
        for i in 0..900u32 {
            let keys = if i % 120 < 3 {
                goldlink_core::Keys::A
            } else {
                goldlink_core::Keys::NONE
            };
            nds.set_input(keys, None);
            nds.run_frame();
        }
        nds.main_ram().to_vec()
    };
    assert!(
        run() == run(),
        "two runs with the same inputs end with the same main RAM"
    );
}
