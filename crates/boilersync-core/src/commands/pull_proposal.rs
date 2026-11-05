//! `boilersync pull-proposal`: a reviewable pull in a scratch copy of the
//! project, applied back selectively.

use std::path::{Path, PathBuf};
use std::process::Command;

use serde::Serialize;

use crate::commands::pull::{PullOptions, pull};
use crate::error::{Error, Result};
use crate::file_ignores::copy_project_files_to_directory;
use crate::fsx;
use crate::git::{self, DiffEntry};
use crate::paths::Paths;
use crate::project_context::resolve_project_template_context;

#[derive(Debug, Clone, Serialize)]
pub struct PullProposal {
    pub project_dir: PathBuf,
    pub workspace_dir: PathBuf,
    pub proposal_dir: PathBuf,
    pub changed_files: Vec<DiffEntry>,
}

/// Options for [`create_pull_proposal`].
#[derive(Debug, Clone, Default)]
pub struct ProposalOptions {
    pub include_starter: bool,
    pub include_children: bool,
    /// Directory for the proposal workspace; a per-project temp dir when `None`.
    pub workspace_dir: Option<PathBuf>,
}

fn is_safe_relative_path(path: &Path) -> bool {
    !path.is_absolute() && !path.components().any(|c| matches!(c, std::path::Component::ParentDir))
}

/// Copy the project into a scratch git repo, commit it, run the pull there and
/// report what changed.
pub fn create_pull_proposal(
    paths: &Paths,
    project_dir: Option<&Path>,
    options: &ProposalOptions,
) -> Result<PullProposal> {
    let project_dir = fsx::resolve(&match project_dir {
        Some(dir) => dir.to_path_buf(),
        None => paths.root_dir()?,
    });
    let workspace_dir = match &options.workspace_dir {
        Some(dir) => fsx::expanduser(&dir.display().to_string()),
        None => std::env::temp_dir().join(format!("boilersync-pull-{}", fsx::path_hash(&project_dir))),
    };
    let proposal_dir = workspace_dir.join("proposal");

    fsx::recreate_dir(&proposal_dir)?;
    copy_project_files_to_directory(&project_dir, &proposal_dir, &[])?;
    git::init_scratch_repo(&proposal_dir)?;
    git::commit_all(&proposal_dir, "Current project before Boilersync pull")?;

    let project_context = resolve_project_template_context(paths, &proposal_dir)?;
    let mut collected_variables = project_context.variables.clone();
    if let Some(snake) = &project_context.name_snake {
        collected_variables.insert("name_snake".to_string(), serde_json::Value::String(snake.clone()));
    }
    if let Some(pretty) = &project_context.name_pretty {
        collected_variables.insert("name_pretty".to_string(), serde_json::Value::String(pretty.clone()));
    }

    eprintln!("Creating pull proposal in {}", proposal_dir.display());
    pull(
        &paths.at(proposal_dir.clone()),
        PullOptions {
            template_ref: Some(project_context.template_ref.clone()),
            collected_variables: Some(collected_variables),
            allow_non_empty: true,
            include_starter: options.include_starter,
            no_input: true,
            target_dir: Some(proposal_dir.clone()),
            recursive: options.include_children,
        },
    )?;
    git::git(&proposal_dir, &["add", "-N", "-A"])?;
    let changed_files = git::parse_name_status(&git::git(&proposal_dir, &["diff", "--name-status"])?);

    Ok(PullProposal { project_dir, workspace_dir, proposal_dir, changed_files })
}

pub fn proposal_to_json(proposal: &PullProposal) -> Result<String> {
    let payload = serde_json::json!({
        "project_dir": proposal.project_dir,
        "workspace_dir": proposal.workspace_dir,
        "proposal_dir": proposal.proposal_dir,
        "changed_count": proposal.changed_files.len(),
        "changed_files": proposal.changed_files,
    });
    Ok(serde_json::to_string_pretty(&payload)?)
}

pub fn proposal_to_text(proposal: &PullProposal) -> String {
    let mut lines = vec![
        format!("Project: {}", proposal.project_dir.display()),
        format!("Proposal: {}", proposal.proposal_dir.display()),
        String::new(),
        "Review proposed changes with:".to_string(),
        format!("  git -C {} diff", proposal.proposal_dir.display()),
        String::new(),
        "Apply a whole proposed file from the project root with:".to_string(),
        format!("  boilersync pull-proposal apply-file {} PATH", proposal.proposal_dir.display()),
        String::new(),
        "Apply selected hunks by saving a patch and running:".to_string(),
        "  boilersync pull-proposal apply-patch PATCH".to_string(),
        String::new(),
        format!("Changed files: {}", proposal.changed_files.len()),
    ];
    for entry in &proposal.changed_files {
        lines.push(format!("  {}\t{}", entry.status, entry.path));
    }
    lines.join("\n")
}

/// Copy one proposed file into the project. Returns the written path.
pub fn apply_proposed_file(
    paths: &Paths,
    proposal_dir: &Path,
    relative_path: &Path,
    project_dir: Option<&Path>,
) -> Result<PathBuf> {
    if !is_safe_relative_path(relative_path) {
        return Err(Error::Invalid(format!(
            "Path must be relative and stay inside the project: {}",
            relative_path.display()
        )));
    }
    let project_dir = fsx::resolve(&match project_dir {
        Some(dir) => dir.to_path_buf(),
        None => paths.root_dir()?,
    });
    let source = fsx::expanduser(&proposal_dir.display().to_string()).join(relative_path);
    if !source.is_file() {
        return Err(Error::NotFound(format!("Proposed file does not exist: {}", source.display())));
    }
    let target = project_dir.join(relative_path);
    fsx::copy_file(&source, &target)?;
    Ok(target)
}

/// Apply a saved patch to the project with `git apply`.
pub fn apply_patch_file(paths: &Paths, patch_path: &Path, project_dir: Option<&Path>) -> Result<()> {
    let project_dir = fsx::resolve(&match project_dir {
        Some(dir) => dir.to_path_buf(),
        None => paths.root_dir()?,
    });
    let patch = fsx::expanduser(&patch_path.display().to_string());
    let status = Command::new("git")
        .arg("apply")
        .arg(&patch)
        .current_dir(&project_dir)
        .status()
        .map_err(|e| Error::Git(format!("Failed to run git apply: {e}")))?;
    if !status.success() {
        return Err(Error::Git(format!("git apply {} failed ({status})", patch.display())));
    }
    Ok(())
}
