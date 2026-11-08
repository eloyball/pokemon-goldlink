//! The update check (RFC §6 M5): once per start, on its own thread, the repository's releases
//! (`release.json`'s `repository`) are listed and the newest player release (`vX.Y[.Z]`; the
//! relay's `relay-v*` releases are skipped) is compared with this one. A failure (offline, rate
//! limited) says nothing: it's a courtesy, never in the way.

use std::sync::mpsc::{Receiver, channel};

/// "0.3" / "v0.3.1" → (0, 3, 1).
pub fn parse_version(text: &str) -> Option<(u32, u32, u32)> {
    let t = text.strip_prefix('v').unwrap_or(text);
    let mut parts = t.split('.');
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next().map_or(Some(0), |p| p.parse().ok())?;
    let patch = parts.next().map_or(Some(0), |p| p.parse().ok())?;
    parts.next().is_none().then_some((major, minor, patch))
}

/// A newer player release among `tags` than `current`: (tag, page).
pub fn newer(current: &str, releases: &[(String, String)]) -> Option<(String, String)> {
    let mine = parse_version(current)?;
    releases
        .iter()
        .filter(|(tag, _)| tag.starts_with('v'))
        .filter_map(|(tag, url)| Some((parse_version(tag)?, tag, url)))
        .filter(|(v, _, _)| *v > mine)
        .max_by_key(|(v, _, _)| *v)
        .map(|(_, t, u)| (t.clone(), u.clone()))
}

/// Starts the check. The answer: a message for the player, or None (up to date, or no answer).
pub fn check(repository: String, current: String) -> Receiver<Option<String>> {
    let (tx, rx) = channel();
    std::thread::spawn(move || {
        let found = (|| {
            if repository.split('/').count() != 2 {
                return None;
            }
            let url = format!("https://api.github.com/repos/{repository}/releases?per_page=30");
            let body = ureq::get(&url)
                .header("User-Agent", "GoldLink-update-check")
                .header("Accept", "application/vnd.github+json")
                .call()
                .ok()?
                .body_mut()
                .read_to_string()
                .ok()?;
            let list: Vec<serde_json::Value> = serde_json::from_str(&body).ok()?;
            let releases: Vec<(String, String)> = list
                .iter()
                .filter(|r| r["draft"] != true && r["prerelease"] != true)
                .filter_map(|r| {
                    Some((
                        r["tag_name"].as_str()?.to_owned(),
                        r["html_url"].as_str()?.to_owned(),
                    ))
                })
                .collect();
            newer(&current, &releases)
        })();
        tx.send(found.map(|(tag, url)| format!("GoldLink {tag} is out: {url}")))
            .ok();
    });
    rx
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_and_the_newest_player_release() {
        assert_eq!(parse_version("0.3"), Some((0, 3, 0)));
        assert_eq!(parse_version("v0.3.1"), Some((0, 3, 1)));
        assert_eq!(parse_version("relay-v0.3"), None);
        assert_eq!(parse_version("v1.2.3.4"), None);
        let list = |tags: &[&str]| {
            tags.iter()
                .map(|t| (t.to_string(), format!("https://x/{t}")))
                .collect::<Vec<_>>()
        };
        assert_eq!(newer("0.3", &list(&["v0.3", "relay-v0.4", "v0.1"])), None);
        assert_eq!(
            newer("0.3", &list(&["v0.3.1", "v0.5", "relay-v0.9", "v0.4"])),
            Some(("v0.5".into(), "https://x/v0.5".into()))
        );
    }
}

#[cfg(test)]
mod live {
    /// Asks GitHub for real (ignored: it needs the network).
    #[test]
    #[ignore = "needs the network"]
    fn finds_a_newer_release_than_0_1() {
        let rx = super::check("eloyball/pokemon-goldlink".into(), "0.1".into());
        let note = rx.recv_timeout(std::time::Duration::from_secs(20)).unwrap();
        assert!(
            note.as_deref().is_some_and(|n| n.starts_with("GoldLink v")),
            "{note:?}"
        );
    }
}
