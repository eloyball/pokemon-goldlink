//! UC2's exit criteria with the real game (ignored by default; they need the built co-op ROM and
//! the regression's bedroom save, never in the repository):
//!
//!     cargo test -p goldlink-runtime --release -- --ignored
//!
//! `GOLDLINK_TEST_ROM` (default `hg-engine/test.nds`) and `GOLDLINK_BEDROOM_SAVE` (default
//! `BizHawk/coop_saves/TEST03-BC71B831.sav`, the `start-save` regression's save).

use std::mem::offset_of;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use chrono::NaiveDate;
use goldlink_abi::{CoopCommBlock, CoopDiagnostics, CoopPlayerState, CoopProgressionGates};
use goldlink_core::Keys;
use goldlink_runtime::{
    Command, Emulator, Event, FrameHook, GameSetup, Input, Runner, screen_hash,
};

const MAILBOX: usize = 0x3D8060;
const WORLD: &str = "TEST03-BC71B831";

fn repo() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn file(var: &str, default: &str) -> PathBuf {
    std::env::var_os(var)
        .map(PathBuf::from)
        .unwrap_or_else(|| repo().join(default))
}

fn rom() -> Arc<Vec<u8>> {
    let p = file("GOLDLINK_TEST_ROM", "hg-engine/test.nds");
    Arc::new(std::fs::read(&p).unwrap_or_else(|e| panic!("{}: {e}", p.display())))
}

fn temp_data(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("goldlink-rt-{name}-{}", std::process::id()));
    std::fs::remove_dir_all(&d).ok();
    std::fs::create_dir_all(d.join("coop_saves")).unwrap();
    d
}

fn setup(rom: Arc<Vec<u8>>, data: PathBuf) -> GameSetup {
    GameSetup {
        rom,
        data_dir: data,
        world: WORLD.into(),
        mailbox_offset: MAILBOX,
        rtc: Some(
            NaiveDate::from_ymd_opt(2026, 11, 8)
                .unwrap()
                .and_hms_opt(12, 0, 0)
                .unwrap(),
        ),
        backups: false,
    }
}

/// A scripted player: presses Start or A now and then, like the probes' boot loops.
fn scripted(frame: u64) -> Input {
    let keys = match frame % 60 {
        20..=22 if frame % 120 < 60 => Keys::START,
        20..=22 => Keys::A,
        40..=41 if frame % 180 < 60 => Keys::DOWN,
        _ => Keys::NONE,
    };
    let touch = (frame % 300 == 150).then_some((128, 96));
    Input { keys, touch }
}

/// Exit criterion (RFC §6 M2, automated): an input replay reproduces the framebuffer hash. The
/// same script twice, once frame by frame and once on the emulation thread through a FrameHook
/// (the path the window uses), gives the same screens.
#[test]
#[ignore = "needs hg-engine/test.nds"]
fn input_replay_reproduces_the_screens() {
    const FRAMES: u64 = 1500;
    let rom = rom();

    let mut a = Emulator::new(setup(rom.clone(), temp_data("replay-a"))).unwrap();
    let mut hashes = Vec::new();
    for f in 0..FRAMES {
        a.frame(scripted(f));
        if f % 500 == 499 {
            hashes.push(screen_hash(&a));
        }
    }
    let direct = screen_hash(&a);

    struct Replay(u64);
    impl FrameHook for Replay {
        fn pre_frame(&mut self, _: &mut Emulator, input: &mut Input) {
            *input = scripted(self.0);
            self.0 += 1;
        }
    }
    let b = Emulator::new(setup(rom, temp_data("replay-b"))).unwrap();
    let runner = Runner::start(b, vec![Box::new(Replay(0))]);
    runner.send(Command::Speed(None));
    let (tx, rx) = std::sync::mpsc::channel();
    // Stop exactly at FRAMES: a Call runs between frames, so poll the count from there.
    loop {
        let tx = tx.clone();
        runner.send(Command::Call(Box::new(move |e: &mut Emulator| {
            tx.send(e.frame_count()).ok();
        })));
        let n = rx.recv().unwrap();
        if n + 200 >= FRAMES {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    runner.send(Command::Pause(true));
    let (htx, hrx) = std::sync::mpsc::channel();
    // Paused: step the rest by hand on the thread.
    runner.send(Command::Call(Box::new(move |e: &mut Emulator| {
        let mut f = e.frame_count();
        while f < FRAMES {
            e.frame(scripted(f));
            f += 1;
        }
        htx.send(screen_hash(e)).ok();
    })));
    let threaded = hrx.recv().unwrap();
    runner.send(Command::Pause(false));
    drop(runner.stop());

    assert_ne!(
        hashes[0], hashes[2],
        "the screens should change over the script"
    );
    assert_eq!(direct, threaded, "the replay gives the same screens");
}

/// Plays the bridge the way the probes do: heartbeat, connected, the world's save loaded, and
/// (once `started` is set) the relay's roster lock.
struct FakeBridge {
    hb: u32,
    started: Arc<AtomicBool>,
}

impl FrameHook for FakeBridge {
    fn post_frame(&mut self, emu: &mut Emulator) {
        if emu.mailbox().is_none() {
            return;
        }
        self.hb += 1;
        let at = emu.mailbox_offset();
        let ram = emu.nds.main_ram_mut();
        let w = |ram: &mut [u8], off: usize, bytes: &[u8]| {
            ram[at + off..at + off + bytes.len()].copy_from_slice(bytes)
        };
        w(
            ram,
            offset_of!(CoopCommBlock, heartbeat_bridge),
            &self.hb.to_le_bytes(),
        );
        w(ram, offset_of!(CoopCommBlock, is_connected_to_room), &[1]);
        w(ram, offset_of!(CoopCommBlock, save_slot_ready), &[1]);
        if self.started.load(Ordering::Relaxed) {
            w(
                ram,
                offset_of!(CoopCommBlock, gates)
                    + offset_of!(CoopProgressionGates, flag_campaign_started),
                &[1],
            );
        }
    }
}

fn mb_u8(emu: &Emulator, off: usize) -> u8 {
    emu.mailbox().map_or(0, |m| m[off])
}

fn in_field(emu: &Emulator) -> bool {
    mb_u8(
        emu,
        offset_of!(CoopCommBlock, diag) + offset_of!(CoopDiagnostics, field_entry_status),
    ) == 3
        && mb_u8(
            emu,
            offset_of!(CoopCommBlock, local_player) + offset_of!(CoopPlayerState, is_active),
        ) == 1
}

fn save_counter(emu: &Emulator) -> u32 {
    let at = offset_of!(CoopCommBlock, save_counter);
    emu.mailbox()
        .map_or(0, |m| u32::from_le_bytes(m[at..at + 4].try_into().unwrap()))
}

/// Exit criterion (RFC §6 M2, saves): the game's own save reaches the world's file through
/// melonDS's save callback, and a reboot continues from it. The bedroom save's adventure-start
/// save (the `start-save` probe) is a real in-game save the ROM makes by itself.
#[test]
#[ignore = "needs hg-engine/test.nds and the bedroom save"]
fn in_game_save_reaches_the_world_file_and_reloads() {
    let data = temp_data("save");
    let bedroom = std::fs::read(file(
        "GOLDLINK_BEDROOM_SAVE",
        "BizHawk/coop_saves/TEST03-BC71B831.sav",
    ))
    .expect("the bedroom save");
    std::fs::write(
        data.join("coop_saves").join(format!("{WORLD}.sav")),
        &bedroom,
    )
    .unwrap();

    let started = Arc::new(AtomicBool::new(false));
    let mut bridge = FakeBridge {
        hb: 0,
        started: started.clone(),
    };
    let mut emu = Emulator::new(setup(rom(), data.clone())).unwrap();
    let mut step = |emu: &mut Emulator, f: u64, input: Input| {
        let mut input = input;
        bridge.pre_frame(emu, &mut input);
        emu.frame(input);
        bridge.post_frame(emu);
        let _ = f;
    };

    // Boot to the overworld (Continue).
    let mut f = 0;
    while !in_field(&emu) {
        assert!(f < 4000, "never reached the overworld");
        let keys = if f % 60 >= 20 && f % 60 < 23 {
            if f % 120 < 60 { Keys::START } else { Keys::A }
        } else {
            Keys::NONE
        };
        step(&mut emu, f, Input { keys, touch: None });
        f += 1;
    }
    for _ in 0..90 {
        step(&mut emu, f, Input::default());
        f += 1;
    }
    let counter0 = save_counter(&emu);
    emu.take_events();

    // The relay locks the roster: the ROM shows its message and saves.
    started.store(true, Ordering::Relaxed);
    for _ in 0..480 {
        step(&mut emu, f, Input::default());
        f += 1;
    }
    for i in 0..6 {
        step(
            &mut emu,
            f,
            Input {
                keys: if i < 3 { Keys::A } else { Keys::NONE },
                touch: None,
            },
        );
        f += 1;
    }
    let mut saved = false;
    for _ in 0..2400 {
        step(&mut emu, f, Input::default());
        f += 1;
        if emu.take_events().contains(&Event::Saved) {
            saved = true;
            break;
        }
    }
    assert!(
        saved,
        "no Saved event (save_counter {counter0} -> {})",
        save_counter(&emu)
    );
    assert_ne!(save_counter(&emu), counter0, "the ROM finished a save");
    let on_disk = std::fs::read(emu.save_path()).unwrap();
    assert_ne!(on_disk, bedroom, "the world's file changed");
    assert_eq!(
        on_disk.as_slice(),
        emu.nds.save(),
        "the file holds the cart's save"
    );
    assert!(
        data.join("coop_saves")
            .join(format!("{WORLD}.sav.bak"))
            .is_file()
    );

    // Reboot: the cart gets the file, and the game continues from it to the overworld (a
    // damaged save would stop at the title's "the save data is corrupted" message). The ROM
    // runs its adventure-start save again then: a bedroom save with an empty party still
    // counts as fresh (coop_ghosts.c, COOP_SAVE_FRESH), on BizHawk too.
    emu.boot().unwrap();
    assert_eq!(emu.nds.save(), on_disk.as_slice());
    let mut g = 0;
    while !in_field(&emu) {
        assert!(g < 4000, "never reached the overworld after the reboot");
        let keys = if g % 60 >= 20 && g % 60 < 23 {
            if g % 120 < 60 { Keys::START } else { Keys::A }
        } else {
            Keys::NONE
        };
        step(&mut emu, g, Input { keys, touch: None });
        g += 1;
    }
    std::fs::remove_dir_all(&data).ok();
}
