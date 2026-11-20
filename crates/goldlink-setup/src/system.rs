//! Where GoldLink goes, and how the desktop learns of it, and taking it away again. Windows: the
//! Start menu's and the desktop's shortcuts, the apps list's entry (Settings → Apps, with an
//! uninstaller). Linux: the app menu's entry (the one scripts/release/client/install.sh writes,
//! with an "Uninstall GoldLink" action) and a launcher on the desktop.

use std::path::{Path, PathBuf};

/// The uninstaller: this program, copied into the program's folder.
pub const UNINSTALLER: &str = if cfg!(windows) {
    "GoldLink-Uninstall.exe"
} else {
    "goldlink-uninstall"
};

/// The program's folder unless the player picks another (`GOLDLINK_SETUP_DIR` for the tests):
/// `%LOCALAPPDATA%\Programs\GoldLink`, where programs installed for one user go; on Linux
/// `~/.local/opt/GoldLink`. No administrator (root) is needed, and the launcher's updater can
/// write there. (The player's data is `%LOCALAPPDATA%\GoldLink`, `~/.local/share/goldlink`,
/// apart from it: uninstalling never touches it.)
pub fn default_dir() -> PathBuf {
    if let Some(d) = std::env::var_os("GOLDLINK_SETUP_DIR") {
        return PathBuf::from(d);
    }
    if cfg!(windows) {
        let base = dirs::data_local_dir().unwrap_or_else(std::env::temp_dir);
        base.join("Programs").join("GoldLink")
    } else {
        let home = dirs::home_dir().unwrap_or_else(std::env::temp_dir);
        home.join(".local").join("opt").join("GoldLink")
    }
}

/// The player's data folder (crates/goldlink `data_dir`), named on the pages.
pub fn data_dir() -> PathBuf {
    let base = dirs::data_local_dir().unwrap_or_default();
    base.join(if cfg!(windows) {
        "GoldLink"
    } else {
        "goldlink"
    })
}

/// A folder the player picked: GoldLink gets a folder of its own in it, so uninstalling (which
/// removes the program's folder) can't take anything else with it.
pub fn own_folder(picked: &Path) -> PathBuf {
    if picked
        .file_name()
        .is_some_and(|n| n.to_string_lossy().eq_ignore_ascii_case("GoldLink"))
    {
        picked.to_path_buf()
    } else {
        picked.join("GoldLink")
    }
}

/// The version installed in `dir` (its release.json), if GoldLink is there.
pub fn installed_version(dir: &Path) -> Option<String> {
    let text = std::fs::read_to_string(dir.join("release.json")).ok()?;
    let json: serde_json::Value = serde_json::from_str(&text).ok()?;
    json["version"].as_str().map(str::to_owned)
}

/// Whether `dir` holds GoldLink (its program and its release.json): what the uninstaller
/// checks before removing a folder.
pub fn holds_goldlink(dir: &Path) -> bool {
    dir.join(goldlink_update::program_name()).is_file() && dir.join("release.json").is_file()
}

#[cfg(windows)]
pub use windows_impl::{installed_dir, integrate, uninstall};

#[cfg(not(windows))]
pub use linux_impl::{installed_dir, integrate, uninstall};

#[cfg(not(windows))]
mod linux_impl {
    use std::os::unix::fs::PermissionsExt;
    use std::path::{Path, PathBuf};

    use super::UNINSTALLER;

    /// The app menu's entry: `$XDG_DATA_HOME/applications/goldlink.desktop`, as install.sh's.
    fn entry() -> Option<PathBuf> {
        Some(
            dirs::data_dir()?
                .join("applications")
                .join("goldlink.desktop"),
        )
    }

    fn desktop() -> Option<PathBuf> {
        Some(dirs::desktop_dir()?.join("goldlink.desktop"))
    }

    fn write(path: &Path, text: &str, executable: bool) -> Result<(), String> {
        let fail = |e: std::io::Error| format!("Couldn't write {}: {e}", path.display());
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(fail)?;
        }
        let tmp = path.with_extension("desktop.tmp");
        std::fs::write(&tmp, text).map_err(fail)?;
        if executable {
            std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o755)).map_err(fail)?;
        }
        std::fs::rename(&tmp, path).map_err(fail)
    }

    /// Where the app menu's entry says GoldLink is (an earlier install), if it does.
    pub fn installed_dir() -> Option<PathBuf> {
        let text = std::fs::read_to_string(entry()?).ok()?;
        let dir = PathBuf::from(text.lines().find_map(|l| l.strip_prefix("Path="))?);
        super::holds_goldlink(&dir).then_some(dir)
    }

    /// After the files: the uninstaller, the app menu's entry and the desktop's launcher.
    pub fn integrate(
        dir: &Path,
        _version: &str,
        _page: &str,
        desktop_too: bool,
    ) -> Result<(), String> {
        let shown = dir.display().to_string();
        // What a desktop entry's Exec can't carry (install.sh refuses them too).
        if shown.contains(['\\', '"', '`', '$', '\n']) {
            return Err(format!(
                "App menus can't start GoldLink from {shown} (a \\, \", ` or $ in it): pick another folder."
            ));
        }
        let program = dir.join(goldlink_update::program_name());
        let uninstaller = dir.join(UNINSTALLER);
        let me =
            std::env::current_exe().map_err(|e| format!("Couldn't find the installer: {e}"))?;
        if me != uninstaller {
            std::fs::copy(&me, &uninstaller)
                .and_then(|_| {
                    std::fs::set_permissions(&uninstaller, std::fs::Permissions::from_mode(0o755))
                })
                .map_err(|e| format!("Couldn't write {}: {e}", uninstaller.display()))?;
        }
        let text = format!(
            "[Desktop Entry]\nType=Application\nName=GoldLink\nComment=Six-player co-op Pokémon HeartGold\n\
             Exec=\"{program}\"\nPath={shown}\nIcon={icon}\nTerminal=false\nCategories=Game;\n\
             StartupWMClass=goldlink\nActions=Uninstall;\n\n\
             [Desktop Action Uninstall]\nName=Uninstall GoldLink\nExec=\"{uninstaller}\" --uninstall\n",
            program = program.display(),
            icon = dir.join("goldlink.png").display(),
            uninstaller = uninstaller.display(),
        );
        let apps = entry().ok_or("Can't find your app menu's folder")?;
        write(&apps, &text, false)?;
        if let Some(lnk) = desktop() {
            if desktop_too {
                write(&lnk, &text, true)?;
            } else {
                std::fs::remove_file(&lnk).ok();
            }
        }
        refresh_menu(&apps);
        Ok(())
    }

    fn refresh_menu(entry: &Path) {
        if let Some(apps) = entry.parent() {
            std::process::Command::new("update-desktop-database")
                .arg(apps)
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status()
                .ok();
        }
    }

    /// Takes GoldLink away: the program's folder (a running program can be deleted here), the
    /// app menu's entry and the desktop's launcher. The player's data folder is never touched.
    pub fn uninstall(dir: &Path) -> Result<(), String> {
        if !super::holds_goldlink(dir) {
            return Err(format!(
                "{} doesn't look like GoldLink's folder: nothing removed.",
                dir.display()
            ));
        }
        std::fs::remove_dir_all(dir)
            .map_err(|e| format!("Couldn't remove {}: {e}", dir.display()))?;
        for f in [entry(), desktop()].into_iter().flatten() {
            // Only ours: one that starts this folder's GoldLink.
            let ours = std::fs::read_to_string(&f)
                .is_ok_and(|t| t.contains(&format!("Path={}", dir.display())));
            if ours {
                std::fs::remove_file(&f).ok();
            }
        }
        if let Some(apps) = entry() {
            refresh_menu(&apps);
        }
        Ok(())
    }
}

#[cfg(windows)]
#[allow(unsafe_code)]
mod windows_impl {
    use std::path::{Path, PathBuf};

    use windows::Win32::Foundation::ERROR_SUCCESS;
    use windows::Win32::System::Com::{
        CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx,
        IPersistFile,
    };
    use windows::Win32::System::Registry::{
        HKEY, HKEY_CURRENT_USER, KEY_WRITE, REG_DWORD, REG_OPTION_NON_VOLATILE, REG_SZ,
        RRF_RT_REG_SZ, RegCloseKey, RegCreateKeyExW, RegDeleteTreeW, RegGetValueW, RegSetValueExW,
    };
    use windows::Win32::UI::Shell::{IShellLinkW, ShellLink};
    use windows::core::{HSTRING, Interface, PCWSTR};

    use super::UNINSTALLER;

    /// The apps list's entry (per user: no administrator).
    const KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Uninstall\GoldLink";

    fn start_menu() -> Option<PathBuf> {
        Some(
            dirs::data_dir()?
                .join("Microsoft")
                .join("Windows")
                .join("Start Menu")
                .join("Programs")
                .join("GoldLink.lnk"),
        )
    }

    fn desktop() -> Option<PathBuf> {
        Some(dirs::desktop_dir()?.join("GoldLink.lnk"))
    }

    /// A shortcut at `lnk` to `target`, started in `dir`, with the program's own icon.
    fn shortcut(lnk: &Path, target: &Path, dir: &Path) -> Result<(), String> {
        let fail =
            |e: windows::core::Error| format!("Couldn't make the shortcut {}: {e}", lnk.display());
        if let Some(parent) = lnk.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("Couldn't make {}: {e}", parent.display()))?;
        }
        // SAFETY: COM on this thread (the installer's worker), then one ShellLink object used
        // through its interfaces; the strings outlive the calls.
        unsafe {
            let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
            let link: IShellLinkW =
                CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER).map_err(fail)?;
            link.SetPath(&HSTRING::from(target.as_os_str()))
                .map_err(fail)?;
            link.SetWorkingDirectory(&HSTRING::from(dir.as_os_str()))
                .map_err(fail)?;
            link.SetIconLocation(&HSTRING::from(target.as_os_str()), 0)
                .map_err(fail)?;
            link.SetDescription(&HSTRING::from("Co-op Pokémon HeartGold"))
                .map_err(fail)?;
            let file: IPersistFile = link.cast().map_err(fail)?;
            file.Save(&HSTRING::from(lnk.as_os_str()), true)
                .map_err(fail)?;
        }
        Ok(())
    }

    enum Value {
        Text(String),
        Number(u32),
    }

    fn write_entry(values: &[(&str, Value)]) -> Result<(), String> {
        let mut key = HKEY::default();
        // SAFETY: the key is opened, written and closed here; every buffer outlives its call.
        unsafe {
            let made = RegCreateKeyExW(
                HKEY_CURRENT_USER,
                &HSTRING::from(KEY),
                0,
                PCWSTR::null(),
                REG_OPTION_NON_VOLATILE,
                KEY_WRITE,
                None,
                &mut key,
                None,
            );
            if made != ERROR_SUCCESS {
                return Err(format!(
                    "Couldn't add GoldLink to Windows' apps ({made:?})."
                ));
            }
            for (name, value) in values {
                let (kind, bytes) = match value {
                    Value::Text(s) => (
                        REG_SZ,
                        s.encode_utf16()
                            .chain(Some(0))
                            .flat_map(u16::to_le_bytes)
                            .collect::<Vec<u8>>(),
                    ),
                    Value::Number(n) => (REG_DWORD, n.to_le_bytes().to_vec()),
                };
                let set = RegSetValueExW(key, &HSTRING::from(*name), 0, kind, Some(&bytes));
                if set != ERROR_SUCCESS {
                    let _ = RegCloseKey(key);
                    return Err(format!(
                        "Couldn't add GoldLink to Windows' apps ({name}: {set:?})."
                    ));
                }
            }
            let _ = RegCloseKey(key);
        }
        Ok(())
    }

    /// Where the apps list says GoldLink is (an earlier install), if it does.
    pub fn installed_dir() -> Option<PathBuf> {
        let mut buf = [0u16; 1024];
        let mut len = (buf.len() * 2) as u32;
        // SAFETY: the buffer and its length are this function's.
        let read = unsafe {
            RegGetValueW(
                HKEY_CURRENT_USER,
                &HSTRING::from(KEY),
                &HSTRING::from("InstallLocation"),
                RRF_RT_REG_SZ,
                None,
                Some(buf.as_mut_ptr().cast()),
                Some(&mut len),
            )
        };
        if read != ERROR_SUCCESS {
            return None;
        }
        let n = (len as usize / 2).saturating_sub(1).min(buf.len());
        let dir = PathBuf::from(String::from_utf16_lossy(&buf[..n]));
        super::holds_goldlink(&dir).then_some(dir)
    }

    /// After the files: the uninstaller, the shortcuts and the apps list's entry.
    pub fn integrate(
        dir: &Path,
        version: &str,
        page: &str,
        desktop_too: bool,
    ) -> Result<(), String> {
        let program = dir.join(goldlink_update::program_name());
        let uninstaller = dir.join(UNINSTALLER);
        let me =
            std::env::current_exe().map_err(|e| format!("Couldn't find the installer: {e}"))?;
        if me != uninstaller {
            std::fs::copy(&me, &uninstaller)
                .map_err(|e| format!("Couldn't write {}: {e}", uninstaller.display()))?;
        }
        if let Some(lnk) = start_menu() {
            shortcut(&lnk, &program, dir)?;
        }
        if let Some(lnk) = desktop() {
            if desktop_too {
                shortcut(&lnk, &program, dir)?;
            } else {
                std::fs::remove_file(&lnk).ok();
            }
        }
        let size_kb = folder_size(dir) / 1024;
        let quoted = format!("\"{}\"", uninstaller.display());
        write_entry(&[
            ("DisplayName", Value::Text("GoldLink".into())),
            (
                "DisplayVersion",
                Value::Text(version.trim_start_matches('v').into()),
            ),
            ("Publisher", Value::Text("GoldLink".into())),
            ("DisplayIcon", Value::Text(program.display().to_string())),
            ("InstallLocation", Value::Text(dir.display().to_string())),
            (
                "UninstallString",
                Value::Text(format!("{quoted} --uninstall")),
            ),
            (
                "QuietUninstallString",
                Value::Text(format!("{quoted} --uninstall --quiet")),
            ),
            ("URLInfoAbout", Value::Text(page.into())),
            ("NoModify", Value::Number(1)),
            ("NoRepair", Value::Number(1)),
            (
                "EstimatedSize",
                Value::Number(u32::try_from(size_kb).unwrap_or(u32::MAX)),
            ),
        ])
    }

    fn folder_size(dir: &Path) -> u64 {
        std::fs::read_dir(dir)
            .into_iter()
            .flatten()
            .flatten()
            .map(|e| match e.file_type() {
                Ok(t) if t.is_dir() => folder_size(&e.path()),
                _ => e.metadata().map_or(0, |m| m.len()),
            })
            .sum()
    }

    /// Takes GoldLink away: the program's files (all but this uninstaller, which can't delete
    /// itself while it runs: a hidden `cmd` removes it and the folder once it has closed), the
    /// shortcuts and the apps list's entry. The player's data folder is never touched.
    pub fn uninstall(dir: &Path) -> Result<(), String> {
        if !super::holds_goldlink(dir) {
            return Err(format!(
                "{} doesn't look like GoldLink's folder: nothing removed.",
                dir.display()
            ));
        }
        // GoldLink itself first: running, it can't be removed, and nothing else is touched.
        std::fs::remove_file(dir.join(goldlink_update::program_name()))
            .map_err(|_| "GoldLink is still open: close it, then uninstall again.".to_owned())?;
        let me = std::env::current_exe().ok();
        if let Ok(entries) = std::fs::read_dir(dir) {
            for e in entries.flatten() {
                let path = e.path();
                if Some(&path) == me.as_ref() {
                    continue;
                }
                if path.is_dir() {
                    std::fs::remove_dir_all(&path).ok();
                } else {
                    std::fs::remove_file(&path).ok();
                }
            }
        }
        for lnk in [start_menu(), desktop()].into_iter().flatten() {
            std::fs::remove_file(lnk).ok();
        }
        // SAFETY: deletes this program's own key; the string outlives the call.
        unsafe {
            let _ = RegDeleteTreeW(HKEY_CURRENT_USER, &HSTRING::from(KEY));
        }
        // The uninstaller and the folder once this program has closed (the last page may stay
        // open a while): tried every second or two for about a minute.
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        // Quoted only with a space in it: Wine's cmd doesn't remove a quoted folder (the
        // installer's test runs there), Windows' needs the quotes only around spaces.
        let d = dir.display().to_string();
        let d = if d.contains(' ') {
            format!("\"{d}\"")
        } else {
            d
        };
        let script = format!(
            "for /l %i in (1,1,40) do @(ping -n 2 127.0.0.1 > nul & rmdir /s /q {d} 2> nul & if not exist {d} exit)"
        );
        std::process::Command::new("cmd.exe")
            .args(["/c", &script])
            .current_dir(std::env::temp_dir())
            .creation_flags(CREATE_NO_WINDOW)
            .spawn()
            .ok();
        Ok(())
    }
}
