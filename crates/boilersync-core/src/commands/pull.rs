//! `boilersync pull`: render the template chain into an existing (or empty)
//! project directory and record the result in `.boilersync`.

use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::error::{Error, Result};
use crate::fsx;
use crate::git;
use crate::inheritance::get_template_inheritance_chain;
use crate::interpolation::InterpolationContext;
use crate::jinja::Context;
use crate::names::{default_project_snake_from_directory_name, snake_to_pretty};
use crate::paths::{METADATA_FILENAME, Paths, children_from_boilersync};
use crate::project_metadata::{load_project_metadata, write_project_metadata};
use crate::prompt::prompt_or_default;
use crate::template_config::should_skip_git;
use crate::template_processor::process_template_directory;
use crate::template_sources::resolve_source_from_boilersync;

/// Options for [`pull`].
#[derive(Debug, Clone, Default)]
pub struct PullOptions {
    /// Template reference; auto-detected from `.boilersync` when `None`.
    pub template_ref: Option<String>,
    /// Pre-collected variables (restored from metadata when `None` and the
    /// template is auto-detected).
    pub collected_variables: Option<Context>,
    /// Allow pulling into a non-empty directory (requires a clean git repo).
    pub allow_non_empty: bool,
    /// Also render `.starter` files.
    pub include_starter: bool,
    /// Never prompt; fail on missing variables.
    pub no_input: bool,
    /// Target directory; the working directory when `None`.
    pub target_dir: Option<PathBuf>,
    /// Also pull registered child projects.
    pub recursive: bool,
}

/// True when the directory is not a git repository or has no uncommitted
/// changes (untracked files count as changes).
pub fn is_git_repo_clean(target_dir: &Path) -> bool {
    if !target_dir.join(".git").exists() {
        return true;
    }
    git::git(target_dir, &["status", "--porcelain"]).map(|output| output.trim().is_empty()).unwrap_or(true)
}

/// Pull updates for every child project listed in a `.boilersync` file.
pub fn pull_children(paths: &Paths, boilersync_path: &Path, include_starter: bool) -> Result<()> {
    let child_paths = children_from_boilersync(boilersync_path);
    if child_paths.is_empty() {
        return Ok(());
    }
    let parent_dir = boilersync_path.parent().unwrap_or(boilersync_path);
    log::info!("\n🔄 Found {} child project(s) to update:", child_paths.len());
    for child in &child_paths {
        log::info!("  📁 {}", child.strip_prefix(parent_dir).unwrap_or(child).display());
    }
    for child in &child_paths {
        let name = child.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        log::info!("\n🔄 Updating child project: {name}");
        let child_paths_ctx = paths.at(child.clone());
        let options = PullOptions {
            allow_non_empty: true,
            include_starter,
            target_dir: Some(child.clone()),
            recursive: false,
            ..PullOptions::default()
        };
        match pull(&child_paths_ctx, options) {
            Ok(()) => log::info!("✅ Updated child project: {name}"),
            Err(err) => log::error!("Failed to update child project {name}: {err}"),
        }
    }
    Ok(())
}

struct ResolvedNames {
    variables: Context,
    snake: String,
    pretty: String,
}

fn non_empty(value: Option<&Value>) -> Option<String> {
    value.and_then(Value::as_str).filter(|text| !text.is_empty()).map(str::to_string)
}

fn resolve_name_variables(
    target_dir: &Path,
    collected_variables: Option<&Context>,
    stored_name_snake: Option<&str>,
    stored_name_pretty: Option<&str>,
    no_input: bool,
) -> Result<ResolvedNames> {
    let mut variables = collected_variables.cloned().unwrap_or_default();
    let dir_name = target_dir.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();

    let snake = match non_empty(variables.get("name_snake")).or_else(|| stored_name_snake.map(str::to_string)) {
        Some(value) => value,
        None => prompt_or_default(
            "Enter value for 'name_snake'",
            &default_project_snake_from_directory_name(&dir_name),
            no_input,
        )?,
    };
    let pretty = match non_empty(variables.get("name_pretty")).or_else(|| stored_name_pretty.map(str::to_string)) {
        Some(value) => value,
        None => prompt_or_default("Enter value for 'name_pretty'", &snake_to_pretty(&snake), no_input)?,
    };
    variables.insert("name_snake".to_string(), Value::String(snake.clone()));
    variables.insert("name_pretty".to_string(), Value::String(pretty.clone()));
    Ok(ResolvedNames { variables, snake, pretty })
}

/// Pull template changes into a project directory.
pub fn pull(paths: &Paths, options: PullOptions) -> Result<()> {
    let target_dir = options.target_dir.clone().unwrap_or_else(|| paths.cwd().to_path_buf());
    let mut collected_variables = options.collected_variables.clone();
    let mut stored_name_snake = None;
    let mut stored_name_pretty = None;

    let template_ref = match &options.template_ref {
        Some(template_ref) => template_ref.clone(),
        None => {
            let project_dir = paths.root_dir().map_err(|e| auto_detect_error(&e))?;
            let data = load_project_metadata(&project_dir).map_err(|e| auto_detect_error(&e))?;
            let source = resolve_source_from_boilersync(paths, data.get("template").and_then(Value::as_str), true)
                .map_err(|e| auto_detect_error(&e))?;
            stored_name_snake = non_empty(data.get("name_snake"));
            stored_name_pretty = non_empty(data.get("name_pretty"));
            if collected_variables.is_none() {
                collected_variables = Some(match data.get("variables") {
                    Some(Value::Object(map)) => map.clone(),
                    _ => Context::new(),
                });
            }
            let template_ref = source.canonical_ref();
            log::info!("📋 Auto-detected template '{template_ref}' from .boilersync file");
            template_ref
        }
    };

    let inheritance_chain = get_template_inheritance_chain(paths, &template_ref).map_err(|e| match e {
        Error::Invalid(message) => Error::Invalid(format!("Template inheritance error: {message}")),
        other => other,
    })?;
    if inheritance_chain.len() > 1 {
        let refs: Vec<&str> = inheritance_chain.iter().map(|s| s.ref_.as_str()).collect();
        log::info!("🔗 Template inheritance chain: {}", refs.join(" → "));
    }
    let leaf = inheritance_chain.last().expect("chain is never empty");
    let leaf_template_ref = leaf.ref_.clone();

    let has_files = fsx::has_files_except_ds_store(&target_dir)?;
    if has_files {
        if !options.allow_non_empty {
            return Err(Error::AlreadyExists(format!("Target directory is not empty: {}", target_dir.display())));
        }
        if !is_git_repo_clean(&target_dir) {
            return Err(Error::Runtime(
                "Cannot pull into non-empty directory with uncommitted changes. Please commit or stash your changes first."
                    .to_string(),
            ));
        }
        log::info!("⚠️  Pulling into non-empty directory (git repo is clean)");
    }

    let names = resolve_name_variables(
        &target_dir,
        collected_variables.as_ref(),
        stored_name_snake.as_deref(),
        stored_name_pretty.as_deref(),
        options.no_input,
    )?;

    log::info!("\n🚀 Pulling from template '{leaf_template_ref}'");
    log::info!("📝 Using project name: {}", names.snake);
    log::info!("📝 Using pretty name: {}", names.pretty);

    let mut ctx = InterpolationContext::new();
    ctx.set_project_names(&names.snake, Some(&names.pretty));
    ctx.set_collected_variables(&names.variables);

    let count = inheritance_chain.len();
    for (index, template_source) in inheritance_chain.iter().enumerate() {
        if count > 1 {
            let kind = if index == 0 {
                "parent"
            } else if index == count - 1 {
                "child"
            } else {
                "intermediate"
            };
            log::info!("\n📦 Processing {kind} template '{}'...", template_source.ref_);
        }
        process_template_directory(
            &mut ctx,
            &template_source.template_dir,
            &target_dir,
            options.no_input,
            !options.include_starter,
        )?;
    }

    let collected = ctx.get_collected_variables();
    write_project_metadata(&target_dir, leaf, &names.snake, &names.pretty, &collected)?;

    if has_files && options.allow_non_empty {
        log::info!("\n✅ Template '{leaf_template_ref}' pulled successfully into existing project!");
    } else {
        log::info!("\n✅ Project initialized successfully from template '{leaf_template_ref}'!");
    }
    log::info!("📁 Created .boilersync file to track template origin");

    if options.include_starter && !should_skip_git(&inheritance_chain) && !target_dir.join(".git").exists() {
        log::info!("\n🔧 Initializing git repository...");
        git::git(&target_dir, &["init", "-q"])?;
        git::git(&target_dir, &["add", "."])?;
        git::git(
            &target_dir,
            &[
                "-c",
                "commit.gpgsign=false",
                "commit",
                "-q",
                "-m",
                &format!("Initial commit from template '{leaf_template_ref}'"),
            ],
        )?;
        log::info!("✅ Git repository initialized and all changes committed");
    }

    if options.recursive {
        pull_children(paths, &target_dir.join(METADATA_FILENAME), options.include_starter)?;
    }
    Ok(())
}

fn auto_detect_error(err: &Error) -> Error {
    Error::NotFound(format!(
        "Could not auto-detect template reference from .boilersync file: {err}. Please specify a template ref explicitly or run from a directory with .boilersync file."
    ))
}
