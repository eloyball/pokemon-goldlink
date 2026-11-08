//! Sound out through the system's default output device (cpal: WASAPI, ALSA/PipeWire,
//! CoreAudio). The core resamples to the device's rate, so this only copies samples from the
//! emulation thread's ring and applies the volume.

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

use crate::runner::{Shared, lock};

/// Keeps the sound playing while it lives. Not Send on every platform: keep it on the UI thread.
pub struct AudioOut {
    _stream: cpal::Stream,
    pub rate: f64,
    /// 0..=100; 0 is silent.
    volume: Arc<AtomicU32>,
}

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
        let err = |e| eprintln!("goldlink: sound: {e}");
        let stream = match supported.sample_format() {
            cpal::SampleFormat::F32 => {
                let (s, v) = (shared.clone(), vol.clone());
                let mut tmp = Vec::new();
                device.build_output_stream(
                    &config,
                    move |out: &mut [f32], _| {
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
                let (s, v) = (shared.clone(), vol.clone());
                let mut tmp = Vec::new();
                device.build_output_stream(
                    &config,
                    move |out: &mut [i16], _| {
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
        })
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
