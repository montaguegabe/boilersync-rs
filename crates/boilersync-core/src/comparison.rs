//! Comparing a project with its freshly rendered template in a scratch git
//! repository (the basis of `diff`).

use std::path::{Path, PathBuf};

use crate::error::{Error, Result};
use crate::file_ignores::copyable_project_files;
use crate::fsx;
use crate::git::{self, DiffEntry};
use crate::jinja::Context;
use crate::paths::{Paths, child_paths_from_metadata};
use crate::project_context::resolve_project_template_context;
use crate::template_workspace::{OwnershipMap, copy_rendered_template_chain};

#[derive(Debug, Clone)]
pub struct ComparisonWorkspace {
    pub project_dir: PathBuf,
    pub comparison_dir: PathBuf,
    pub template_ref: String,
    pub ownership_map: OwnershipMap,
}

/// How `git diff` output is formatted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiffMode {
    Stat,
    NameStatus,
    Patch,
}

/// True when any component of a template-relative path has `.starter` as its
/// first extension.
pub fn is_starter_source_path(path: &str) -> bool {
    Path::new(path).components().any(|component| {
        let name = component.as_os_str().to_string_lossy();
        let parts: Vec<&str> = name.split('.').collect();
        parts.len() > 1 && parts[1] == "starter"
    })
}

/// The scratch directory for comparing `project_dir`, stable per project.
pub fn comparison_dir_for(project_dir: &Path) -> PathBuf {
    std::env::temp_dir().join(format!("boilersync-diff-{}", fsx::path_hash(&fsx::resolve(project_dir)))).join("project")
}

fn is_relative_to(path: &Path, parent: &Path) -> bool {
    path.strip_prefix(parent).is_ok()
}

/// Copy the project files that belong to this project (not to a registered
/// child project) into `target_dir`.
pub fn copy_project_files_for_comparison(source_dir: &Path, target_dir: &Path, metadata: &Context) -> Result<()> {
    let child_paths = child_paths_from_metadata(metadata);
    for relative in copyable_project_files(source_dir)? {
        let name = relative.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        if name == ".boilersync" || name == ".git" || fsx::posix(&relative).contains(".git/") {
            continue;
        }
        if child_paths.iter().any(|child| relative == *child || is_relative_to(&relative, child)) {
            continue;
        }
        fsx::copy_file(&source_dir.join(&relative), &target_dir.join(&relative))?;
    }
    Ok(())
}

fn changed_paths(repo_dir: &Path) -> Result<Vec<String>> {
    Ok(git::git(repo_dir, &["diff", "--cached", "--name-only"])?
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(str::to_string)
        .collect())
}

fn filtered_changed_paths(repo_dir: &Path, ownership_map: &OwnershipMap, include_starter: bool) -> Result<Vec<String>> {
    let paths = changed_paths(repo_dir)?;
    if include_starter {
        return Ok(paths);
    }
    Ok(paths
        .into_iter()
        .filter(|path| {
            let source = ownership_map.get(path).map(|o| o.source_relative_path.as_str()).unwrap_or(path.as_str());
            !is_starter_source_path(source)
        })
        .collect())
}

/// Render the project's template chain into a scratch repo, commit it, then
/// stage the project's own files on top so `git diff --cached` shows the drift.
pub fn build_comparison_workspace(paths: &Paths, project_dir: &Path) -> Result<ComparisonWorkspace> {
    let project_context = resolve_project_template_context(paths, project_dir)?;
    let template_ref = project_context.template_ref.clone();
    let comparison_dir = comparison_dir_for(project_dir);
    fsx::recreate_dir(&comparison_dir)?;

    let mut ctx = project_context.interpolation_context();
    let ownership_map = copy_rendered_template_chain(&mut ctx, &project_context.inheritance_chain, &comparison_dir)?;

    git::init_scratch_repo(&comparison_dir)?;
    git::commit_all(&comparison_dir, &format!("Fresh template: {template_ref}"))?;

    fsx::clear_dir(&comparison_dir, &[".git"])?;
    copy_project_files_for_comparison(project_dir, &comparison_dir, &project_context.metadata)?;
    git::git(&comparison_dir, &["add", "-A"])?;

    Ok(ComparisonWorkspace { project_dir: project_dir.to_path_buf(), comparison_dir, template_ref, ownership_map })
}

/// The staged diff of the comparison workspace in the requested format.
pub fn git_diff(comparison: &ComparisonWorkspace, include_starter: bool, mode: DiffMode) -> Result<String> {
    let paths = filtered_changed_paths(&comparison.comparison_dir, &comparison.ownership_map, include_starter)?;
    if paths.is_empty() {
        return Ok(String::new());
    }
    let mut args = vec!["diff", "--cached"];
    match mode {
        DiffMode::Stat => args.push("--stat"),
        DiffMode::NameStatus => args.push("--name-status"),
        DiffMode::Patch => {}
    }
    args.push("--");
    args.extend(paths.iter().map(String::as_str));
    git::git(&comparison.comparison_dir, &args)
}

/// The changed files of the comparison workspace as structured entries.
pub fn diff_entries(comparison: &ComparisonWorkspace, include_starter: bool) -> Result<Vec<DiffEntry>> {
    let paths = filtered_changed_paths(&comparison.comparison_dir, &comparison.ownership_map, include_starter)?;
    if paths.is_empty() {
        return Ok(Vec::new());
    }
    let mut args = vec!["diff", "--cached", "--name-status", "--"];
    args.extend(paths.iter().map(String::as_str));
    let output = git::git(&comparison.comparison_dir, &args)?;
    Ok(git::parse_name_status(&output))
}

impl std::str::FromStr for DiffMode {
    type Err = Error;

    fn from_str(text: &str) -> Result<Self> {
        match text {
            "stat" => Ok(DiffMode::Stat),
            "name-status" => Ok(DiffMode::NameStatus),
            "patch" => Ok(DiffMode::Patch),
            other => Err(Error::Invalid(format!("Unknown diff mode: {other}"))),
        }
    }
}
