//! Rendering a template directory into a project: path placeholders, the
//! `.starter` / `.boilersync` extensions, variable discovery and defaults.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use regex::Regex;
use serde_json::Value;

use crate::error::{Error, Result};
use crate::fsx;
use crate::interpolation::InterpolationContext;
use crate::jinja::{Context, py_str, render_file_in_place, render_value, undeclared_variables};
use crate::template_config::get_template_config;
use crate::variables::collect_missing_variables;

/// Files named like this hold template metadata and are never copied.
pub const TEMPLATE_CONFIG_FILENAME: &str = "template.json";

fn path_name_variable_pattern() -> &'static Regex {
    static PATTERN: std::sync::OnceLock<Regex> = std::sync::OnceLock::new();
    PATTERN.get_or_init(|| Regex::new(r"\b(NAME_[A-Z0-9_]+)\b").expect("valid regex"))
}

/// Interpolate upper-case `NAME_*` variables in a file or folder name by plain
/// substitution. No other variables apply to paths.
pub fn interpolate_path_name(path_name: &str, context: &Context) -> String {
    let mut result = path_name.to_string();
    for (key, value) in context {
        if key.starts_with("NAME_") && key.chars().all(|c| !c.is_lowercase()) {
            result = result.replace(key, &py_str(value));
        }
    }
    result
}

/// Drop a trailing `.boilersync` extension.
pub fn remove_boilersync_extension(file_name: &str) -> String {
    file_name.strip_suffix(".boilersync").unwrap_or(file_name).to_string()
}

/// Drop `.starter` when it is the first extension (`cli.starter.py` -> `cli.py`).
pub fn remove_starter_extension(file_name: &str) -> String {
    let parts: Vec<&str> = file_name.split('.').collect();
    if parts.len() > 1 && parts[1] == "starter" {
        if parts.len() > 2 {
            return format!("{}.{}", parts[0], parts[2..].join("."));
        }
        return parts[0].to_string();
    }
    file_name.to_string()
}

/// Remove both special extensions from a template file name.
pub fn process_file_extensions(file_name: &str) -> String {
    remove_boilersync_extension(&remove_starter_extension(file_name))
}

/// True when the file has `.starter` as its first extension.
pub fn is_starter_file(path: &Path) -> bool {
    let name = path.file_name().map(|n| n.to_string_lossy()).unwrap_or_default();
    let parts: Vec<&str> = name.split('.').collect();
    parts.len() > 1 && parts[1] == "starter"
}

/// Lower-case names of the `NAME_*` placeholders used in a relative path.
pub fn extract_variables_from_template_path(path: &Path) -> HashSet<String> {
    let mut variables = HashSet::new();
    for segment in path.components() {
        let segment = segment.as_os_str().to_string_lossy();
        for capture in path_name_variable_pattern().captures_iter(&segment) {
            variables.insert(capture[1].to_lowercase());
        }
    }
    variables
}

fn scan_item(path: &Path, source_dir: &Path, exclude_starter: bool, variables: &mut HashSet<String>) -> Result<()> {
    let relative = path.strip_prefix(source_dir).unwrap_or(path);
    variables.extend(extract_variables_from_template_path(relative));
    if path.is_dir() {
        for child in fsx::sorted_entries(path)? {
            scan_item(&child, source_dir, exclude_starter, variables)?;
        }
    } else if path.is_file() && !(exclude_starter && is_starter_file(path)) {
        // Binary files (not UTF-8) are skipped.
        if let Ok(content) = std::fs::read_to_string(path) {
            variables.extend(undeclared_variables(&content));
        }
    }
    Ok(())
}

/// Every variable the template reads, from path placeholders and file content.
/// With `exclude_starter`, starter files contribute only path placeholders.
pub fn scan_template_for_variables(source_dir: &Path, exclude_starter: bool) -> Result<HashSet<String>> {
    let mut variables = HashSet::new();
    for item in fsx::sorted_entries(source_dir)? {
        scan_item(&item, source_dir, exclude_starter, &mut variables)?;
    }
    Ok(variables)
}

/// Apply `template.json` `defaults` that the context does not already define.
/// String defaults are rendered with the context accumulated so far.
pub fn apply_template_defaults(ctx: &mut InterpolationContext, template_dir: &Path) -> Result<()> {
    let config = get_template_config(template_dir);
    let Some(defaults) = config.get("defaults") else { return Ok(()) };
    if !crate::conditions::truthy(Some(defaults)) {
        return Ok(());
    }
    let Value::Object(defaults) = defaults else {
        return Err(Error::Invalid(format!(
            "Template '{}' has invalid 'defaults' config: expected object",
            template_dir.display()
        )));
    };
    for (key, value) in defaults {
        if ctx.has_variable(key) {
            continue;
        }
        let context = ctx.get_context();
        ctx.set_collected_variable(key, render_value(value, &context)?);
    }
    Ok(())
}

/// Callback receiving the template source file and the final rendered
/// destination file after interpolation and extension cleanup.
pub type OnFileCopied<'a> = &'a mut dyn FnMut(&Path, &Path) -> Result<()>;

fn process_item(
    src: &Path,
    dst: &Path,
    context: &Context,
    exclude_starter: bool,
    on_file_copied: &mut Option<OnFileCopied<'_>>,
) -> Result<()> {
    let dst_name = dst.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let dst_parent = dst.parent().map(Path::to_path_buf).unwrap_or_default();
    if src.is_file() {
        if src.file_name().is_some_and(|name| name == TEMPLATE_CONFIG_FILENAME) {
            return Ok(());
        }
        if exclude_starter && is_starter_file(src) {
            return Ok(());
        }
        let final_name = process_file_extensions(&interpolate_path_name(&dst_name, context));
        let final_dst = dst_parent.join(final_name);
        fsx::copy_file(src, &final_dst)?;
        render_file_in_place(&final_dst, context)?;
        if let Some(callback) = on_file_copied {
            callback(src, &final_dst)?;
        }
    } else if src.is_dir() {
        let final_dst = dst_parent.join(interpolate_path_name(&dst_name, context));
        std::fs::create_dir_all(&final_dst).map_err(|e| Error::io(&final_dst, e))?;
        for child in fsx::sorted_entries(src)? {
            let child_name = child.file_name().map(PathBuf::from).unwrap_or_default();
            process_item(&child, &final_dst.join(child_name), context, exclude_starter, on_file_copied)?;
        }
    }
    Ok(())
}

/// Copy a template directory into `target_dir`, interpolating path names and
/// rendering file content.
pub fn copy_and_process_template(
    source_dir: &Path,
    target_dir: &Path,
    context: &Context,
    exclude_starter: bool,
    mut on_file_copied: Option<OnFileCopied<'_>>,
) -> Result<()> {
    for item in fsx::sorted_entries(source_dir)? {
        let name = item.file_name().map(PathBuf::from).unwrap_or_default();
        process_item(&item, &target_dir.join(name), context, exclude_starter, &mut on_file_copied)?;
    }
    Ok(())
}

/// Render one template directory into `target_dir` with the current context,
/// applying its defaults and collecting missing variables first.
pub fn process_template_directory(
    ctx: &mut InterpolationContext,
    template_dir: &Path,
    target_dir: &Path,
    no_input: bool,
    exclude_starter: bool,
) -> Result<()> {
    apply_template_defaults(ctx, template_dir)?;
    let template_variables = scan_template_for_variables(template_dir, exclude_starter)?;
    collect_missing_variables(ctx, &template_variables, no_input)?;
    let context = ctx.get_context();
    copy_and_process_template(template_dir, target_dir, &context, exclude_starter, None)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn context(value: Value) -> Context {
        match value {
            Value::Object(map) => map,
            _ => unreachable!(),
        }
    }

    #[test]
    fn interpolates_only_upper_case_name_variables_in_paths() {
        let ctx = context(json!({
            "NAME_SNAKE": "my_project", "NAME_PASCAL": "MyProject", "NAME_KEBAB": "my-project",
            "publish": "1", "debug": "true", "author_name": "John Doe", "name_snake": "my_project"
        }));
        assert_eq!(interpolate_path_name("NAME_SNAKE_service.py", &ctx), "my_project_service.py");
        assert_eq!(interpolate_path_name("NAME_PASCAL/", &ctx), "MyProject/");
        assert_eq!(interpolate_path_name("docs/NAME_KEBAB-guide.md", &ctx), "docs/my-project-guide.md");
        assert_eq!(interpolate_path_name("publish.yml", &ctx), "publish.yml");
        assert_eq!(interpolate_path_name("debug.config", &ctx), "debug.config");
        assert_eq!(interpolate_path_name("author_name.txt", &ctx), "author_name.txt");
        assert_eq!(interpolate_path_name("name_snake_test.py", &ctx), "name_snake_test.py");
        assert_eq!(interpolate_path_name("NAME_SNAKE_file.py", &Context::new()), "NAME_SNAKE_file.py");
    }

    #[test]
    fn strips_special_extensions() {
        assert_eq!(process_file_extensions("cli.starter.py"), "cli.py");
        assert_eq!(process_file_extensions("README.starter.md"), "README.md");
        assert_eq!(process_file_extensions("Cargo.starter.toml"), "Cargo.toml");
        assert_eq!(process_file_extensions("pyproject.toml.boilersync"), "pyproject.toml");
        assert_eq!(process_file_extensions("main.starter.rs.boilersync"), "main.rs");
        assert_eq!(process_file_extensions("plain.txt"), "plain.txt");
        assert_eq!(remove_starter_extension("name.starter"), "name");
        assert!(is_starter_file(Path::new("a/b/x.starter.md")));
        assert!(!is_starter_file(Path::new("x.md.starter")));
    }

    #[test]
    fn extracts_path_placeholders_as_lower_case_variables() {
        let found = extract_variables_from_template_path(Path::new("cli/NAME_SNAKE/NAME_PASCAL.py"));
        let mut found: Vec<_> = found.into_iter().collect();
        found.sort();
        assert_eq!(found, ["name_pascal", "name_snake"]);
    }
}
