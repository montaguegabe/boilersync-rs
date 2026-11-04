//! Where boilersync looks for things: the template cache and the current
//! project root (the nearest directory with a `.boilersync` file).

use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::error::{Error, Result};
use crate::fsx;

/// Environment variable overriding the template cache directory.
pub const TEMPLATE_DIR_ENV: &str = "BOILERSYNC_TEMPLATE_DIR";
/// Environment variable overriding the project root lookup.
pub const ROOT_DIR_ENV: &str = "BOILERSYNC_ROOT_DIR";
/// The project metadata file name.
pub const METADATA_FILENAME: &str = ".boilersync";

/// Locations for one invocation. Built from the environment by the CLI and
/// constructed explicitly by tests and embedding applications.
#[derive(Debug, Clone)]
pub struct Paths {
    template_root_dir: PathBuf,
    cwd: PathBuf,
    root_override: Option<PathBuf>,
}

impl Paths {
    /// Explicit locations: the template cache root and the working directory
    /// the project root is searched from.
    pub fn new(template_root_dir: impl Into<PathBuf>, cwd: impl Into<PathBuf>) -> Self {
        Self { template_root_dir: template_root_dir.into(), cwd: cwd.into(), root_override: None }
    }

    /// Resolve from `BOILERSYNC_TEMPLATE_DIR`, `BOILERSYNC_ROOT_DIR` and the
    /// process working directory.
    pub fn from_env() -> Result<Self> {
        let template_root_dir = match std::env::var(TEMPLATE_DIR_ENV) {
            Ok(value) if !value.is_empty() => fsx::expanduser(&value),
            _ => default_template_root_dir()?,
        };
        let cwd = std::env::current_dir()?;
        let mut paths = Self::new(template_root_dir, cwd);
        if let Ok(root) = std::env::var(ROOT_DIR_ENV)
            && !root.is_empty()
        {
            paths.root_override = Some(PathBuf::from(root));
        }
        Ok(paths)
    }

    /// Use `root` as the project root instead of searching upward from the
    /// working directory.
    pub fn with_root_override(mut self, root: impl Into<PathBuf>) -> Self {
        self.root_override = Some(root.into());
        self
    }

    /// Same template cache, different working directory.
    pub fn at(&self, cwd: impl Into<PathBuf>) -> Self {
        Self::new(self.template_root_dir.clone(), cwd)
    }

    pub fn template_root_dir(&self) -> &Path {
        &self.template_root_dir
    }

    pub fn cwd(&self) -> &Path {
        &self.cwd
    }

    /// The nearest directory at or above the working directory that contains
    /// a `.boilersync` file (or the configured override).
    pub fn root_dir(&self) -> Result<PathBuf> {
        if let Some(root) = &self.root_override {
            return Ok(root.clone());
        }
        for dir in self.cwd.ancestors() {
            if is_boilersync_manifest(&dir.join(METADATA_FILENAME)) {
                return Ok(dir.to_path_buf());
            }
        }
        Err(Error::NotFound(format!("Could not find {METADATA_FILENAME} in any parent directory")))
    }

    /// The project's `.boilersync` file.
    pub fn boilersync_json_path(&self) -> Result<PathBuf> {
        Ok(self.root_dir()?.join(METADATA_FILENAME))
    }
}

/// `~/.boilersync/templates`.
pub fn default_template_root_dir() -> Result<PathBuf> {
    let home = dirs::home_dir().ok_or_else(|| Error::Other("Could not determine the home directory".to_string()))?;
    Ok(home.join(".boilersync").join("templates"))
}

/// A `.boilersync` entry counts only when it is a regular file (a directory
/// of that name, as in the template cache, is not project metadata).
pub fn is_boilersync_manifest(path: &Path) -> bool {
    path.is_file()
}

/// The nearest ancestor of `start_dir` (excluding `start_dir` itself) that
/// holds a `.boilersync` file.
pub fn find_parent_boilersync(start_dir: &Path) -> Option<PathBuf> {
    start_dir
        .ancestors()
        .skip(1)
        .find(|dir| is_boilersync_manifest(&dir.join(METADATA_FILENAME)))
        .map(Path::to_path_buf)
}

fn read_json_object(path: &Path) -> Option<serde_json::Map<String, Value>> {
    let text = std::fs::read_to_string(path).ok()?;
    match serde_json::from_str::<Value>(&text).ok()? {
        Value::Object(map) => Some(map),
        _ => None,
    }
}

/// Register `child_path` in the `children` list of the parent's `.boilersync`.
/// Unreadable parent metadata is reported and skipped.
pub fn add_child_to_parent(child_path: &Path, parent_boilersync_path: &Path) -> Result<()> {
    let Some(mut parent_data) = read_json_object(parent_boilersync_path) else {
        log::warn!("Could not read parent .boilersync file at {}", parent_boilersync_path.display());
        return Ok(());
    };
    let Some(parent_dir) = parent_boilersync_path.parent() else {
        return Ok(());
    };
    let Ok(relative) = child_path.strip_prefix(parent_dir) else {
        log::warn!("Child path {} is not relative to parent {}", child_path.display(), parent_dir.display());
        return Ok(());
    };
    let relative = fsx::posix(relative);

    let children = parent_data.entry("children").or_insert_with(|| Value::Array(Vec::new()));
    let Value::Array(children) = children else {
        log::warn!("Parent .boilersync 'children' is not a list: {}", parent_boilersync_path.display());
        return Ok(());
    };
    if children.iter().any(|child| child.as_str() == Some(&relative)) {
        return Ok(());
    }
    children.push(Value::String(relative.clone()));
    let mut text = serde_json::to_string_pretty(&parent_data)?;
    text.push('\n');
    std::fs::write(parent_boilersync_path, text).map_err(|e| Error::io(parent_boilersync_path, e))?;
    log::info!("Added child project '{relative}' to parent .boilersync");
    Ok(())
}

/// Absolute paths of the existing child projects listed in a `.boilersync`.
pub fn children_from_boilersync(boilersync_path: &Path) -> Vec<PathBuf> {
    let Some(data) = read_json_object(boilersync_path) else {
        return Vec::new();
    };
    let Some(parent_dir) = boilersync_path.parent() else {
        return Vec::new();
    };
    let mut result = Vec::new();
    for child in data.get("children").and_then(Value::as_array).into_iter().flatten() {
        let Some(relative) = child.as_str() else { continue };
        let child_dir = parent_dir.join(relative);
        if child_dir.is_dir() {
            result.push(child_dir);
        } else {
            log::warn!("Child project path does not exist: {}", child_dir.display());
        }
    }
    result
}

/// Relative child paths recorded in project metadata (as written).
pub fn child_paths_from_metadata(metadata: &serde_json::Map<String, Value>) -> Vec<PathBuf> {
    metadata
        .get("children")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(PathBuf::from)
        .collect()
}
