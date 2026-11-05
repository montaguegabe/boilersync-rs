//! Shared fixtures: a temporary template cache with committed template source
//! repositories, and helpers to initialize projects from them.

#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::process::Command;

use boilersync_core::commands::init::{InitOptions, init};
use boilersync_core::jinja::Context;
use boilersync_core::paths::Paths;
use serde_json::Value;
use tempfile::TempDir;

pub const ORG: &str = "acme";
pub const REPO: &str = "templates";

pub struct Fixture {
    pub tmp: TempDir,
    pub root: PathBuf,
    pub template_root: PathBuf,
}

pub fn fixture() -> Fixture {
    let tmp = tempfile::tempdir().expect("tempdir");
    let root = tmp.path().to_path_buf();
    let template_root = root.join("templates");
    std::fs::create_dir_all(&template_root).unwrap();
    Fixture { tmp, root, template_root }
}

impl Fixture {
    /// `Paths` resolving the project root from `cwd`.
    pub fn paths(&self, cwd: &Path) -> Paths {
        Paths::new(self.template_root.clone(), cwd)
    }

    pub fn template_ref(&self, subdir: &str) -> String {
        format!("{ORG}/{REPO}#{subdir}")
    }

    pub fn canonical_ref(&self, subdir: &str) -> String {
        format!("https://github.com/{ORG}/{REPO}.git#{subdir}")
    }

    pub fn repo_dir(&self) -> PathBuf {
        self.template_root.join(ORG).join(REPO)
    }

    /// Write a template into the fixture's source repo and commit the repo.
    /// Returns the new HEAD commit.
    pub fn write_template(&self, subdir: &str, files: &[(&str, &str)], config: Option<Value>) -> String {
        let repo_dir = self.repo_dir();
        let template_dir = repo_dir.join(subdir);
        std::fs::create_dir_all(&template_dir).unwrap();
        if let Some(config) = config {
            write(&template_dir.join("template.json"), &config.to_string());
        }
        for (relative, contents) in files {
            write(&template_dir.join(relative), contents);
        }
        commit_template_repo(&repo_dir)
    }

    /// Create a template directory without committing (a bare `.git` marker
    /// makes it count as an initialized source).
    pub fn write_uncommitted_template(&self, subdir: &str, files: &[(&str, &str)], config: Option<Value>) {
        let repo_dir = self.repo_dir();
        std::fs::create_dir_all(repo_dir.join(".git")).unwrap();
        let template_dir = repo_dir.join(subdir);
        std::fs::create_dir_all(&template_dir).unwrap();
        if let Some(config) = config {
            write(&template_dir.join("template.json"), &config.to_string());
        }
        for (relative, contents) in files {
            write(&template_dir.join(relative), contents);
        }
    }

    /// Initialize `target` (created if missing) from `subdir` without prompts.
    pub fn init_project(&self, subdir: &str, target: &Path, variables: Value) {
        std::fs::create_dir_all(target).unwrap();
        let options =
            InitOptions { template_variables: Some(context(variables)), no_input: true, ..InitOptions::new() };
        init(&self.paths(target), &self.template_ref(subdir), target, &options)
            .unwrap_or_else(|e| panic!("init {subdir} into {}: {e}", target.display()));
    }
}

/// Commit everything in `repo_dir` (initializing it when needed). Returns HEAD.
pub fn commit_template_repo(repo_dir: &Path) -> String {
    if !repo_dir.join(".git").is_dir() {
        git(repo_dir, &["init", "-q"]);
        git(repo_dir, &["config", "user.name", "Boilersync Tests"]);
        git(repo_dir, &["config", "user.email", "tests@example.invalid"]);
        git(repo_dir, &["config", "commit.gpgsign", "false"]);
    }
    git(repo_dir, &["add", "-A"]);
    if !git(repo_dir, &["status", "--porcelain"]).is_empty() {
        git(repo_dir, &["commit", "-q", "-m", "Update template fixtures"]);
    }
    git(repo_dir, &["rev-parse", "HEAD"])
}

pub fn git(cwd: &Path, args: &[&str]) -> String {
    let output = Command::new("git").args(args).current_dir(cwd).output().expect("git runs");
    assert!(
        output.status.success(),
        "git {} failed in {}: {}",
        args.join(" "),
        cwd.display(),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).trim().to_string()
}

pub fn write(path: &Path, text: &str) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
}

pub fn read(path: &Path) -> String {
    std::fs::read_to_string(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

pub fn read_json(path: &Path) -> Value {
    serde_json::from_str(&read(path)).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

pub fn context(value: Value) -> Context {
    match value {
        Value::Object(map) => map,
        other => panic!("expected an object, got {other}"),
    }
}
