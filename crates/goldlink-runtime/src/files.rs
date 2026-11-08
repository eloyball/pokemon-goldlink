//! Small file helpers shared by saves, backups and settings.

use std::fs;
use std::path::Path;

/// Writes through a temporary file and a rename, so an interrupted write never leaves half a file.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let dir = path.parent().ok_or("bad path")?;
    fs::create_dir_all(dir).map_err(|e| format!("can't create {}: {e}", dir.display()))?;
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(".tmp");
    let tmp = std::path::PathBuf::from(tmp);
    fs::write(&tmp, bytes).map_err(|e| format!("can't write {}: {e}", tmp.display()))?;
    fs::rename(&tmp, path).map_err(|e| format!("can't write {}: {e}", path.display()))
}
