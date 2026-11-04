//! Small filesystem helpers shared across the crate.

use std::path::{Path, PathBuf};

use crate::error::{Error, Result};

/// Canonicalize `path` when it exists; otherwise return it unchanged.
pub fn resolve(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

/// Expand a leading `~` to the user's home directory.
pub fn expanduser(path: &str) -> PathBuf {
    if let Some(rest) = path.strip_prefix('~')
        && (rest.is_empty() || rest.starts_with('/'))
        && let Some(home) = dirs::home_dir()
    {
        return home.join(rest.trim_start_matches('/'));
    }
    PathBuf::from(path)
}

/// Copy `src` to `dst`, creating the parent directories of `dst`.
pub fn copy_file(src: &Path, dst: &Path) -> Result<()> {
    if let Some(parent) = dst.parent() {
        std::fs::create_dir_all(parent).map_err(|e| Error::io(parent, e))?;
    }
    std::fs::copy(src, dst).map_err(|e| Error::Other(format!("copy {} -> {}: {e}", src.display(), dst.display())))?;
    Ok(())
}

/// Remove everything inside `dir` except entries named in `keep`.
pub fn clear_dir(dir: &Path, keep: &[&str]) -> Result<()> {
    for entry in std::fs::read_dir(dir).map_err(|e| Error::io(dir, e))? {
        let entry = entry.map_err(|e| Error::io(dir, e))?;
        if keep.iter().any(|name| entry.file_name() == *name) {
            continue;
        }
        let path = entry.path();
        if path.is_dir() && !path.is_symlink() {
            std::fs::remove_dir_all(&path).map_err(|e| Error::io(&path, e))?;
        } else {
            std::fs::remove_file(&path).map_err(|e| Error::io(&path, e))?;
        }
    }
    Ok(())
}

/// Recreate `dir` empty (removing any previous contents).
pub fn recreate_dir(dir: &Path) -> Result<()> {
    if dir.exists() {
        std::fs::remove_dir_all(dir).map_err(|e| Error::io(dir, e))?;
    }
    std::fs::create_dir_all(dir).map_err(|e| Error::io(dir, e))?;
    Ok(())
}

/// Sorted directory entries (by file name) so output is deterministic.
pub fn sorted_entries(dir: &Path) -> Result<Vec<PathBuf>> {
    let mut entries: Vec<PathBuf> =
        std::fs::read_dir(dir).map_err(|e| Error::io(dir, e))?.filter_map(|e| e.ok()).map(|e| e.path()).collect();
    entries.sort();
    Ok(entries)
}

/// The path as a `/`-separated string, regardless of platform.
pub fn posix(path: &Path) -> String {
    path.components().map(|c| c.as_os_str().to_string_lossy().into_owned()).collect::<Vec<_>>().join("/")
}

/// True when the directory has entries other than `.DS_Store`.
pub fn has_files_except_ds_store(dir: &Path) -> Result<bool> {
    for entry in std::fs::read_dir(dir).map_err(|e| Error::io(dir, e))? {
        let entry = entry.map_err(|e| Error::io(dir, e))?;
        if entry.file_name() != ".DS_Store" {
            return Ok(true);
        }
    }
    Ok(false)
}

/// Short, stable hash of a path used to name per-project scratch directories.
pub fn path_hash(path: &Path) -> String {
    use md5::{Digest, Md5};
    let digest = Md5::digest(path.display().to_string().as_bytes());
    let hex: String = digest.iter().map(|b| format!("{b:02x}")).collect();
    hex[..8].to_string()
}
