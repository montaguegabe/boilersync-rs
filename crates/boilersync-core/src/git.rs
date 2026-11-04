//! Thin wrappers over the `git` command line. boilersync never links libgit2.

use std::path::{Path, PathBuf};
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

/// Run git with a timeout; returns `None` when the timeout elapsed and the
/// process was killed.
pub fn git_output_timeout(cwd: &Path, args: &[&str], timeout: std::time::Duration) -> Result<Option<Output>> {
    use std::process::Stdio;
    let mut child = Command::new("git")
        .args(args)
        .current_dir(cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| Error::Git(format!("Failed to run git {}: {e}", args.join(" "))))?;
    let start = std::time::Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(_)) => return child.wait_with_output().map(Some).map_err(|e| Error::Git(e.to_string())),
            Ok(None) => {
                if start.elapsed() >= timeout {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Ok(None);
                }
                std::thread::sleep(std::time::Duration::from_millis(25));
            }
            Err(e) => return Err(Error::Git(e.to_string())),
        }
    }
}

/// Run git with `input` on stdin (for example a patch for `git apply`).
/// Returns stdout, or a [`Error::Git`] carrying stderr.
pub fn run_git_with_input(cwd: &Path, args: &[&str], input: &str) -> Result<String> {
    use std::io::Write;
    use std::process::Stdio;
    let mut child = Command::new("git")
        .args(args)
        .current_dir(cwd)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| Error::Git(format!("Failed to run git {}: {e}", args.join(" "))))?;
    if let Some(mut stdin) = child.stdin.take() {
        stdin.write_all(input.as_bytes()).map_err(|e| Error::Git(e.to_string()))?;
    }
    let output = child.wait_with_output().map_err(|e| Error::Git(e.to_string()))?;
    if !output.status.success() {
        let stderr = stderr_string(&output).trim().to_string();
        return Err(Error::Git(format!(
            "git {} failed (code {}): {stderr}",
            args.join(" "),
            output.status.code().map(|c| c.to_string()).unwrap_or_else(|| "signal".to_string())
        )));
    }
    Ok(stdout_string(&output))
}

pub fn stdout_string(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

pub fn stderr_string(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

/// The top level of the working tree containing `path`, resolved.
pub fn git_top_level(path: &Path) -> Option<PathBuf> {
    let output = git_output(path, &["rev-parse", "--show-toplevel"]).ok()?;
    if !output.status.success() {
        return None;
    }
    let top = stdout_string(&output).trim().to_string();
    if top.is_empty() {
        return None;
    }
    Some(crate::fsx::resolve(Path::new(&top)))
}

/// `true` when `repo_path` is the root of a working tree. A directory that
/// merely sits inside another repository is not.
pub fn is_git_repo_root(repo_path: &Path) -> bool {
    if !repo_path.join(".git").exists() {
        return false;
    }
    git_top_level(repo_path).is_some_and(|top| top == crate::fsx::resolve(repo_path))
}
