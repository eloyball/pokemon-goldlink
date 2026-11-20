//! The window's GPU device, watched (KNOWN_RISKS #138). wgpu's default for an error nothing
//! catches is a panic, and it loses the device for any failed Direct3D call (not only a GPU
//! reset): a laptop's graphics switching, a driver hiccup while minimized, and GoldLink was gone.
//! Here errors are logged instead, and a lost device is flagged: the shell then opens the window
//! again on a new device, the game running on meanwhile (`shell::run`).

use std::sync::atomic::{AtomicBool, Ordering};

use eframe::egui_wgpu;

pub(crate) static LOST: AtomicBool = AtomicBool::new(false);

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
    #[cfg(feature = "dev-hooks")]
    crate::dev_hooks::window_watched();
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
