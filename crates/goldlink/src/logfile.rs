//! GoldLink's log (KNOWN_RISKS #138): `goldlink.log` in the data folder, the run before kept as
//! `goldlink.old.log`. GoldLink's own lines (the `log` crate's info and up) and the libraries'
//! warnings and errors: wgpu names the graphics call that failed and its error code there, which
//! a crash report alone can't say. The crash report carries the log's last lines.
//!
//! The lines go to the terminal too, when there's one.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

pub const FILE: &str = "goldlink.log";
const OLD: &str = "goldlink.old.log";
/// A run's log stops growing here (a library repeating an error every frame).
const CAP: u64 = 8 << 20;

struct Logger {
    file: Mutex<Option<std::fs::File>>,
    written: AtomicU64,
}

static LOGGER: Logger = Logger {
    file: Mutex::new(None),
    written: AtomicU64::new(0),
};

/// The crates whose info lines are kept (the libraries: warnings and errors only).
fn ours(target: &str) -> bool {
    target.starts_with("goldlink")
}

impl log::Log for Logger {
    fn enabled(&self, m: &log::Metadata) -> bool {
        m.level() <= log::Level::Warn || (m.level() <= log::Level::Info && ours(m.target()))
    }

    fn log(&self, r: &log::Record) {
        if !self.enabled(r.metadata()) {
            return;
        }
        let line = format!(
            "{} {:<5} {}: {}\n",
            chrono::Local::now().format("%Y-%m-%d %H:%M:%S%.3f"),
            r.level(),
            r.target(),
            r.args()
        );
        let mut stderr = std::io::stderr();
        let _ = stderr.write_all(line.as_bytes());
        let mut f = self.file.lock().unwrap_or_else(|e| e.into_inner());
        let Some(file) = f.as_mut() else { return };
        let before = self.written.fetch_add(line.len() as u64, Ordering::Relaxed);
        if before < CAP {
            let _ = file.write_all(line.as_bytes());
            if before + line.len() as u64 >= CAP {
                let _ = file.write_all(b"(the log is full: later lines are left out)\n");
            }
        }
    }

    fn flush(&self) {
        if let Some(f) = self.file.lock().unwrap_or_else(|e| e.into_inner()).as_mut() {
            let _ = f.flush();
        }
    }
}

/// Starts the log in `data` (the run before becomes the old log). Without a folder (or when it
/// can't be written) the lines only go to the terminal.
pub fn install(data: Option<&Path>) {
    if let Some(data) = data {
        let path = data.join(FILE);
        let _ = std::fs::create_dir_all(data);
        let _ = std::fs::rename(&path, data.join(OLD));
        if let Ok(f) = std::fs::File::create(&path) {
            *LOGGER.file.lock().unwrap_or_else(|e| e.into_inner()) = Some(f);
        }
    }
    if log::set_logger(&LOGGER).is_ok() {
        log::set_max_level(log::LevelFilter::Info);
    }
}

pub fn path(data: &Path) -> PathBuf {
    data.join(FILE)
}

/// The log's last `n` lines (for a crash report).
pub fn tail(data: &Path, n: usize) -> Vec<String> {
    log::logger().flush();
    let Ok(text) = std::fs::read(path(data)) else {
        return Vec::new();
    };
    let text = String::from_utf8_lossy(&text);
    let lines: Vec<&str> = text.lines().collect();
    lines[lines.len().saturating_sub(n)..]
        .iter()
        .map(|l| l.to_string())
        .collect()
}

#[cfg(test)]
mod tests {
    #[test]
    fn libraries_only_warn() {
        use log::Log;
        let m = |level, target| log::Metadata::builder().level(level).target(target).build();
        let l = &super::LOGGER;
        assert!(l.enabled(&m(log::Level::Info, "goldlink::shell")));
        assert!(l.enabled(&m(log::Level::Info, "goldlink_light::gpu")));
        assert!(!l.enabled(&m(log::Level::Info, "wgpu_core::device")));
        assert!(l.enabled(&m(log::Level::Error, "wgpu_hal::auxil::dxgi::result")));
        assert!(!l.enabled(&m(log::Level::Debug, "goldlink")));
    }
}
