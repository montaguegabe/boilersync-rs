//! Which project files take part in comparisons and pushes: git-visible files
//! when the project is a git repository, minus well-known dependency, build
//! and cache directories.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use crate::error::Result;
use crate::fsx;
use crate::git;

/// Directory names never copied into a scratch workspace.
pub const DEFAULT_COPY_IGNORED_NAMES: &[&str] = &[
    ".cache",
    ".git",
    ".mypy_cache",
    ".next",
    ".pytest_cache",
    ".ruff_cache",
    ".turbo",
    ".venv",
    "__pycache__",
    "build",
    "dist",
    "node_modules",
];

/// True when any path component is one of the ignored directory names.
pub fn is_default_copy_ignored(path: &Path) -> bool {
    path.components()
        .any(|component| DEFAULT_COPY_IGNORED_NAMES.contains(&component.as_os_str().to_string_lossy().as_ref()))
}

/// The `CLAUDE.md -> AGENTS.md` symlink that keeps Claude Code reading a
/// project's `AGENTS.md`. It is editor plumbing, not template content.
pub fn is_agents_compatibility_symlink(root_dir: &Path, path: &Path) -> bool {
    if path != Path::new("CLAUDE.md") {
        return false;
    }
    let source = root_dir.join(path);
    if !source.is_symlink() {
        return false;
    }
    std::fs::read_link(&source).map(|target| target == Path::new("AGENTS.md")).unwrap_or(false)
}

/// Git-visible file paths relative to `root_dir`, or `None` outside git.
///
/// Git-visible means tracked files plus untracked files that are not ignored
/// by the repo's normal ignore rules. This keeps scratch workspaces from
/// copying ignored dependency, build and cache files.
pub fn git_visible_files(root_dir: &Path) -> Option<Vec<PathBuf>> {
    if !git::is_inside_work_tree(root_dir) {
        return None;
    }
    let output = git::git_output(
        root_dir,
        &["ls-files", "-z", "--cached", "--others", "--exclude-standard", "--deduplicate", "--", "."],
    )
    .ok()?;
    if !output.status.success() {
        return None;
    }
    let mut paths = Vec::new();
    for raw in output.stdout.split(|byte| *byte == 0) {
        if raw.is_empty() {
            continue;
        }
        let path = PathBuf::from(String::from_utf8_lossy(raw).into_owned());
        if path.is_absolute()
            || path.components().any(|c| matches!(c, std::path::Component::ParentDir))
            || is_default_copy_ignored(&path)
            || is_agents_compatibility_symlink(root_dir, &path)
        {
            continue;
        }
        if root_dir.join(&path).is_file() {
            paths.push(path);
        }
    }
    Some(paths)
}

fn walk(dir: &Path, root_dir: &Path, paths: &mut Vec<PathBuf>) -> Result<()> {
    for entry in fsx::sorted_entries(dir)? {
        let relative = entry.strip_prefix(root_dir).unwrap_or(&entry).to_path_buf();
        if is_default_copy_ignored(&relative) {
            continue;
        }
        if entry.is_dir() && !entry.is_symlink() {
            walk(&entry, root_dir, paths)?;
        } else if entry.is_file() && !is_agents_compatibility_symlink(root_dir, &relative) {
            paths.push(relative);
        }
    }
    Ok(())
}

/// The project files that take part in comparisons: git-visible files inside
/// a repository, otherwise every file outside the ignored directories.
pub fn copyable_project_files(root_dir: &Path) -> Result<Vec<PathBuf>> {
    if let Some(paths) = git_visible_files(root_dir) {
        return Ok(paths);
    }
    let mut paths = Vec::new();
    walk(root_dir, root_dir, &mut paths)?;
    Ok(paths)
}

/// Copy the copyable project files from `source_dir` to `target_dir`, skipping
/// files whose name is in `exclude_names`.
pub fn copy_project_files_to_directory(source_dir: &Path, target_dir: &Path, exclude_names: &[&str]) -> Result<()> {
    let excluded: HashSet<&str> = exclude_names.iter().copied().collect();
    for relative in copyable_project_files(source_dir)? {
        let name = relative.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        if excluded.contains(name.as_str()) {
            continue;
        }
        fsx::copy_file(&source_dir.join(&relative), &target_dir.join(&relative))?;
    }
    Ok(())
}
