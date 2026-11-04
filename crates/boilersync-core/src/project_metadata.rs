//! The `.boilersync` file: template provenance and saved interpolation inputs.

use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::error::{Error, Result};
use crate::git;
use crate::jinja::Context;
use crate::paths::{METADATA_FILENAME, Paths};
use crate::template_sources::{TemplateSource, resolve_source_from_boilersync};

pub const TEMPLATE_COMMIT_FIELD: &str = "template_commit";

/// Result of `boilersync check-pull`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PullCheckStatus {
    pub project_dir: PathBuf,
    pub template_ref: String,
    pub recorded_template_commit: Option<String>,
    pub current_template_commit: String,
    pub due_for_pull: bool,
}

/// Read `<project_dir>/.boilersync` as a JSON object.
pub fn load_project_metadata(project_dir: &Path) -> Result<Context> {
    let path = project_dir.join(METADATA_FILENAME);
    let text = std::fs::read_to_string(&path)
        .map_err(|e| Error::NotFound(format!("Could not read {}: {e}", path.display())))?;
    match serde_json::from_str::<Value>(&text)? {
        Value::Object(map) => Ok(map),
        _ => Err(Error::Invalid(format!("Expected {} to contain a JSON object.", path.display()))),
    }
}

/// HEAD of the cached template source repository.
pub fn get_template_repo_commit(template_source: &TemplateSource) -> Result<String> {
    git::head_commit(&template_source.local_repo_path)
}

fn write_metadata_file(path: &Path, metadata: &Context) -> Result<()> {
    let mut text = serde_json::to_string_pretty(metadata)?;
    text.push('\n');
    std::fs::write(path, text).map_err(|e| Error::io(path, e))
}

/// Write (or update) the project's `.boilersync`, keeping unrelated existing
/// keys such as `children`.
pub fn write_project_metadata(
    project_dir: &Path,
    template_source: &TemplateSource,
    name_snake: &str,
    name_pretty: &str,
    variables: &Context,
) -> Result<Context> {
    let path = project_dir.join(METADATA_FILENAME);
    let mut metadata = if path.exists() { load_project_metadata(project_dir)? } else { Context::new() };
    metadata.insert("template".to_string(), Value::String(template_source.canonical_ref()));
    metadata.insert(TEMPLATE_COMMIT_FIELD.to_string(), Value::String(get_template_repo_commit(template_source)?));
    metadata.insert("name_snake".to_string(), Value::String(name_snake.to_string()));
    metadata.insert("name_pretty".to_string(), Value::String(name_pretty.to_string()));
    metadata.insert("variables".to_string(), Value::Object(variables.clone()));
    write_metadata_file(&path, &metadata)?;
    Ok(metadata)
}

/// Compare the recorded template commit with the cached template repo HEAD.
pub fn get_pull_check_status(paths: &Paths, project_dir: Option<&Path>) -> Result<PullCheckStatus> {
    let project_dir = match project_dir {
        Some(dir) => dir.to_path_buf(),
        None => paths.root_dir()?,
    };
    let metadata = load_project_metadata(&project_dir)?;
    let template_source =
        resolve_source_from_boilersync(paths, metadata.get("template").and_then(Value::as_str), true)?;
    let recorded = metadata
        .get(TEMPLATE_COMMIT_FIELD)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .map(str::to_string);
    let current = get_template_repo_commit(&template_source)?;
    Ok(PullCheckStatus {
        project_dir,
        template_ref: template_source.canonical_ref(),
        due_for_pull: recorded.as_deref() != Some(current.as_str()),
        recorded_template_commit: recorded,
        current_template_commit: current,
    })
}
