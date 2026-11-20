//! The window's GPU device, watched (KNOWN_RISKS #138). wgpu's default for an error nothing
//! catches is a panic, and it loses the device for any failed Direct3D call (not only a GPU
//! reset): a laptop's graphics switching, a driver hiccup while minimized, and GoldLink was gone.
//! Here errors are logged instead, and a lost device is flagged: the shell then opens the window
//! again on a new device, the game running on meanwhile (`shell::run`).

use std::sync::atomic::{AtomicBool, Ordering};

use eframe::egui_wgpu;

static LOST: AtomicBool = AtomicBool::new(false);

/// The window's device was lost since [`clear`].
pub fn lost() -> bool {
    LOST.load(Ordering::Relaxed)
}

pub fn clear() {
    LOST.store(false, Ordering::Relaxed);
}

/// Marks a render state as watched (in its renderer's resources, so a new window's is watched
/// again).
struct Watched;

/// Watches the window's device, once per device.
pub fn watch(rs: &egui_wgpu::RenderState) {
    {
        let mut r = rs.renderer.write();
        if r.callback_resources.contains::<Watched>() {
            return;
        }
        r.callback_resources.insert(Watched);
    }
    let info = rs.adapter.get_info();
    log::info!(
        "the window draws on {} ({:?}, driver {} {})",
        info.name,
        info.backend,
        info.driver,
        info.driver_info
    );
    #[cfg(feature = "debug-gpu")]
    debug::watched();
    rs.device.on_uncaptured_error(Box::new(|e| {
        let text = e.to_string();
        log::error!("the window's GPU: {text}");
        if text.contains("device is lost") || matches!(e, wgpu::Error::OutOfMemory { .. }) {
            LOST.store(true, Ordering::Relaxed);
        }
    }));
    rs.device.set_device_lost_callback(|reason, message| {
        if reason == wgpu::DeviceLostReason::Destroyed {
            return; // the window closing
        }
        log::error!("the window's GPU device was lost ({reason:?}): {message}");
        LOST.store(true, Ordering::Relaxed);
    });
}

/// With the `debug-gpu` feature only (docs/DEBUG_TOOLS.md; never in a release): `GOLDLINK_DEBUG_LOSE_GPU=<seconds>[,<times>]` loses
/// the window's device that long after each window opens (`times` windows, 1 by default): first
/// an error nothing catches (logged, not a panic), then the loss as wgpu reports it.
#[cfg(feature = "debug-gpu")]
pub mod debug {
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::time::{Duration, Instant};

    use eframe::egui_wgpu;

    static OPENED: Mutex<Option<Instant>> = Mutex::new(None);
    static LOST_SO_FAR: AtomicU32 = AtomicU32::new(0);

    pub(super) fn watched() {
        *OPENED.lock().unwrap() = Some(Instant::now());
    }

    pub fn tick(rs: &egui_wgpu::RenderState) {
        let Some(spec) = std::env::var("GOLDLINK_DEBUG_LOSE_GPU").ok() else {
            return;
        };
        let mut parts = spec.split(',');
        let secs: f32 = parts.next().and_then(|s| s.parse().ok()).unwrap_or(10.0);
        let times: u32 = parts.next().and_then(|s| s.parse().ok()).unwrap_or(1);
        if LOST_SO_FAR.load(Ordering::Relaxed) >= times {
            return;
        }
        let due = OPENED
            .lock()
            .unwrap()
            .is_some_and(|t| t.elapsed() >= Duration::from_secs_f32(secs));
        if !due {
            return;
        }
        *OPENED.lock().unwrap() = None;
        LOST_SO_FAR.fetch_add(1, Ordering::Relaxed);
        log::warn!("debug: losing the window's GPU device");
        // A validation error with no scope: the handler's, not a panic.
        let _ = rs.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("debug: invalid"),
            size: 4,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::STORAGE,
            mapped_at_creation: false,
        });
        super::LOST.store(true, Ordering::Relaxed);
    }
}
