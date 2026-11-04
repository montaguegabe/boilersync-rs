//! Thin wrappers over the `git` command line. boilersync never links libgit2.

use std::path::Path;
use std::process::{Command, Output};

use crate::error::{Error, Result};

/// Run `git <args>` in `cwd` and return the raw output without checking the
/// exit status.
pub fn git_output(cwd: &Path, args: &[&str]) -> Result<Output> {
    Command::new("git")
        .args(args)
        .current_dir(cwd)
        .output()
        .map_err(|e| Error::Git(format!("Failed to run git {}: {e}", args.join(" "))))
}

/// Run `git <args>` in `cwd`, failing with the command's stderr when it exits
/// non-zero. Returns trimmed stdout.
pub fn git(cwd: &Path, args: &[&str]) -> Result<String> {
    let output = git_output(cwd, args)?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        let detail = if stderr.is_empty() { String::new() } else { format!(": {stderr}") };
        return Err(Error::Git(format!("git {} failed in {}{detail}", args.join(" "), cwd.display())));
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim_end_matches(['\n', '\r']).to_string())
}

/// Like [`git`] but inherits stdio so clone progress reaches the terminal.
pub fn git_passthrough(cwd: &Path, args: &[&str]) -> Result<()> {
    let status = Command::new("git")
        .args(args)
        .current_dir(cwd)
        .status()
        .map_err(|e| Error::Git(format!("Failed to run git {}: {e}", args.join(" "))))?;
    if !status.success() {
        return Err(Error::Git(format!("git {} failed ({status})", args.join(" "))));
    }
    Ok(())
}

/// True when `dir` is inside a git working tree.
pub fn is_inside_work_tree(dir: &Path) -> bool {
    git_output(dir, &["rev-parse", "--is-inside-work-tree"])
        .map(|o| o.status.success() && String::from_utf8_lossy(&o.stdout).trim() == "true")
        .unwrap_or(false)
}

/// `git init` with a fixed boilersync identity, for scratch repositories.
pub fn init_scratch_repo(dir: &Path) -> Result<()> {
    git(dir, &["init", "-q"])?;
    git(dir, &["config", "user.name", "BoilerSync"])?;
    git(dir, &["config", "user.email", "boilersync@example.invalid"])?;
    git(dir, &["config", "commit.gpgsign", "false"])?;
    Ok(())
}

/// Stage everything and commit (allowing an empty commit).
pub fn commit_all(dir: &Path, message: &str) -> Result<()> {
    git(dir, &["add", "-A"])?;
    git(dir, &["-c", "commit.gpgsign=false", "commit", "-q", "--allow-empty", "-m", message])?;
    Ok(())
}

/// HEAD commit hash of the repository at `repo_dir`.
pub fn head_commit(repo_dir: &Path) -> Result<String> {
    git(repo_dir, &["rev-parse", "HEAD"])
}

/// Output of `git -C <repo_dir> <args>`, or `None` when it fails or is empty.
pub fn optional_output(repo_dir: &Path, args: &[&str]) -> Option<String> {
    let output = git_output(repo_dir, args).ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if text.is_empty() { None } else { Some(text) }
}

/// Parse `git diff --name-status` output into entries.
pub fn parse_name_status(output: &str) -> Vec<DiffEntry> {
    output
        .lines()
        .filter_map(|line| {
            let parts: Vec<&str> = line.split('\t').collect();
            match parts.as_slice() {
                [status, path] => {
                    Some(DiffEntry { status: status.to_string(), old_path: None, path: path.to_string() })
                }
                [status, old_path, path, ..] => Some(DiffEntry {
                    status: status.to_string(),
                    old_path: Some(old_path.to_string()),
                    path: path.to_string(),
                }),
                _ => None,
            }
        })
        .collect()
}

/// One line of `git diff --name-status`.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct DiffEntry {
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub old_path: Option<String>,
    pub path: String,
}
