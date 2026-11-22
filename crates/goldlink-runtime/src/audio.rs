//! Sound out through the system's default output device (cpal: WASAPI, ALSA/PipeWire,
//! CoreAudio). The core resamples to the device's rate, so this only copies samples from the
//! emulation thread's ring and applies the volume.
//!
//! A stream can die under the game: ALSA's `poll()` returned POLLERR on the user's PipeWire
//! (the audio server reconfiguring the output), a Windows device switched or unplugged. The
//! sound then stayed off until the world was joined again (the user and their friends' v0.7.2
//! play test), and the error, logged as it repeated, filled the log. [`AudioOut::healthy`] says
//! when it died, so the window opens the device again; the error is logged once.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::time::{Duration, Instant};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

use crate::runner::{Shared, lock};

/// Keeps the sound playing while it lives. Not Send on every platform: keep it on the UI thread.
pub struct AudioOut {
    _stream: cpal::Stream,
    pub rate: f64,
    /// 0..=100; 0 is silent.
    volume: Arc<AtomicU32>,
    /// The stream reported an error, and when it last asked for sound (ms since `opened`).
    broken: Arc<AtomicBool>,
    last_call: Arc<AtomicU64>,
    opened: Instant,
}

/// How long a stream may go without asking for sound before it's taken for dead (the device
/// asks every few milliseconds while it plays).
const SILENT_FOR: Duration = Duration::from_secs(2);

impl AudioOut {
    /// Opens the default output device. Errors when there's none (the game runs silent).
    pub fn open(shared: Arc<Shared>, volume: u8) -> Result<Self, String> {
        let host = cpal::default_host();
        let device = host
            .default_output_device()
            .ok_or("no sound output device")?;
        let supported = device
            .default_output_config()
            .map_err(|e| format!("the sound device: {e}"))?;
        let rate = supported.sample_rate();
        let channels = supported.channels() as usize;
        let config = cpal::StreamConfig {
            channels: supported.channels(),
            sample_rate: rate,
            buffer_size: cpal::BufferSize::Default,
        };
        let vol = Arc::new(AtomicU32::new(u32::from(volume.min(100))));
        let broken = Arc::new(AtomicBool::new(false));
        let last_call = Arc::new(AtomicU64::new(0));
        let opened = Instant::now();
        let err = {
            let broken = broken.clone();
            move |e| {
                if !broken.swap(true, Ordering::Relaxed) {
                    log::warn!("sound: {e} (opening the sound device again)");
                }
            }
        };
        let stream = match supported.sample_format() {
            cpal::SampleFormat::F32 => {
                let (s, v, at) = (shared.clone(), vol.clone(), last_call.clone());
                let mut tmp = Vec::new();
                device.build_output_stream(
                    &config,
                    move |out: &mut [f32], _| {
                        at.store(opened.elapsed().as_millis() as u64, Ordering::Relaxed);
                        fill(&s, &v, channels, out.len(), &mut tmp);
                        for (o, x) in out.iter_mut().zip(&tmp) {
                            *o = *x as f32 / 32768.0;
                        }
                    },
                    err,
                    None,
                )
            }
            cpal::SampleFormat::I16 => {
                let (s, v, at) = (shared.clone(), vol.clone(), last_call.clone());
                let mut tmp = Vec::new();
                device.build_output_stream(
                    &config,
                    move |out: &mut [i16], _| {
                        at.store(opened.elapsed().as_millis() as u64, Ordering::Relaxed);
                        fill(&s, &v, channels, out.len(), &mut tmp);
                        out.copy_from_slice(&tmp[..out.len()]);
                    },
                    err,
                    None,
                )
            }
            other => return Err(format!("unsupported sound format {other:?}")),
        }
        .map_err(|e| format!("the sound device: {e}"))?;
        stream
            .play()
            .map_err(|e| format!("the sound device: {e}"))?;
        Ok(Self {
            _stream: stream,
            rate: f64::from(rate),
            volume: vol,
            broken,
            last_call,
            opened,
        })
    }

    /// Whether the stream still plays: no error from it, and it asked for sound lately (a
    /// stream may die without one).
    pub fn healthy(&self) -> bool {
        if self.broken.load(Ordering::Relaxed) {
            return false;
        }
        let since_open = self.opened.elapsed();
        let last = Duration::from_millis(self.last_call.load(Ordering::Relaxed));
        since_open < last + SILENT_FOR
    }

    pub fn set_volume(&self, volume: u8) {
        self.volume
            .store(u32::from(volume.min(100)), Ordering::Relaxed);
    }
}

/// Fills `tmp` with `len` samples for a device with `channels` channels: the ring's stereo
/// frames, at the volume, spread over the device's channels (mono: averaged).
fn fill(shared: &Shared, volume: &AtomicU32, channels: usize, len: usize, tmp: &mut Vec<i16>) {
    let frames = len / channels.max(1);
    let mut stereo = vec![0i16; frames * 2];
    lock(&shared.audio).pop_into(&mut stereo);
    let v = volume.load(Ordering::Relaxed) as i32;
    tmp.clear();
    tmp.resize(len, 0);
    for f in 0..frames {
        let l = i32::from(stereo[f * 2]) * v / 100;
        let r = i32::from(stereo[f * 2 + 1]) * v / 100;
        let out = &mut tmp[f * channels..(f + 1) * channels];
        match channels {
            1 => out[0] = ((l + r) / 2) as i16,
            _ => {
                out[0] = l as i16;
                out[1] = r as i16;
            }
        }
    }
}
