//! Crash reports (RFC §6 M5: diagnostics). A panic anywhere writes `crashes/crash-<time>.json` in
//! the data folder: the version and build, the system, the message, where, and a backtrace. The
//! launcher says so on the next start, once, so the player can pass the file on. No network: the
//! report stays on the player's computer. It ends with the log's last lines (`logfile.rs`).
//!
//! A crash inside melonDS's C++ (a segfault) ends the process without a report.

use std::path::{Path, PathBuf};

pub const DIR: &str = "crashes";
const SEEN: &str = ".seen";

/// Installs the panic hook (keeps the default one too, which prints to the terminal).
pub fn install(data: PathBuf, version: &'static str) {
    let default = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let message = info
            .payload()
            .downcast_ref::<&str>()
            .map(|s| s.to_string())
            .or_else(|| info.payload().downcast_ref::<String>().cloned())
            .unwrap_or_else(|| "(no message)".into());
        if NONFATAL.with(|n| n.get()) {
            // Caught where it happened (the lighting's thread): a line in the log, no report.
            log::error!(
                "{} panicked (caught): {message} at {}",
                std::thread::current().name().unwrap_or("?"),
                info.location()
                    .map_or_else(String::new, |l| format!("{}:{}", l.file(), l.line()))
            );
            return;
        }
        let report = serde_json::json!({
            "goldlink": version,
            "build": option_env!("GOLDLINK_BUILD").unwrap_or("dev"),
            "os": std::env::consts::OS,
            "arch": std::env::consts::ARCH,
            "time": chrono::Utc::now().to_rfc3339(),
            "thread": std::thread::current().name().unwrap_or("?"),
            "message": message,
            "location": info.location().map(|l| format!("{}:{}", l.file(), l.line())),
            "backtrace": std::backtrace::Backtrace::force_capture().to_string(),
            "log": crate::logfile::tail(&data, 40),
        });
        let name = format!(
            "crash-{}.json",
            chrono::Local::now().format("%Y%m%d-%H%M%S")
        );
        let path = data.join(DIR).join(name);
        if let Ok(text) = serde_json::to_string_pretty(&report) {
            goldlink_runtime::write_atomic(&path, text.as_bytes()).ok();
        }
        default(info);
    }));
}

thread_local! {
    static NONFATAL: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Runs `f` on this thread with its panics caught: logged, with no crash report (for work that
/// can stop without the game, like the lighting). None when it panicked.
pub fn caught<T>(f: impl FnOnce() -> T) -> Option<T> {
    NONFATAL.with(|n| n.set(true));
    let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)).ok();
    NONFATAL.with(|n| n.set(false));
    r
}

/// The reports the player hasn't been told about yet, newest first; marks them told.
pub fn take_new(data: &Path) -> Vec<PathBuf> {
    let dir = data.join(DIR);
    let seen = std::fs::metadata(dir.join(SEEN))
        .and_then(|m| m.modified())
        .ok();
    let mut new: Vec<(std::time::SystemTime, PathBuf)> = std::fs::read_dir(&dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.path().extension().is_some_and(|x| x == "json"))
        .filter_map(|e| Some((e.metadata().ok()?.modified().ok()?, e.path())))
        .filter(|(t, _)| seen.is_none_or(|s| *t > s))
        .collect();
    new.sort_by_key(|e| std::cmp::Reverse(e.0));
    if !new.is_empty() {
        goldlink_runtime::write_atomic(&dir.join(SEEN), b"").ok();
    }
    new.into_iter().map(|(_, p)| p).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reports_are_told_once() {
        let data = std::env::temp_dir().join(format!("goldlink-crash-{}", std::process::id()));
        std::fs::remove_dir_all(&data).ok();
        std::fs::create_dir_all(data.join(DIR)).unwrap();
        assert!(take_new(&data).is_empty());
        std::fs::write(data.join(DIR).join("crash-1.json"), "{}").unwrap();
        assert_eq!(take_new(&data).len(), 1);
        assert!(take_new(&data).is_empty(), "told once");
        std::fs::remove_dir_all(&data).ok();
    }
}
