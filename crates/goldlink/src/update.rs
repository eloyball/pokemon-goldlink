//! Updates (RFC §6 M5; dev/docs/unified-client/UC10_UPDATER.md). Once per start, on its own
//! thread, the repository's releases (`release.json`'s `repository`) are listed and the newest
//! player release (`vX.Y[.Z]`; the relay's `relay-v*` releases are skipped) is compared with this
//! one. A failure (offline, rate limited) says nothing: it's a courtesy, never in the way.
//!
//! A newer release carrying this platform's download, `SHA256SUMS` and `SHA256SUMS.sig` can be
//! installed from the launcher: downloaded, its signature checked against [`RELEASE_KEY`] and its
//! checksum against the signed sums, unpacked, then put in place in the program's folder (each
//! old file renamed `*.goldlink-old` first: Windows can't overwrite a running program but can
//! rename it), and the new program started. The player's data folder is never touched.

use std::io::Read;
use std::path::{Component, Path, PathBuf};
use std::sync::mpsc::{Receiver, Sender, channel};

use sha2::{Digest, Sha256};

/// The maintainer's release key (Ed25519; `goldlink-sign public`): only releases it signed are
/// installed.
pub const RELEASE_KEY: [u8; 32] = [
    0x58, 0xd6, 0x9d, 0x7b, 0xd3, 0x31, 0x21, 0x3f, 0xcd, 0xf4, 0xbf, 0x8d, 0x1c, 0x33, 0xf6, 0xe2,
    0xc8, 0xa0, 0x39, 0x89, 0x6f, 0x35, 0xd0, 0x06, 0xa2, 0x00, 0x1b, 0xb2, 0x5f, 0x10, 0x8c, 0x89,
];

/// Where releases are listed; `GOLDLINK_UPDATE_API` points it elsewhere (the tests' local
/// server). The signature check is the same either way.
const API: &str = "https://api.github.com";

/// The most an update's archive may be.
const MAX_DOWNLOAD: u64 = 256 << 20;
/// Leftovers of the last update: old files, and an unfinished one's staging folder.
const OLD_SUFFIX: &str = ".goldlink-old";
const STAGING: &str = ".goldlink-update";

/// "0.3" / "v0.3.1" → (0, 3, 1).
pub fn parse_version(text: &str) -> Option<(u32, u32, u32)> {
    let t = text.strip_prefix('v').unwrap_or(text);
    let mut parts = t.split('.');
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next().map_or(Some(0), |p| p.parse().ok())?;
    let patch = parts.next().map_or(Some(0), |p| p.parse().ok())?;
    parts.next().is_none().then_some((major, minor, patch))
}

/// A newer player release: its tag, its page, and its files (name, download address).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Available {
    pub tag: String,
    pub page: String,
    pub assets: Vec<(String, String)>,
}

impl Available {
    fn asset(&self, name: &str) -> Option<&str> {
        self.assets
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, u)| u.as_str())
    }

    /// This platform's download, when this platform has one.
    pub fn archive_name(&self) -> Option<String> {
        let os = if cfg!(windows) {
            "windows.zip"
        } else if cfg!(target_os = "linux") {
            "linux.tar.gz"
        } else {
            return None;
        };
        Some(format!("GoldLink-{}-{os}", self.tag))
    }

    /// Whether the launcher can install it (else it offers the release page).
    pub fn installable(&self) -> bool {
        self.archive_name()
            .is_some_and(|a| self.asset(&a).is_some())
            && self.asset("SHA256SUMS").is_some()
            && self.asset("SHA256SUMS.sig").is_some()
    }
}

/// The newest player release in a releases list (GitHub's JSON) newer than `current`.
pub fn newer(current: &str, list: &[serde_json::Value]) -> Option<Available> {
    let mine = parse_version(current)?;
    list.iter()
        .filter(|r| r["draft"] != true && r["prerelease"] != true)
        .filter_map(|r| {
            let tag = r["tag_name"].as_str()?;
            let v = parse_version(tag).filter(|_| tag.starts_with('v'))?;
            Some((v, r))
        })
        .filter(|(v, _)| *v > mine)
        .max_by_key(|(v, _)| *v)
        .and_then(|(_, r)| {
            Some(Available {
                tag: r["tag_name"].as_str()?.to_owned(),
                page: r["html_url"].as_str()?.to_owned(),
                assets: r["assets"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|a| {
                        Some((
                            a["name"].as_str()?.to_owned(),
                            a["browser_download_url"].as_str()?.to_owned(),
                        ))
                    })
                    .collect(),
            })
        })
}

fn api() -> String {
    std::env::var("GOLDLINK_UPDATE_API").unwrap_or_else(|_| API.to_owned())
}

/// Asks for the releases now (blocking): a newer one, or None (up to date, or no answer).
pub fn find(repository: &str, current: &str) -> Option<Available> {
    find_at(&api(), repository, current)
}

/// [`find`] at another address than GitHub's API (the tests' local server).
pub fn find_at(api: &str, repository: &str, current: &str) -> Option<Available> {
    if repository.split('/').count() != 2 {
        return None;
    }
    let url = format!("{api}/repos/{repository}/releases?per_page=30");
    let body = ureq::get(&url)
        .header("User-Agent", "GoldLink-update-check")
        .header("Accept", "application/vnd.github+json")
        .call()
        .ok()?
        .body_mut()
        .read_to_string()
        .ok()?;
    let list: Vec<serde_json::Value> = serde_json::from_str(&body).ok()?;
    newer(current, &list)
}

/// Starts the check on its own thread.
pub fn check(repository: String, current: String) -> Receiver<Option<Available>> {
    let (tx, rx) = channel();
    std::thread::spawn(move || {
        tx.send(find(&repository, &current)).ok();
    });
    rx
}

/// Where an update is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Stage {
    /// Bytes so far, and of how many (0: not said).
    Downloading(u64, u64),
    Checking,
    Installing,
    /// Installed: the new program to start.
    Done(PathBuf),
    /// Nothing was changed (or everything was put back): why.
    Failed(String),
}

/// The program's own file name in its folder.
pub fn program_name() -> &'static str {
    if cfg!(windows) {
        "GoldLink.exe"
    } else {
        "GoldLink"
    }
}

/// Installs `update` into the program folder `dir` on its own thread, checked against `key`.
pub fn start(update: Available, dir: PathBuf, key: [u8; 32]) -> Receiver<Stage> {
    let (tx, rx) = channel();
    std::thread::spawn(move || {
        let end = match install_release(&update, &dir, &key, &tx) {
            Ok(program) => Stage::Done(program),
            Err(e) => Stage::Failed(e),
        };
        tx.send(end).ok();
    });
    rx
}

/// Downloads, checks, unpacks and installs `update` into `dir`. Returns the new program.
pub fn install_release(
    update: &Available,
    dir: &Path,
    key: &[u8; 32],
    progress: &Sender<Stage>,
) -> Result<PathBuf, String> {
    if !update.installable() {
        return Err("this release can't be installed from here".into());
    }
    let archive_name = update.archive_name().unwrap_or_default();
    let sums = download(
        update.asset("SHA256SUMS").unwrap_or_default(),
        1 << 16,
        |_, _| {},
    )?;
    let sig = download(
        update.asset("SHA256SUMS.sig").unwrap_or_default(),
        1 << 12,
        |_, _| {},
    )?;
    // Checked before the big download: a release that isn't signed by the key isn't fetched.
    let sig = String::from_utf8(sig).map_err(|_| "the release's signature isn't readable")?;
    goldlink_sign::verify(key, &update.tag, &sums, &sig)?;
    let want = sum_for(&String::from_utf8_lossy(&sums), &archive_name)
        .ok_or_else(|| format!("{archive_name} isn't in the release's checksums"))?;
    let archive = download(
        update.asset(&archive_name).unwrap_or_default(),
        MAX_DOWNLOAD,
        |done, total| {
            progress.send(Stage::Downloading(done, total)).ok();
        },
    )?;
    progress.send(Stage::Checking).ok();
    if Sha256::digest(&archive).as_slice() != want {
        return Err("the download is damaged (its checksum doesn't match): try again".into());
    }
    let files = unpack(&archive_name, &archive)?;
    if !files.iter().any(|f| f.path == Path::new(program_name())) {
        return Err(format!("the download has no {}", program_name()));
    }
    progress.send(Stage::Installing).ok();
    install(dir, &files)?;
    Ok(dir.join(program_name()))
}

fn download(url: &str, cap: u64, mut progress: impl FnMut(u64, u64)) -> Result<Vec<u8>, String> {
    let failed =
        |e: &dyn std::fmt::Display| format!("the download failed ({e}): check your connection");
    let mut response = ureq::get(url)
        .header("User-Agent", "GoldLink-updater")
        .call()
        .map_err(|e| failed(&e))?;
    let total = response
        .headers()
        .get("content-length")
        .and_then(|v| v.to_str().ok()?.parse().ok())
        .unwrap_or(0u64);
    if total > cap {
        return Err("the download is larger than an update can be".into());
    }
    let mut reader = response.body_mut().with_config().limit(cap).reader();
    let mut bytes = Vec::with_capacity(total as usize);
    let mut buf = vec![0u8; 1 << 16];
    loop {
        let n = reader.read(&mut buf).map_err(|e| failed(&e))?;
        if n == 0 {
            break;
        }
        bytes.extend_from_slice(&buf[..n]);
        progress(bytes.len() as u64, total);
    }
    Ok(bytes)
}

/// A file's SHA-256 in `SHA256SUMS` (`sha256sum`'s lines: `<hex>  <name>`).
pub fn sum_for(sums: &str, name: &str) -> Option<[u8; 32]> {
    sums.lines().find_map(|line| {
        let (hash, file) = line.split_once(char::is_whitespace)?;
        let file = file.trim_start().trim_start_matches('*');
        (file == name).then(|| hex::decode(hash).ok()?.try_into().ok())?
    })
}

/// A file of an update: where it goes in the program's folder, its bytes, whether it runs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct File {
    pub path: PathBuf,
    pub data: Vec<u8>,
    pub executable: bool,
}

/// The archive's files under its top folder (`GoldLink-<tag>/`), checked to stay in it.
pub fn unpack(name: &str, archive: &[u8]) -> Result<Vec<File>, String> {
    let bad = |e: &dyn std::fmt::Display| format!("the download can't be unpacked ({e})");
    let mut files = Vec::new();
    if name.ends_with(".zip") {
        let mut zip = zip::ZipArchive::new(std::io::Cursor::new(archive)).map_err(|e| bad(&e))?;
        for i in 0..zip.len() {
            let mut entry = zip.by_index(i).map_err(|e| bad(&e))?;
            if entry.is_dir() {
                continue;
            }
            let path = inside(Path::new(entry.name()))?;
            let mut data = Vec::new();
            entry.read_to_end(&mut data).map_err(|e| bad(&e))?;
            let executable = entry.unix_mode().is_some_and(|m| m & 0o111 != 0);
            files.push(File {
                path,
                data,
                executable,
            });
        }
    } else if name.ends_with(".tar.gz") {
        let mut tar = tar::Archive::new(flate2::read::GzDecoder::new(archive));
        for entry in tar.entries().map_err(|e| bad(&e))? {
            let mut entry = entry.map_err(|e| bad(&e))?;
            let kind = entry.header().entry_type();
            if kind.is_dir() {
                continue;
            }
            if !kind.is_file() {
                return Err(bad(&"a link or device in the archive"));
            }
            let path = inside(&entry.path().map_err(|e| bad(&e))?)?;
            let executable = entry.header().mode().is_ok_and(|m| m & 0o111 != 0);
            let mut data = Vec::new();
            entry.read_to_end(&mut data).map_err(|e| bad(&e))?;
            files.push(File {
                path,
                data,
                executable,
            });
        }
    } else {
        return Err(bad(&"not a .zip or .tar.gz"));
    }
    Ok(files)
}

/// `GoldLink-v0.6.1/LICENSES/COPYING` → `LICENSES/COPYING`: the top folder dropped; refused when
/// it would leave the program's folder.
fn inside(path: &Path) -> Result<PathBuf, String> {
    let mut parts = Vec::new();
    for c in path.components() {
        match c {
            Component::Normal(p) => parts.push(p),
            Component::CurDir => {}
            _ => {
                return Err(format!(
                    "the download has a file outside its folder ({})",
                    path.display()
                ));
            }
        }
    }
    if parts.len() < 2 {
        return Err(format!(
            "the download has a file outside its folder ({})",
            path.display()
        ));
    }
    Ok(parts[1..].iter().collect())
}

/// Puts `files` in place in `dir`: all written to a staging folder first (nothing changes when
/// that fails), then each renamed in, an existing file renamed `*.goldlink-old` first. A failed
/// step puts back what was done.
pub fn install(dir: &Path, files: &[File]) -> Result<(), String> {
    let staging = dir.join(STAGING);
    let cant_write = |e: &dyn std::fmt::Display| {
        format!("GoldLink can't write to its folder ({e}): update by hand from the release page")
    };
    std::fs::remove_dir_all(&staging).ok();
    let staged = (|| -> std::io::Result<()> {
        for f in files {
            let to = staging.join(&f.path);
            if let Some(parent) = to.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::write(&to, &f.data)?;
            #[cfg(unix)]
            if f.executable {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(&to, std::fs::Permissions::from_mode(0o755))?;
            }
        }
        Ok(())
    })();
    if let Err(e) = staged {
        std::fs::remove_dir_all(&staging).ok();
        return Err(cant_write(&e));
    }
    // (old file moved aside, or None when there was none; the new file's place)
    let mut done: Vec<(Option<PathBuf>, PathBuf)> = Vec::new();
    let mut result = Ok(());
    for f in files {
        let target = dir.join(&f.path);
        let step = (|| -> std::io::Result<()> {
            if let Some(parent) = target.parent() {
                std::fs::create_dir_all(parent)?;
            }
            let old = if target.exists() {
                let old = old_name(&target);
                std::fs::remove_file(&old).ok();
                std::fs::rename(&target, &old)?;
                Some(old)
            } else {
                None
            };
            let renamed = std::fs::rename(staging.join(&f.path), &target);
            if let Err(e) = renamed {
                if let Some(old) = &old {
                    std::fs::rename(old, &target).ok();
                }
                return Err(e);
            }
            done.push((old, target.clone()));
            Ok(())
        })();
        if let Err(e) = step {
            result = Err(cant_write(&e));
            break;
        }
    }
    if result.is_err() {
        for (old, target) in done.into_iter().rev() {
            match old {
                Some(old) => {
                    std::fs::remove_file(&target).ok();
                    std::fs::rename(&old, &target).ok();
                }
                None => {
                    std::fs::remove_file(&target).ok();
                }
            }
        }
    }
    std::fs::remove_dir_all(&staging).ok();
    result
}

fn old_name(path: &Path) -> PathBuf {
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(OLD_SUFFIX);
    path.with_file_name(name)
}

/// Deletes the last update's leftovers in the program's folder: `*.goldlink-old` files (the old
/// program among them, no longer running) and an unfinished staging folder.
pub fn clean_up(dir: &Path) {
    std::fs::remove_dir_all(dir.join(STAGING)).ok();
    fn walk(dir: &Path, depth: u32) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for e in entries.flatten() {
            let path = e.path();
            let Ok(kind) = e.file_type() else { continue };
            if kind.is_dir() && depth < 4 {
                walk(&path, depth + 1);
            } else if kind.is_file()
                && path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|n| n.ends_with(OLD_SUFFIX))
            {
                std::fs::remove_file(&path).ok();
            }
        }
    }
    walk(dir, 0);
}

/// Starts the new program (on the launcher); the caller then closes this one.
pub fn restart(program: &Path) -> Result<(), String> {
    std::process::Command::new(program)
        .current_dir(program.parent().unwrap_or(Path::new(".")))
        .spawn()
        .map(|_| ())
        .map_err(|e| {
            format!("the new version is installed but didn't start ({e}): start GoldLink again")
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn release(tag: &str, assets: &[&str]) -> serde_json::Value {
        serde_json::json!({
            "tag_name": tag,
            "html_url": format!("https://x/{tag}"),
            "draft": false,
            "prerelease": false,
            "assets": assets.iter().map(|a| serde_json::json!({
                "name": a.replace("TAG", tag),
                "browser_download_url": format!("https://x/{tag}/{}", a.replace("TAG", tag)),
            })).collect::<Vec<_>>(),
        })
    }

    #[test]
    fn versions_and_the_newest_player_release() {
        assert_eq!(parse_version("0.3"), Some((0, 3, 0)));
        assert_eq!(parse_version("v0.3.1"), Some((0, 3, 1)));
        assert_eq!(parse_version("relay-v0.3"), None);
        assert_eq!(parse_version("v1.2.3.4"), None);
        let list = |tags: &[&str]| tags.iter().map(|t| release(t, &[])).collect::<Vec<_>>();
        assert_eq!(newer("0.3", &list(&["v0.3", "relay-v0.4", "v0.1"])), None);
        let found = newer("0.3", &list(&["v0.3.1", "v0.5", "relay-v0.9", "v0.4"])).unwrap();
        assert_eq!(
            (found.tag.as_str(), found.page.as_str()),
            ("v0.5", "https://x/v0.5")
        );
        let mut draft = release("v0.9", &[]);
        draft["draft"] = true.into();
        assert_eq!(newer("0.5", &[draft]), None);
    }

    #[test]
    fn installable_only_with_its_download_sums_and_signature() {
        let full = [
            "GoldLink-TAG-windows.zip",
            "GoldLink-TAG-linux.tar.gz",
            "SHA256SUMS",
            "SHA256SUMS.sig",
        ];
        let a = newer("0.6", &[release("v0.6.1", &full)]).unwrap();
        assert_eq!(a.installable(), cfg!(any(windows, target_os = "linux")));
        // v0.6 and older: no signature, the page only.
        let a = newer("0.5", &[release("v0.6", &full[..3])]).unwrap();
        assert!(!a.installable());
    }

    #[test]
    fn sums_lines() {
        let h = "ab".repeat(32);
        let sums = format!(
            "{h}  GoldLink-v0.6.1-linux.tar.gz\n{}  GoldLink-v0.6.1-windows.zip\n",
            "cd".repeat(32)
        );
        assert_eq!(
            sum_for(&sums, "GoldLink-v0.6.1-linux.tar.gz"),
            Some([0xab; 32])
        );
        assert_eq!(
            sum_for(&sums, "GoldLink-v0.6.1-windows.zip"),
            Some([0xcd; 32])
        );
        assert_eq!(sum_for(&sums, "GoldLink-v0.6.1"), None);
    }

    fn tar_gz(entries: &[(&str, &[u8], u32)]) -> Vec<u8> {
        let mut b = tar::Builder::new(flate2::write::GzEncoder::new(
            Vec::new(),
            flate2::Compression::fast(),
        ));
        for (path, data, mode) in entries {
            let mut h = tar::Header::new_gnu();
            h.set_size(data.len() as u64);
            h.set_mode(*mode);
            h.set_entry_type(tar::EntryType::Regular);
            // set_path refuses "..": written raw, as a hostile archive would be.
            h.as_old_mut().name[..path.len()].copy_from_slice(path.as_bytes());
            h.set_cksum();
            b.append(&h, *data).unwrap();
        }
        b.into_inner().unwrap().finish().unwrap()
    }

    #[test]
    fn unpacks_under_the_top_folder_only() {
        let a = tar_gz(&[
            ("GoldLink-v0.6.1/GoldLink", b"prog", 0o755),
            ("GoldLink-v0.6.1/LICENSES/COPYING", b"gpl", 0o644),
        ]);
        let files = unpack("GoldLink-v0.6.1-linux.tar.gz", &a).unwrap();
        assert_eq!(files.len(), 2);
        assert_eq!(
            (files[0].path.as_path(), files[0].executable),
            (Path::new("GoldLink"), true)
        );
        assert_eq!(
            (files[1].path.as_path(), files[1].executable),
            (Path::new("LICENSES/COPYING"), false)
        );
        for hostile in ["GoldLink-v0.6.1/../../evil", "/etc/evil", "GoldLink"] {
            let a = tar_gz(&[(hostile, b"x", 0o644)]);
            assert!(unpack("x.tar.gz", &a).is_err(), "{hostile}");
        }
        // A zip, as the Windows download.
        let mut z = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        let o = zip::write::SimpleFileOptions::default();
        z.start_file("GoldLink-v0.6.1/GoldLink.exe", o).unwrap();
        std::io::Write::write_all(&mut z, b"exe").unwrap();
        let zip = z.finish().unwrap().into_inner();
        let files = unpack("GoldLink-v0.6.1-windows.zip", &zip).unwrap();
        assert_eq!(
            (files[0].path.as_path(), files[0].data.as_slice()),
            (Path::new("GoldLink.exe"), &b"exe"[..])
        );
    }

    fn scratch(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("goldlink-update-{name}-{}", std::process::id()));
        std::fs::remove_dir_all(&d).ok();
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn file(path: &str, data: &[u8]) -> File {
        File {
            path: path.into(),
            data: data.to_vec(),
            executable: false,
        }
    }

    #[test]
    fn installs_over_the_old_files_and_cleans_up_after() {
        let d = scratch("install");
        std::fs::write(d.join("GoldLink"), b"old").unwrap();
        std::fs::write(d.join("keep.txt"), b"mine").unwrap();
        install(
            &d,
            &[file("GoldLink", b"new"), file("LICENSES/COPYING", b"gpl")],
        )
        .unwrap();
        assert_eq!(std::fs::read(d.join("GoldLink")).unwrap(), b"new");
        assert_eq!(
            std::fs::read(d.join("GoldLink.goldlink-old")).unwrap(),
            b"old"
        );
        assert_eq!(std::fs::read(d.join("LICENSES/COPYING")).unwrap(), b"gpl");
        assert_eq!(std::fs::read(d.join("keep.txt")).unwrap(), b"mine");
        assert!(!d.join(STAGING).exists());
        clean_up(&d);
        assert!(!d.join("GoldLink.goldlink-old").exists());
        assert!(d.join("GoldLink").exists() && d.join("keep.txt").exists());
        std::fs::remove_dir_all(&d).ok();
    }

    #[test]
    fn a_failed_install_puts_everything_back() {
        let d = scratch("rollback");
        std::fs::write(d.join("GoldLink"), b"old").unwrap();
        // "sub/x" stages fine but can't go in place ("sub" is a file in the program's folder),
        // after GoldLink already went in: GoldLink is put back, the new LICENSES file taken out.
        std::fs::write(d.join("sub"), b"a file").unwrap();
        let r = install(
            &d,
            &[
                file("GoldLink", b"new"),
                file("LICENSES/COPYING", b"gpl"),
                file("sub/x", b"y"),
            ],
        );
        assert!(r.is_err());
        assert_eq!(std::fs::read(d.join("GoldLink")).unwrap(), b"old");
        assert!(!d.join("LICENSES/COPYING").exists());
        assert_eq!(std::fs::read(d.join("sub")).unwrap(), b"a file");
        assert!(!d.join("GoldLink.goldlink-old").exists());
        assert!(!d.join(STAGING).exists());
        std::fs::remove_dir_all(&d).ok();
    }
}

#[cfg(test)]
mod live {
    /// Asks GitHub for real (ignored: it needs the network).
    #[test]
    #[ignore = "needs the network"]
    fn finds_a_newer_release_than_0_1() {
        let found = super::find("eloyball/pokemon-goldlink", "0.1");
        assert!(found.is_some_and(|a| a.tag.starts_with('v')));
    }
}
