//! Thin wrappers over the `git` command line. boilersync never links libgit2.

use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::thread;
use std::time::{Duration, Instant};

use crate::error::{Error, Result};

/// Run `git <args>` in `cwd` and return the raw output without checking the
/// exit status.
pub fn git_output(cwd: &Path, args: &[&str]) -> Result<Output> {
    Command::new("git").args(args).current_dir(cwd).output().map_err(|e| spawn_error(args, e))
}

/// Run `git <args>` in `cwd`, failing with the command's stderr when it exits
/// non-zero. Returns trimmed stdout.
pub fn git(cwd: &Path, args: &[&str]) -> Result<String> {
    let output = git_output(cwd, args)?;
    if !output.status.success() {
        return Err(failure(cwd, args, &output));
    }
    Ok(stdout_string(&output).trim_end_matches(['\n', '\r']).to_string())
}

/// Like [`git`] but inherits stdio so clone progress reaches the terminal.
pub fn git_passthrough(cwd: &Path, args: &[&str]) -> Result<()> {
    let status = Command::new("git").args(args).current_dir(cwd).status().map_err(|e| spawn_error(args, e))?;
    if !status.success() {
        return Err(Error::Git(format!("git {} failed ({status})", args.join(" "))));
    }
    Ok(())
}

/// True when `dir` is inside a git working tree.
pub fn is_inside_work_tree(dir: &Path) -> bool {
    git_output(dir, &["rev-parse", "--is-inside-work-tree"])
        .map(|o| o.status.success() && stdout_string(&o).trim() == "true")
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
    let text = stdout_string(&output).trim().to_string();
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
/// process (with everything it spawned) was killed. Stdout and stderr are
/// drained concurrently, so output of any size cannot stall the command.
pub fn git_output_timeout(cwd: &Path, args: &[&str], timeout: Duration) -> Result<Option<Output>> {
    run_bounded(cwd, args, None, timeout)
}

/// Run git with `input` on stdin (for example a patch for `git apply`), bounded
/// by `timeout`. Returns stdout, or an [`Error::Git`] carrying stderr.
pub fn run_git_with_input(cwd: &Path, args: &[&str], input: &str, timeout: Duration) -> Result<String> {
    let Some(output) = run_bounded(cwd, args, Some(input), timeout)? else {
        return Err(Error::Git(format!("git {} timed out after {}s", args.join(" "), timeout.as_secs())));
    };
    if !output.status.success() {
        return Err(failure(cwd, args, &output));
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
    optional_output(path, &["rev-parse", "--show-toplevel"]).map(|top| crate::fsx::resolve(Path::new(&top)))
}

/// `true` when `repo_path` is the root of a working tree: it holds a `.git`
/// directory with a `HEAD`, or a `.git` file pointing at one (worktrees and
/// submodules). A directory that merely sits inside another repository is
/// not. This is a pure filesystem check so callers can afford it per repo.
pub fn is_git_repo_root(repo_path: &Path) -> bool {
    let dot_git = repo_path.join(".git");
    if dot_git.is_dir() {
        return dot_git.join("HEAD").is_file();
    }
    std::fs::read_to_string(&dot_git).is_ok_and(|text| text.starts_with("gitdir:"))
}

fn spawn_error(args: &[&str], err: io::Error) -> Error {
    Error::Git(format!("Failed to run git {}: {err}", args.join(" ")))
}

fn failure(cwd: &Path, args: &[&str], output: &Output) -> Error {
    let stderr = stderr_string(output).trim().to_string();
    let detail = if stderr.is_empty() { String::new() } else { format!(": {stderr}") };
    Error::Git(format!("git {} failed in {}{detail}", args.join(" "), cwd.display()))
}

/// Spawn git in its own process group with piped stdio, feed it `input`, and
/// collect its output within `timeout`. Every pipe is served by its own
/// thread, so neither a large patch on stdin nor voluminous output can
/// deadlock the child. On timeout the whole process group is killed, which
/// also takes down hooks, ssh and credential helpers git started; the same
/// happens when git has exited but an orphan still holds its pipes open.
fn run_bounded(cwd: &Path, args: &[&str], input: Option<&str>, timeout: Duration) -> Result<Option<Output>> {
    let mut command = Command::new("git");
    command
        .args(args)
        .current_dir(cwd)
        .stdin(if input.is_some() { Stdio::piped() } else { Stdio::null() })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    let mut child = command.spawn().map_err(|e| spawn_error(args, e))?;
    let pid = child.id();
    let deadline = Instant::now() + timeout;

    let stdin_writer = match (child.stdin.take(), input) {
        (Some(mut pipe), Some(text)) => {
            let text = text.to_owned();
            Some(thread::spawn(move || match pipe.write_all(text.as_bytes()) {
                // git may legitimately exit before reading all of its input
                // (for example when the first hunk of a patch fails); its
                // stderr, not EPIPE, then explains what happened.
                Err(e) if e.kind() != io::ErrorKind::BrokenPipe => Err(e),
                _ => Ok(()),
            }))
        }
        _ => None,
    };
    let stdout = drain(child.stdout.take());
    let stderr = drain(child.stderr.take());
    let exit = wait_in_background(child);

    let status = match recv_until(&exit, deadline) {
        Some(status) => status,
        None => {
            kill_process_tree(pid);
            let _ = exit.recv();
            let _ = stdout.recv();
            let _ = stderr.recv();
            return Ok(None);
        }
    };
    let status = status.map_err(|e| Error::Git(format!("Failed to wait for git {}: {e}", args.join(" "))))?;
    if let Some(writer) = stdin_writer {
        writer
            .join()
            .map_err(|_| Error::Git("git stdin writer panicked".to_string()))?
            .map_err(|e| Error::Git(format!("Failed to write input to git {}: {e}", args.join(" "))))?;
    }
    let stdout = collect_until(&stdout, deadline, pid);
    let stderr = collect_until(&stderr, deadline, pid);
    Ok(Some(Output { status, stdout, stderr }))
}

fn drain<R: Read + Send + 'static>(pipe: Option<R>) -> Receiver<io::Result<Vec<u8>>> {
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let mut buffer = Vec::new();
        let result = match pipe {
            Some(mut pipe) => pipe.read_to_end(&mut buffer).map(|_| buffer),
            None => Ok(buffer),
        };
        let _ = tx.send(result);
    });
    rx
}

fn wait_in_background(mut child: Child) -> Receiver<io::Result<std::process::ExitStatus>> {
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let _ = tx.send(child.wait());
    });
    rx
}

/// Receive from `rx` before `deadline`; `None` when the deadline passed or the
/// sender went away without sending.
fn recv_until<T>(rx: &Receiver<T>, deadline: Instant) -> Option<T> {
    rx.recv_timeout(deadline.saturating_duration_since(Instant::now())).ok()
}

/// Collect drained pipe bytes before `deadline`. If an orphaned descendant of
/// git still holds the pipe open past the deadline, kill the process group and
/// take whatever was read.
fn collect_until(rx: &Receiver<io::Result<Vec<u8>>>, deadline: Instant, pid: u32) -> Vec<u8> {
    let bytes = match recv_until(rx, deadline) {
        Some(bytes) => bytes,
        None => {
            kill_process_tree(pid);
            rx.recv().unwrap_or_else(|_| Ok(Vec::new()))
        }
    };
    bytes.unwrap_or_default()
}

/// Kill the process group led by `pid` (unix) or the process tree rooted at it
/// (Windows). Processes git spawned, like hooks and ssh, die with it.
fn kill_process_tree(pid: u32) {
    #[cfg(unix)]
    {
        // SAFETY: killpg has no memory-safety preconditions; the pid came from
        // a child this function spawned with its own process group.
        unsafe {
            libc::killpg(pid as libc::pid_t, libc::SIGKILL);
        }
    }
    #[cfg(windows)]
    {
        let _ = Command::new("taskkill").args(["/T", "/F", "/PID", &pid.to_string()]).output();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch_repo() -> tempfile::TempDir {
        let tmp = tempfile::tempdir().unwrap();
        init_scratch_repo(tmp.path()).unwrap();
        tmp
    }

    #[test]
    fn timed_run_drains_output_larger_than_a_pipe_buffer() {
        let tmp = scratch_repo();
        let dir = tmp.path();
        // 3000 untracked files make `git status --porcelain` emit ~100KB, well
        // past the 64KB pipe buffer that used to deadlock the poll loop.
        for i in 0..3000 {
            std::fs::write(dir.join(format!("file-with-a-reasonably-long-name-{i:05}.txt")), "x").unwrap();
        }
        let output = git_output_timeout(dir, &["status", "--porcelain"], Duration::from_secs(30))
            .unwrap()
            .expect("status must not time out");
        assert!(output.status.success());
        assert!(output.stdout.len() > 65_536, "expected >64KB, got {}", output.stdout.len());
        assert_eq!(stdout_string(&output).lines().count(), 3000);
    }

    #[test]
    fn timed_run_kills_hooks_and_reports_timeout() {
        let tmp = scratch_repo();
        let dir = tmp.path();
        git(dir, &["config", "core.hooksPath", ".githooks"]).unwrap();
        std::fs::create_dir_all(dir.join(".githooks")).unwrap();
        let hook = dir.join(".githooks/pre-commit");
        std::fs::write(&hook, "#!/bin/sh\nsleep 30\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&hook, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        std::fs::write(dir.join("a.txt"), "a").unwrap();
        git(dir, &["add", "a.txt"]).unwrap();
        let started = Instant::now();
        let result = git_output_timeout(dir, &["commit", "-m", "hang"], Duration::from_millis(500)).unwrap();
        assert!(result.is_none(), "a hanging hook must surface as a timeout");
        // Killing the process group also kills the hook's `sleep`, so this
        // returns promptly instead of waiting for the hook's pipes to close.
        assert!(started.elapsed() < Duration::from_secs(10), "took {:?}", started.elapsed());
    }

    #[test]
    fn input_run_reports_git_stderr_when_patch_is_rejected() {
        let tmp = scratch_repo();
        let dir = tmp.path();
        std::fs::write(dir.join("a.txt"), "one\n").unwrap();
        commit_all(dir, "init").unwrap();
        let bad_patch = "diff --git a/a.txt b/a.txt\n--- a/a.txt\n+++ b/a.txt\n@@ -1 +1 @@\n-nope\n+two\n";
        let err = run_git_with_input(dir, &["apply", "--cached", "-"], bad_patch, Duration::from_secs(30))
            .expect_err("patch must be rejected");
        let text = err.to_string();
        assert!(text.contains("patch does not apply") || text.contains("patch failed"), "{text}");
        assert!(!text.contains("Broken pipe"), "{text}");
    }

    #[test]
    fn input_run_applies_a_large_patch() {
        let tmp = scratch_repo();
        let dir = tmp.path();
        std::fs::write(dir.join("a.txt"), "").unwrap();
        commit_all(dir, "init").unwrap();
        let mut patch = String::from("diff --git a/a.txt b/a.txt\n--- a/a.txt\n+++ b/a.txt\n@@ -0,0 +1,20000 @@\n");
        for i in 0..20000 {
            patch.push_str(&format!("+line number {i} with some padding to grow the patch\n"));
        }
        assert!(patch.len() > 65_536);
        run_git_with_input(dir, &["apply", "--cached", "-"], &patch, Duration::from_secs(30)).unwrap();
        let staged = git(dir, &["diff", "--cached", "--numstat"]).unwrap();
        assert!(staged.starts_with("20000\t0\ta.txt"), "{staged}");
    }
}
