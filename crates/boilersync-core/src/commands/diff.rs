//! `boilersync diff`: how a project (and optionally its children) diverged
//! from the rendered upstream template.

use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::comparison::{DiffMode, build_comparison_workspace, diff_entries, git_diff};
use crate::error::Result;
use crate::git::DiffEntry;
use crate::paths::{METADATA_FILENAME, Paths, children_from_boilersync};

#[derive(Debug, Clone, Serialize)]
pub struct ProjectDiff {
    pub project_dir: PathBuf,
    pub template_ref: Option<String>,
    pub comparison_dir: Option<PathBuf>,
    pub entries: Vec<DiffEntry>,
    pub output: String,
    pub error: Option<String>,
}

/// Output format of [`diff`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputMode {
    Stat,
    NameStatus,
    Patch,
    Json,
}

/// Options for [`diff`].
#[derive(Debug, Clone)]
pub struct DiffOptions {
    pub include_starter: bool,
    pub include_children: bool,
    pub recursive: bool,
    pub mode: OutputMode,
}

impl Default for DiffOptions {
    fn default() -> Self {
        Self { include_starter: false, include_children: false, recursive: false, mode: OutputMode::Stat }
    }
}

/// The root project plus its registered children (direct, or all descendants
/// with `recursive`).
pub fn collect_project_dirs(root_dir: &Path, include_children: bool, recursive: bool) -> Vec<PathBuf> {
    let mut project_dirs = vec![root_dir.to_path_buf()];
    if !include_children && !recursive {
        return project_dirs;
    }
    let mut visited = vec![crate::fsx::resolve(root_dir)];
    let mut queue = vec![root_dir.to_path_buf()];
    while !queue.is_empty() {
        let parent = queue.remove(0);
        for child in children_from_boilersync(&parent.join(METADATA_FILENAME)) {
            let resolved = crate::fsx::resolve(&child);
            if visited.contains(&resolved) {
                continue;
            }
            visited.push(resolved);
            project_dirs.push(child.clone());
            if recursive {
                queue.push(child);
            }
        }
    }
    project_dirs
}

pub fn create_project_diff(
    paths: &Paths,
    project_dir: &Path,
    include_starter: bool,
    mode: DiffMode,
) -> Result<ProjectDiff> {
    let comparison = build_comparison_workspace(paths, project_dir)?;
    let entries = diff_entries(&comparison, include_starter)?;
    let output = git_diff(&comparison, include_starter, mode)?;
    Ok(ProjectDiff {
        project_dir: project_dir.to_path_buf(),
        template_ref: Some(comparison.template_ref),
        comparison_dir: Some(comparison.comparison_dir),
        entries,
        output,
        error: None,
    })
}

/// Diff the root project and, per the options, its children. The root
/// project's errors propagate; a child's error is recorded in its entry.
pub fn create_diffs(paths: &Paths, root_dir: &Path, options: &DiffOptions, mode: DiffMode) -> Result<Vec<ProjectDiff>> {
    let project_dirs = collect_project_dirs(root_dir, options.include_children, options.recursive);
    let mut results = vec![create_project_diff(paths, &project_dirs[0], options.include_starter, mode)?];
    for project_dir in &project_dirs[1..] {
        match create_project_diff(paths, project_dir, options.include_starter, mode) {
            Ok(result) => results.push(result),
            Err(error) => results.push(ProjectDiff {
                project_dir: project_dir.clone(),
                template_ref: None,
                comparison_dir: None,
                entries: Vec::new(),
                output: String::new(),
                error: Some(error.to_string()),
            }),
        }
    }
    Ok(results)
}

pub fn results_to_json(results: &[ProjectDiff], include_starter: bool) -> Result<String> {
    let projects: Vec<serde_json::Value> = results
        .iter()
        .map(|result| {
            serde_json::json!({
                "project_dir": result.project_dir,
                "template_ref": result.template_ref,
                "comparison_dir": result.comparison_dir,
                "changed_count": result.entries.len(),
                "changed_files": result.entries,
                "error": result.error,
            })
        })
        .collect();
    Ok(serde_json::to_string_pretty(&serde_json::json!({"include_starter": include_starter, "projects": projects}))?)
}

pub fn results_to_text(results: &[ProjectDiff]) -> String {
    results
        .iter()
        .map(|result| {
            let header = format!("Project: {}", result.project_dir.display());
            if let Some(error) = &result.error {
                return format!("{header}\nError: {error}");
            }
            if result.output.trim().is_empty() {
                return format!("{header}\nNo divergence from template.");
            }
            format!(
                "{header}\nTemplate: {}\nComparison directory: {}\n{}",
                result.template_ref.as_deref().unwrap_or(""),
                result.comparison_dir.as_deref().map(|p| p.display().to_string()).unwrap_or_default(),
                result.output
            )
        })
        .collect::<Vec<_>>()
        .join("\n\n")
}

/// Run the diff for the current project and format it.
pub fn diff(paths: &Paths, options: &DiffOptions) -> Result<String> {
    let git_mode = match options.mode {
        OutputMode::Stat => DiffMode::Stat,
        OutputMode::NameStatus | OutputMode::Json => DiffMode::NameStatus,
        OutputMode::Patch => DiffMode::Patch,
    };
    let results = create_diffs(paths, &paths.root_dir()?, options, git_mode)?;
    if options.mode == OutputMode::Json {
        return results_to_json(&results, options.include_starter);
    }
    Ok(results_to_text(&results))
}
