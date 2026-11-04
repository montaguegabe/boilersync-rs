//! `boilersync templates ...`: the local template cache.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::Serialize;
use serde_json::Value;

use crate::error::{Error, Result};
use crate::fsx;
use crate::git;
use crate::inheritance::get_template_inheritance_chain;
use crate::paths::Paths;
use crate::template_config::get_template_config;
use crate::template_processor::{TEMPLATE_CONFIG_FILENAME, scan_template_for_variables};
use crate::template_sources::{TemplateSource, parse_repo_locator};

const EXCLUDED_TEMPLATE_SCAN_DIRS: &[&str] =
    &[".git", ".github", ".idea", ".vscode", "__pycache__", "build", "dist", "node_modules"];

/// Variables every template gets for free; not reported as inputs.
const DEFAULT_CONTEXT_VARIABLES: &[&str] = &[
    "NAME_SNAKE",
    "NAME_PASCAL",
    "NAME_KEBAB",
    "NAME_CAMEL",
    "NAME_PRETTY",
    "name_pascal",
    "name_kebab",
    "name_camel",
];

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct LocalTemplate {
    pub template_ref: String,
    pub org: String,
    pub repo: String,
    pub subdir: String,
    pub template_dir: PathBuf,
    pub display_name: String,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct TemplateSourceInfo {
    pub org: String,
    pub repo: String,
    pub path: PathBuf,
    pub remote_url: Option<String>,
    pub branch: Option<String>,
    pub commit: Option<String>,
    pub template_count: usize,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct DuplicateRemote {
    pub remote_url: String,
    pub paths: Vec<PathBuf>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct TemplateSourcesReport {
    pub template_root_dir: PathBuf,
    pub source_count: usize,
    pub sources: Vec<TemplateSourceInfo>,
    pub duplicate_remotes: Vec<DuplicateRemote>,
}

/// A template input as shown by `templates details`.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct InputDefinition {
    pub name: String,
    pub label: String,
    #[serde(rename = "type")]
    pub value_type: String,
    pub required: bool,
    pub description: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub choices: Option<Vec<Value>>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct TemplateDetails {
    pub template_ref: String,
    pub template_dir: PathBuf,
    pub template_root_dir: PathBuf,
    pub inheritance_chain: Vec<String>,
    pub variables: Vec<InputDefinition>,
    pub options: Vec<InputDefinition>,
}

fn walk_template_dirs(dir: &Path, repo_dir: &Path, found: &mut Vec<String>) -> Result<()> {
    let entries = fsx::sorted_entries(dir)?;
    let mut subdirs = Vec::new();
    let mut visible_files = Vec::new();
    for entry in &entries {
        let name = entry.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        if entry.is_dir() {
            if !name.starts_with('.') && !EXCLUDED_TEMPLATE_SCAN_DIRS.contains(&name.as_str()) {
                subdirs.push(entry.clone());
            }
        } else if !name.starts_with('.') {
            visible_files.push(name);
        }
    }
    if dir != repo_dir && visible_files.iter().any(|name| name == TEMPLATE_CONFIG_FILENAME) {
        // A template root is identified by template.json, so its child
        // directories are not exposed as additional templates.
        found.push(fsx::posix(dir.strip_prefix(repo_dir).unwrap_or(dir)));
        return Ok(());
    }
    for subdir in subdirs {
        walk_template_dirs(&subdir, repo_dir, found)?;
    }
    Ok(())
}

fn iter_repo_template_subdirs(repo_dir: &Path) -> Result<Vec<String>> {
    let mut found = Vec::new();
    walk_template_dirs(repo_dir, repo_dir, &mut found)?;
    found.sort();
    found.dedup();
    Ok(found)
}

fn source_repo_dirs(template_root_dir: &Path) -> Result<Vec<(String, String, PathBuf)>> {
    let mut repos = Vec::new();
    if !template_root_dir.exists() {
        return Ok(repos);
    }
    for org_dir in fsx::sorted_entries(template_root_dir)? {
        let org = org_dir.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        if !org_dir.is_dir() || org.starts_with('.') {
            continue;
        }
        for repo_dir in fsx::sorted_entries(&org_dir)? {
            let repo = repo_dir.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            if !repo_dir.is_dir() || repo.starts_with('.') || !repo_dir.join(".git").exists() {
                continue;
            }
            repos.push((org.clone(), repo, repo_dir));
        }
    }
    Ok(repos)
}

/// Every template (directory with a `template.json`) in the local cache.
pub fn list_local_templates(paths: &Paths) -> Result<Vec<LocalTemplate>> {
    let mut templates = Vec::new();
    for (org, repo, repo_dir) in source_repo_dirs(paths.template_root_dir())? {
        for subdir in iter_repo_template_subdirs(&repo_dir)? {
            let template_ref = format!("{org}/{repo}#{subdir}");
            templates.push(LocalTemplate {
                display_name: template_ref.clone(),
                template_ref,
                org: org.clone(),
                repo: repo.clone(),
                template_dir: repo_dir.join(&subdir),
                subdir,
            });
        }
    }
    Ok(templates)
}

/// The source checkouts in the cache, with duplicate remotes flagged.
pub fn list_template_sources(paths: &Paths) -> Result<TemplateSourcesReport> {
    let mut sources = Vec::new();
    for (org, repo, repo_dir) in source_repo_dirs(paths.template_root_dir())? {
        sources.push(TemplateSourceInfo {
            org,
            repo,
            remote_url: git::optional_output(&repo_dir, &["config", "--get", "remote.origin.url"]),
            branch: git::optional_output(&repo_dir, &["branch", "--show-current"]),
            commit: git::optional_output(&repo_dir, &["rev-parse", "--short", "HEAD"]),
            template_count: iter_repo_template_subdirs(&repo_dir)?.len(),
            path: repo_dir,
        });
    }
    let mut paths_by_remote: BTreeMap<String, Vec<PathBuf>> = BTreeMap::new();
    for source in &sources {
        if let Some(remote) = &source.remote_url {
            paths_by_remote.entry(remote.clone()).or_default().push(source.path.clone());
        }
    }
    let duplicate_remotes = paths_by_remote
        .into_iter()
        .filter(|(_, paths)| paths.len() > 1)
        .map(|(remote_url, mut paths)| {
            paths.sort();
            DuplicateRemote { remote_url, paths }
        })
        .collect();
    Ok(TemplateSourcesReport {
        template_root_dir: paths.template_root_dir().to_path_buf(),
        source_count: sources.len(),
        sources,
        duplicate_remotes,
    })
}

fn find_existing_source_for_remote(
    paths: &Paths,
    remote_url: &str,
    exclude_path: &Path,
) -> Result<Option<TemplateSourceInfo>> {
    let excluded = fsx::resolve(exclude_path);
    Ok(list_template_sources(paths)?
        .sources
        .into_iter()
        .find(|source| fsx::resolve(&source.path) != excluded && source.remote_url.as_deref() == Some(remote_url)))
}

fn title_case(name: &str) -> String {
    name.replace('_', " ")
        .trim()
        .split(' ')
        .map(|word| {
            let mut chars = word.chars();
            match chars.next() {
                Some(first) => first.to_uppercase().chain(chars.flat_map(|c| c.to_lowercase())).collect::<String>(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn non_empty_str(value: Option<&Value>) -> Option<String> {
    value.and_then(Value::as_str).map(str::trim).filter(|text| !text.is_empty()).map(str::to_string)
}

/// Normalize the three `template.json` input forms (object, choice list,
/// scalar default) into one definition.
pub fn normalize_input_definition(name: &str, raw: Option<&Value>, default_required: bool) -> InputDefinition {
    let mut result = InputDefinition {
        name: name.to_string(),
        label: title_case(name),
        value_type: "string".to_string(),
        required: default_required,
        description: String::new(),
        default: None,
        choices: None,
    };
    match raw {
        Some(Value::Object(raw)) => {
            if let Some(label) = non_empty_str(raw.get("label")).or_else(|| non_empty_str(raw.get("prompt"))) {
                result.label = label;
            }
            if let Some(description) = non_empty_str(raw.get("description")) {
                result.description = description;
            }
            if let Some(value_type) = non_empty_str(raw.get("type")) {
                result.value_type = value_type.to_lowercase();
            }
            if let Some(default) = raw.get("default") {
                result.default = Some(default.clone());
                if !raw.contains_key("required") {
                    result.required = false;
                }
            }
            if let Some(Value::Bool(required)) = raw.get("required") {
                result.required = *required;
            }
            let choices = ["choices", "options", "enum"]
                .iter()
                .find_map(|key| raw.get(*key).and_then(Value::as_array).filter(|items| !items.is_empty()));
            if let Some(choices) = choices {
                result.choices = Some(choices.clone());
            }
        }
        Some(Value::Array(items)) if !items.is_empty() => result.choices = Some(items.clone()),
        Some(Value::Null) | None => {}
        Some(other) => result.default = Some(other.clone()),
    }
    result
}

fn merge_definition(existing: &InputDefinition, update: InputDefinition) -> InputDefinition {
    InputDefinition {
        default: update.default.or_else(|| existing.default.clone()),
        choices: update.choices.or_else(|| existing.choices.clone()),
        ..update
    }
}

fn merge_input_metadata(chain: &[TemplateSource], key: &str) -> BTreeMap<String, InputDefinition> {
    let mut merged: BTreeMap<String, InputDefinition> = BTreeMap::new();
    for source in chain {
        let config = get_template_config(&source.template_dir);
        let Some(Value::Object(definitions)) = config.get(key) else { continue };
        for (name, raw) in definitions {
            let normalized = normalize_input_definition(name, Some(raw), false);
            let definition = match merged.get(name) {
                Some(existing) => merge_definition(existing, normalized),
                None => normalized,
            };
            merged.insert(name.clone(), definition);
        }
    }
    merged
}

fn built_in_definition(name: &str) -> Option<(&'static str, &'static str)> {
    match name {
        "name_snake" => Some(("Project Name", "Snake_case project name. Defaults to the target directory name.")),
        "name_pretty" => Some(("Pretty Name", "Display name. Defaults to a prettified version of name_snake.")),
        _ => None,
    }
}

/// The effective inputs of a template after resolving inheritance.
pub fn get_template_details(paths: &Paths, template_ref: &str) -> Result<TemplateDetails> {
    let chain = get_template_inheritance_chain(paths, template_ref)?;
    let leaf = chain.last().expect("chain is never empty");

    let mut variable_names = std::collections::BTreeSet::new();
    for source in &chain {
        variable_names.extend(scan_template_for_variables(&source.template_dir, false)?);
    }
    for default in DEFAULT_CONTEXT_VARIABLES {
        variable_names.remove(*default);
    }
    let variable_metadata = merge_input_metadata(&chain, "variables");
    let option_metadata = merge_input_metadata(&chain, "options");
    variable_names.insert("name_snake".to_string());
    variable_names.insert("name_pretty".to_string());

    let mut all_names = variable_names.clone();
    all_names.extend(variable_metadata.keys().cloned());
    let variables = all_names
        .iter()
        .map(|name| {
            let mut definition = match variable_metadata.get(name) {
                Some(metadata) => {
                    let mut definition = metadata.clone();
                    if !variable_names.contains(name) {
                        definition.required = metadata.required;
                    }
                    definition
                }
                None => normalize_input_definition(name, None, variable_names.contains(name)),
            };
            if let Some((label, description)) = built_in_definition(name) {
                definition.label = label.to_string();
                definition.value_type = "string".to_string();
                definition.required = false;
                definition.description = description.to_string();
            }
            definition
        })
        .collect();
    let options = option_metadata.into_values().collect();

    Ok(TemplateDetails {
        template_ref: leaf.ref_.clone(),
        template_dir: leaf.template_dir.clone(),
        template_root_dir: paths.template_root_dir().to_path_buf(),
        inheritance_chain: chain.iter().map(|source| source.ref_.clone()).collect(),
        variables,
        options,
    })
}

/// Clone a template source repository into the cache (`templates init`).
pub fn init_templates(paths: &Paths, repo_url: Option<&str>, no_input: bool) -> Result<()> {
    let repo_url = match repo_url.map(str::trim).filter(|text| !text.is_empty()) {
        Some(url) => url.to_string(),
        None => {
            if no_input {
                return Err(Error::Usage("Template repository URL is required when --no-input is used.".to_string()));
            }
            let url = crate::prompt::prompt("GitHub repository URL to clone into local template cache")?;
            if url.is_empty() {
                return Err(Error::Usage("Template repository URL cannot be empty.".to_string()));
            }
            url
        }
    };
    let locator = parse_repo_locator(&repo_url).map_err(|e| Error::Usage(e.to_string()))?;
    let target_dir = paths.template_root_dir().join(&locator.org).join(&locator.repo);
    if let Some(parent) = target_dir.parent() {
        std::fs::create_dir_all(parent).map_err(|e| Error::io(parent, e))?;
    }
    if target_dir.exists() {
        if target_dir.join(".git").exists() {
            println!("✅ Template source already initialized at: {}", target_dir.display());
            return Ok(());
        }
        if std::fs::read_dir(&target_dir).map_err(|e| Error::io(&target_dir, e))?.next().is_some() {
            return Err(Error::Usage(format!(
                "Template source directory already exists and is not empty: {}",
                target_dir.display()
            )));
        }
    }
    if let Some(duplicate) = find_existing_source_for_remote(paths, &locator.clone_url, &target_dir)? {
        return Err(Error::Usage(format!(
            "Template source remote is already initialized at {}. Use that source instead of creating a duplicate checkout at {}.",
            duplicate.path.display(),
            target_dir.display()
        )));
    }
    println!("📦 Cloning template source into: {}", target_dir.display());
    let parent = target_dir.parent().unwrap_or(&target_dir).to_path_buf();
    git::git_passthrough(&parent, &["clone", &locator.clone_url, &target_dir.display().to_string()])
        .map_err(|e| Error::Usage(format!("Failed to clone template source: {e}")))?;
    println!("✅ Template source initialized successfully.");
    Ok(())
}
