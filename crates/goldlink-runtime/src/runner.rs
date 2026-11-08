//! The emulation thread (RFC §4.4): the DS runs at its own 59.8261 Hz, independent of the
//! display's refresh. The window reads the latest finished frame and writes the input; audio
//! comes out through a ring the sound card drains, and the ring's fill nudges the pace so the
//! two clocks don't drift apart.

use std::collections::VecDeque;
use std::sync::mpsc::{Receiver, Sender, TryRecvError};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use goldlink_core::{SCREEN_HEIGHT, SCREEN_WIDTH};

use crate::emulator::{Emulator, Event, Input};

/// The DS's frame rate (33,513,982 Hz / 560,190 cycles per frame).
pub const DS_FPS: f64 = 59.826_098_5;

/// Code the emulation thread runs around every frame: the co-op bridge (UC3) and dev tools.
pub trait FrameHook: Send {
    /// Before the frame runs, with the frame's input (a hook may change it: replays, scripts).
    fn pre_frame(&mut self, _emu: &mut Emulator, _input: &mut Input) {}
    /// After the frame ran and the save and backups were handled.
    fn post_frame(&mut self, _emu: &mut Emulator) {}
}

pub enum Command {
    /// Write the save and stop; the thread hands the Emulator back.
    Quit,
    Pause(bool),
    /// Speed in percent of the DS (100 = normal; 10..=1000), or None: as fast as it goes (tests).
    Speed(Option<f64>),
    /// The sound card's sample rate.
    AudioRate(f64),
    /// Restore a backup or savestate file.
    LoadState(std::path::PathBuf),
    /// Power cycle on the save file.
    Reboot,
    /// Runs on the emulation thread between frames (dev tools, tests).
    Call(Box<dyn FnOnce(&mut Emulator) + Send>),
}

/// The latest finished frame: both screens, top then bottom, 256 x 384, 0x00RRGGBB.
pub struct Frame {
    pub pixels: Vec<u32>,
    /// Counts frames; changes when the pixels do.
    pub seq: u64,
}

/// Interleaved stereo i16 samples waiting for the sound card.
pub struct AudioRing {
    samples: VecDeque<i16>,
    /// Upper bound (samples); older ones are dropped beyond it.
    cap: usize,
}

impl AudioRing {
    fn new() -> Self {
        Self {
            samples: VecDeque::new(),
            cap: 48_000, // half a second of stereo at 48 kHz
        }
    }

    pub fn len(&self) -> usize {
        self.samples.len()
    }

    pub fn is_empty(&self) -> bool {
        self.samples.is_empty()
    }

    fn push(&mut self, data: &[i16]) {
        self.samples.extend(data);
        let excess = self.samples.len().saturating_sub(self.cap);
        // Keep stereo pairs aligned.
        self.samples.drain(..excess & !1);
    }

    /// Fills `out` (interleaved stereo); silence where the ring runs dry.
    pub fn pop_into(&mut self, out: &mut [i16]) -> usize {
        let n = out.len().min(self.samples.len());
        for (o, s) in out.iter_mut().zip(self.samples.drain(..n)) {
            *o = s;
        }
        out[n..].fill(0);
        n
    }

    pub fn clear(&mut self) {
        self.samples.clear();
    }
}

/// State the window and the emulation thread share.
pub struct Shared {
    pub input: Mutex<Input>,
    pub frame: Mutex<Frame>,
    pub audio: Mutex<AudioRing>,
    pub events: Mutex<Vec<Event>>,
    /// Measured emulated frames per second, updated every second.
    pub fps: Mutex<f64>,
}

pub fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

pub struct Runner {
    pub shared: Arc<Shared>,
    commands: Sender<Command>,
    thread: Option<JoinHandle<Emulator>>,
}

impl Runner {
    /// Starts the emulation thread.
    pub fn start(emu: Emulator, hooks: Vec<Box<dyn FrameHook>>) -> Self {
        let shared = Arc::new(Shared {
            input: Mutex::new(Input::default()),
            frame: Mutex::new(Frame {
                pixels: vec![0; SCREEN_WIDTH * SCREEN_HEIGHT * 2],
                seq: 0,
            }),
            audio: Mutex::new(AudioRing::new()),
            events: Mutex::new(Vec::new()),
            fps: Mutex::new(0.0),
        });
        let (tx, rx) = std::sync::mpsc::channel();
        let s = shared.clone();
        let thread = std::thread::Builder::new()
            .name("emulation".into())
            .spawn(move || run(emu, hooks, s, rx))
            .expect("can't start the emulation thread");
        Self {
            shared,
            commands: tx,
            thread: Some(thread),
        }
    }

    pub fn send(&self, cmd: Command) {
        self.commands.send(cmd).ok();
    }

    /// True once the thread ended (it panicked, or after Quit).
    pub fn finished(&self) -> bool {
        self.thread.as_ref().is_none_or(|t| t.is_finished())
    }

    /// Writes the save, stops the thread and returns the Emulator (None if it panicked).
    pub fn stop(mut self) -> Option<Emulator> {
        self.send(Command::Quit);
        self.thread.take()?.join().ok()
    }
}

impl Drop for Runner {
    fn drop(&mut self) {
        if let Some(t) = self.thread.take() {
            self.commands.send(Command::Quit).ok();
            t.join().ok();
        }
    }
}

/// Keeps the frame rate: sleeps until the next frame is due.
pub struct Pacer {
    next: Option<Instant>,
}

impl Pacer {
    pub fn new() -> Self {
        Self { next: None }
    }

    /// `fps`: frames per second wanted now.
    pub fn wait(&mut self, fps: f64) {
        let period = Duration::from_secs_f64(1.0 / fps.max(1.0));
        let now = Instant::now();
        let due = self.next.map_or(now + period, |t| t + period);
        if due > now {
            std::thread::sleep(due - now);
            self.next = Some(due);
        } else if now - due > period * 4 {
            // Far behind (a stall, a breakpoint): don't race to catch up.
            self.next = Some(now);
        } else {
            self.next = Some(due);
        }
    }

    pub fn reset(&mut self) {
        self.next = None;
    }
}

impl Default for Pacer {
    fn default() -> Self {
        Self::new()
    }
}

/// The pace nudge from the audio ring: 1.0 at the target fill, up to ±0.5% either way. Fuller
/// than the target → slower, so the sound card catches up; emptier → faster.
pub fn audio_nudge(fill: usize, target: usize) -> f64 {
    if target == 0 {
        return 1.0;
    }
    let err = (target as f64 - fill as f64) / target as f64;
    1.0 + 0.005 * err.clamp(-1.0, 1.0)
}

fn run(
    mut emu: Emulator,
    mut hooks: Vec<Box<dyn FrameHook>>,
    shared: Arc<Shared>,
    commands: Receiver<Command>,
) -> Emulator {
    let mut pacer = Pacer::new();
    let mut paused = false;
    let mut speed: Option<f64> = Some(100.0);
    let mut audio_rate: Option<f64> = None;
    let mut audio_buf = vec![0i16; 4096];
    let mut pixels = Vec::with_capacity(SCREEN_WIDTH * SCREEN_HEIGHT * 2);
    let mut fps_window = (Instant::now(), 0u32);
    loop {
        // Commands (blocking while paused).
        loop {
            let cmd = if paused {
                match commands.recv() {
                    Ok(c) => c,
                    Err(_) => return finish(emu),
                }
            } else {
                match commands.try_recv() {
                    Ok(c) => c,
                    Err(TryRecvError::Empty) => break,
                    Err(TryRecvError::Disconnected) => return finish(emu),
                }
            };
            match cmd {
                Command::Quit => return finish(emu),
                Command::Pause(p) => {
                    paused = p;
                    pacer.reset();
                    lock(&shared.audio).clear();
                }
                Command::Speed(s) => {
                    speed = s.map(|s| s.clamp(10.0, 1000.0));
                    pacer.reset();
                }
                Command::AudioRate(r) => {
                    audio_rate = Some(r);
                    emu.nds.set_audio_rate(r);
                }
                Command::LoadState(path) => {
                    if let Err(e) = emu.load_state_file(&path) {
                        lock(&shared.events).push(Event::BackupFailed(e));
                    }
                }
                Command::Reboot => {
                    emu.flush();
                    if let Err(e) = emu.boot() {
                        lock(&shared.events).push(Event::SaveFailed(e));
                    }
                }
                Command::Call(f) => f(&mut emu),
            }
        }

        let mut input = *lock(&shared.input);
        for h in hooks.iter_mut() {
            h.pre_frame(&mut emu, &mut input);
        }
        emu.frame(input);
        for h in hooks.iter_mut() {
            h.post_frame(&mut emu);
        }

        emu.screens(&mut pixels);
        {
            let mut f = lock(&shared.frame);
            std::mem::swap(&mut f.pixels, &mut pixels);
            f.seq = emu.frame_count();
        }
        let events = emu.take_events();
        if !events.is_empty() {
            lock(&shared.events).extend(events);
        }

        // Audio: everything this frame produced. Faster than the DS, the sound would pile up:
        // drop it (fast-forward is silent, as in BizHawk).
        let normal = speed.is_some_and(|s| s <= 100.0);
        let fill = {
            let mut ring = lock(&shared.audio);
            loop {
                let n = emu.nds.audio(&mut audio_buf);
                if n == 0 {
                    break;
                }
                if audio_rate.is_some() && normal {
                    ring.push(&audio_buf[..n * 2]);
                }
            }
            if !normal {
                ring.clear();
            }
            ring.len()
        };

        if let Some(pct) = speed {
            let nudge = match audio_rate {
                // Aim for ~3 frames of sound queued (about 50 ms).
                Some(rate) if normal => audio_nudge(fill / 2, (rate / DS_FPS * 3.0) as usize),
                _ => 1.0,
            };
            pacer.wait(DS_FPS * pct / 100.0 * nudge);
        }

        fps_window.1 += 1;
        let elapsed = fps_window.0.elapsed();
        if elapsed >= Duration::from_secs(1) {
            *lock(&shared.fps) = f64::from(fps_window.1) / elapsed.as_secs_f64();
            fps_window = (Instant::now(), 0);
        }
    }
}

fn finish(mut emu: Emulator) -> Emulator {
    emu.flush();
    emu
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ring_caps_and_pads() {
        let mut r = AudioRing::new();
        r.cap = 8;
        r.push(&[1, 2, 3, 4, 5, 6]);
        r.push(&[7, 8, 9, 10]);
        assert_eq!(r.len(), 8);
        let mut out = [9i16; 10];
        assert_eq!(r.pop_into(&mut out), 8);
        assert_eq!(out, [3, 4, 5, 6, 7, 8, 9, 10, 0, 0]);
    }

    #[test]
    fn nudge_is_small_and_signed() {
        assert_eq!(audio_nudge(100, 100), 1.0);
        assert!(audio_nudge(0, 100) > 1.0);
        assert!(audio_nudge(1000, 100) < 1.0);
        assert!((audio_nudge(1000, 100) - 0.995).abs() < 1e-9);
    }
}
