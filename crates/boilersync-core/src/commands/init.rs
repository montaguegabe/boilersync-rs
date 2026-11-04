//! `boilersync init`: create a project from a template, then run its
//! `template.json` runtime behaviour (hooks, GitHub repo, child templates).

use std::path::Path;
use std::process::Command;

use serde_json::Value;

use crate::commands::pull::{PullOptions, pull};
use crate::conditions::{evaluate_condition, truthy};
use crate::error::{Error, Result};
use crate::fsx;
use crate::inheritance::{get_template_inheritance_chain, resolve_relative_ref};
use crate::interpolation::InterpolationContext;
use crate::jinja::{Context, py_str, render_string, render_value};
use crate::paths::{METADATA_FILENAME, Paths, add_child_to_parent, find_parent_boilersync};
use crate::project_metadata::load_project_metadata;
use crate::template_config::get_template_config;
use crate::template_sources::TemplateSource;

/// The merged runtime configuration of an inheritance chain.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct RuntimeConfig {
    pub children: Vec<Value>,
    pub pre_init: Vec<Value>,
    pub post_init: Vec<Value>,
    pub github: Context,
}

fn expect_list<'a>(value: Option<&'a Value>, template_ref: &str, key: &str) -> Result<Option<&'a Vec<Value>>> {
    match value {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Array(items)) => Ok(Some(items)),
        Some(other) if !truthy(Some(other)) => Ok(None),
        Some(_) => Err(Error::Invalid(format!("Template '{template_ref}' has invalid '{key}' config: expected list"))),
    }
}

/// Concatenate `children` and hooks and merge `github` across the chain, from
/// the root parent to the leaf template.
pub fn merge_runtime_config(inheritance_chain: &[TemplateSource]) -> Result<RuntimeConfig> {
    let mut config = RuntimeConfig::default();
    for source in inheritance_chain {
        let template_config = get_template_config(&source.template_dir);
        if let Some(children) = expect_list(template_config.get("children"), &source.ref_, "children")? {
            config.children.extend(children.iter().cloned());
        }
        match template_config.get("hooks") {
            None | Some(Value::Null) => {}
            Some(Value::Object(hooks)) => {
                if let Some(steps) = expect_list(hooks.get("pre_init"), &source.ref_, "pre_init")? {
                    config.pre_init.extend(steps.iter().cloned());
                }
                if let Some(steps) = expect_list(hooks.get("post_init"), &source.ref_, "post_init")? {
                    config.post_init.extend(steps.iter().cloned());
                }
            }
            Some(other) if !truthy(Some(other)) => {}
            Some(_) => {
                return Err(Error::Invalid(format!(
                    "Template '{}' has invalid 'hooks' config: expected object",
                    source.ref_
                )));
            }
        }
        match template_config.get("github") {
            None | Some(Value::Null) => {}
            Some(Value::Object(github)) => {
                for (key, value) in github {
                    config.github.insert(key.clone(), value.clone());
                }
            }
            Some(other) if !truthy(Some(other)) => {}
            Some(_) => {
                return Err(Error::Invalid(format!(
                    "Template '{}' has invalid 'github' config: expected object",
                    source.ref_
                )));
            }
        }
    }
    Ok(config)
}

/// Run the hook steps of `hook_name` in `target_dir`.
pub fn run_hooks(steps: &[Value], hook_name: &str, target_dir: &Path, context: &Context) -> Result<()> {
    for (index, step) in steps.iter().enumerate() {
        let Value::Object(step) = step else {
            return Err(Error::Invalid(format!("Invalid hook step in {hook_name}: expected object")));
        };
        let step_id = step.get("id").map(py_str).unwrap_or_else(|| format!("{hook_name}_{}", index + 1));
        if !evaluate_condition(step.get("condition"), context)? {
            log::info!("⏭️  Skipping {hook_name} hook '{step_id}' (condition=false)");
            continue;
        }
        let command = match step.get("run").and_then(Value::as_str).map(str::trim) {
            Some(raw) if !raw.is_empty() => render_string(raw, context)?,
            _ => {
                return Err(Error::Invalid(format!(
                    "Hook '{step_id}' in {hook_name} must define non-empty 'run' command"
                )));
            }
        };
        let hook_cwd = match step.get("cwd") {
            Some(cwd) if !cwd.is_null() => fsx::resolve(&target_dir.join(render_string(&py_str(cwd), context)?)),
            _ => target_dir.to_path_buf(),
        };
        let mut process = Command::new("sh");
        process.arg("-c").arg(&command).current_dir(&hook_cwd);
        match step.get("env") {
            None | Some(Value::Null) => {}
            Some(Value::Object(env)) => {
                for (key, value) in env {
                    process.env(key, py_str(&render_value(value, context)?));
                }
            }
            Some(other) if !truthy(Some(other)) => {}
            Some(_) => return Err(Error::Invalid(format!("Hook '{step_id}' in {hook_name} has invalid 'env' value"))),
        }
        let allow_failure = truthy(step.get("allow_failure"));
        log::info!("⚙️  Running {hook_name} hook '{step_id}'");
        let status = process.status().map_err(|e| Error::Runtime(format!("Hook '{step_id}' could not start: {e}")))?;
        if !status.success() {
            let code = status.code().map(|c| c.to_string()).unwrap_or_else(|| "signal".to_string());
            let message = format!("Hook '{step_id}' failed with exit code {code}: {command}");
            if allow_failure {
                log::warn!("{message}");
                continue;
            }
            return Err(Error::Runtime(message));
        }
    }
    Ok(())
}

fn gh(args: &[&str], cwd: Option<&Path>) -> Result<std::process::Output> {
    let mut command = Command::new("gh");
    command.args(args);
    if let Some(cwd) = cwd {
        command.current_dir(cwd);
    }
    command.output().map_err(|e| Error::Runtime(format!("Failed to run gh {}: {e}", args.join(" "))))
}

/// Create the GitHub repository described by the merged `github` config, if
/// `create_repo` is set and its condition holds. Existing repos are kept.
pub fn create_github_repo(github_config: &Context, target_dir: &Path, context: &Context) -> Result<()> {
    if github_config.is_empty() || !truthy(github_config.get("create_repo")) {
        return Ok(());
    }
    if !evaluate_condition(github_config.get("condition"), context)? {
        return Ok(());
    }
    let repo_name_template = github_config.get("repo_name").map(py_str).unwrap_or_else(|| "$${name_kebab}".to_string());
    let repo_name = render_string(&repo_name_template, context)?.trim().to_string();
    if repo_name.is_empty() {
        return Err(Error::Invalid("GitHub repo_name resolved to an empty value".to_string()));
    }

    let user = gh(&["api", "user", "--jq", ".login"], None)?;
    if !user.status.success() {
        return Err(Error::Runtime(format!("gh api user failed: {}", String::from_utf8_lossy(&user.stderr).trim())));
    }
    let github_user = String::from_utf8_lossy(&user.stdout).trim().to_string();
    let full_repo_name = format!("{github_user}/{repo_name}");

    if gh(&["repo", "view", &full_repo_name], None)?.status.success() {
        log::info!("✅ GitHub repository already exists: {full_repo_name}");
        return Ok(());
    }
    let private = github_config.get("private").map(|v| truthy(Some(v))).unwrap_or(true);
    let visibility = if private { "--private" } else { "--public" };
    let created = gh(&["repo", "create", &full_repo_name, visibility], Some(target_dir))?;
    if !created.status.success() {
        return Err(Error::Runtime(format!(
            "gh repo create {full_repo_name} failed: {}",
            String::from_utf8_lossy(&created.stderr).trim()
        )));
    }
    log::info!("✅ Created GitHub repository: {full_repo_name}");
    Ok(())
}

fn merge_contexts(first: Option<&Context>, second: Option<&Context>) -> Option<Context> {
    let mut merged = Context::new();
    for source in [first, second].into_iter().flatten() {
        for (key, value) in source {
            merged.insert(key.clone(), value.clone());
        }
    }
    if merged.is_empty() { None } else { Some(merged) }
}

/// Append the `multi convert-monorepo` post-init hook when the generated
/// workspace has a `multi.json`.
pub fn inject_monorepo_conversion_hook(runtime_config: &mut RuntimeConfig, target_dir: &Path) {
    let multi_json = target_dir.join("multi.json");
    if !multi_json.exists() {
        log::warn!(
            "--monorepo was requested but no multi.json was found at {}; skipping monorepo conversion hook.",
            multi_json.display()
        );
        return;
    }
    runtime_config
        .post_init
        .push(serde_json::json!({"id": "convert-monorepo", "run": "multi convert-monorepo --confirm"}));
}

/// Options for [`init`].
#[derive(Debug, Clone, Default)]
pub struct InitOptions {
    /// Variables collected earlier (lower precedence than `template_variables`).
    pub collected_variables: Option<Context>,
    /// Variables from `--var`, or passed down from a parent template's `children` entry.
    pub template_variables: Option<Context>,
    /// Runtime options (`--option`) merged into the context used for conditions and hooks.
    pub options: Option<Context>,
    pub no_input: bool,
    pub run_hooks: bool,
    pub run_children: bool,
    /// Convert the generated workspace to multi monorepo mode after init.
    pub monorepo: bool,
}

impl InitOptions {
    /// Defaults matching the CLI: hooks and children run.
    pub fn new() -> Self {
        Self { run_hooks: true, run_children: true, ..Self::default() }
    }
}

fn string_config(child: &Context, key: &str, index: usize, what: &str) -> Result<String> {
    match child.get(key).and_then(Value::as_str) {
        Some(value) if !value.is_empty() => Ok(value.to_string()),
        _ => Err(Error::Invalid(format!("Invalid child config at index {index}: missing non-empty '{what}'"))),
    }
}

/// Initialize a new project from a template (empty directory only).
pub fn init(paths: &Paths, template_ref: &str, target_dir: &Path, options: &InitOptions) -> Result<()> {
    let template_variables = merge_contexts(options.collected_variables.as_ref(), options.template_variables.as_ref());

    let parent_dir = find_parent_boilersync(target_dir);
    let inheritance_chain = get_template_inheritance_chain(paths, template_ref)?;
    let mut runtime_config = merge_runtime_config(&inheritance_chain)?;

    pull(
        paths,
        PullOptions {
            template_ref: Some(template_ref.to_string()),
            collected_variables: template_variables,
            allow_non_empty: false,
            include_starter: true,
            no_input: options.no_input,
            target_dir: Some(target_dir.to_path_buf()),
            recursive: false,
        },
    )?;

    if let Some(parent_dir) = &parent_dir {
        add_child_to_parent(target_dir, &parent_dir.join(METADATA_FILENAME))?;
        log::info!("📎 Registered as child project in parent: {}", parent_dir.display());
    }

    if options.monorepo {
        inject_monorepo_conversion_hook(&mut runtime_config, target_dir);
    }

    // The context used for conditions, hooks and children: the saved project
    // variables plus runtime options.
    let metadata = load_project_metadata(target_dir)?;
    let mut ctx = InterpolationContext::new();
    if let (Some(snake), pretty) =
        (metadata.get("name_snake").and_then(Value::as_str), metadata.get("name_pretty").and_then(Value::as_str))
    {
        ctx.set_project_names(snake, pretty);
    }
    if let Some(Value::Object(variables)) = metadata.get("variables") {
        ctx.set_collected_variables(variables);
    }
    // Children inherit the parent's saved variables (the original implementation
    // shared one interpolation context across the whole init), so a child
    // project can be pulled on its own later without re-collecting them.
    let parent_variables = ctx.get_collected_variables();
    let mut runtime_context = ctx.get_context();
    if let Some(extra) = &options.options {
        for (key, value) in extra {
            runtime_context.insert(key.clone(), value.clone());
        }
    }

    if options.run_hooks {
        run_hooks(&runtime_config.pre_init, "pre_init", target_dir, &runtime_context)?;
    }

    create_github_repo(&runtime_config.github, target_dir, &runtime_context)?;

    if options.run_children {
        for (index, child) in runtime_config.children.iter().enumerate() {
            let Value::Object(child) = child else {
                let kind = match child {
                    Value::Array(_) => "list",
                    Value::String(_) => "str",
                    Value::Number(_) => "int",
                    Value::Bool(_) => "bool",
                    _ => "NoneType",
                };
                return Err(Error::Invalid(format!(
                    "Invalid child config at index {index}: expected object, got {kind}"
                )));
            };
            if !evaluate_condition(child.get("condition"), &runtime_context)? {
                continue;
            }
            let child_template_name = string_config(child, "template", index, "template")?;
            let child_target_path = string_config(child, "path", index, "path")?;

            let rendered_child_path = render_string(&child_target_path, &runtime_context)?;
            let child_target_dir = target_dir.join(&rendered_child_path);
            std::fs::create_dir_all(&child_target_dir).map_err(|e| Error::io(&child_target_dir, e))?;

            let mut child_variables = Context::new();
            match child.get("variables") {
                None | Some(Value::Null) => {}
                Some(Value::Object(raw)) => {
                    for (key, value) in raw {
                        child_variables.insert(key.clone(), render_value(value, &runtime_context)?);
                    }
                }
                Some(other) if !truthy(Some(other)) => {}
                Some(_) => {
                    return Err(Error::Invalid(format!(
                        "Invalid child config for '{child_template_name}': 'variables' must be an object"
                    )));
                }
            }
            for key in ["name_snake", "name_pretty"] {
                if let Some(value) = child.get(key).filter(|v| !v.is_null()) {
                    child_variables
                        .insert(key.to_string(), Value::String(render_string(&py_str(value), &runtime_context)?));
                }
            }

            let child_template_ref = resolve_relative_ref(template_ref, &child_template_name, "child")?;
            let child_options = InitOptions {
                collected_variables: Some(parent_variables.clone()),
                template_variables: if child_variables.is_empty() { None } else { Some(child_variables) },
                options: options.options.clone(),
                no_input: options.no_input,
                run_hooks: options.run_hooks,
                run_children: options.run_children,
                monorepo: false,
            };
            init(paths, &child_template_ref, &child_target_dir, &child_options)?;
        }
    }

    if options.run_hooks {
        run_hooks(&runtime_config.post_init, "post_init", target_dir, &runtime_context)?;
    }
    Ok(())
}
