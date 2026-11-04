//! `boilersync push`: review project changes against the raw template chain in
//! a scratch git repository, then copy committed changes back into the
//! owning template directories.

use std::collections::BTreeSet;
use std::io::BufRead;
use std::path::{Path, PathBuf};
use std::process::Command;

use regex::Regex;

use crate::error::{Error, Result};
use crate::file_ignores::copy_project_files_to_directory;
use crate::fsx;
use crate::git;
use crate::jinja::{Context, py_str};
use crate::paths::Paths;
use crate::project_context::resolve_project_template_context;
use crate::template_sources::TemplateSource;
use crate::template_workspace::{OwnershipMap, TemplateOwnership, copy_template_chain_without_interpolation};

const BLOCK_SYNTAX_TOKENS: [&str; 4] = ["$${% block", "$${ super()", "$${% endblock", "$${% extends"];

/// True when a template file uses block inheritance, which cannot be
/// reconstructed from rendered output.
pub fn template_file_contains_block_syntax(file_path: &Path) -> bool {
    std::fs::read_to_string(file_path)
        .map(|content| BLOCK_SYNTAX_TOKENS.iter().any(|token| content.contains(token)))
        .unwrap_or(false)
}

fn name_vars_longest_first(context: &Context) -> Vec<(&String, String)> {
    let mut vars: Vec<(&String, String)> = context
        .iter()
        .filter(|(key, _)| key.starts_with("NAME_") && key.chars().all(|c| !c.is_lowercase()))
        .map(|(key, value)| (key, py_str(value)))
        .collect();
    vars.sort_by_key(|entry| std::cmp::Reverse(entry.1.len()));
    vars
}

/// Replace interpolated `NAME_*` values in a file or folder name with their
/// placeholders.
pub fn reverse_interpolate_path_name(path_name: &str, context: &Context) -> String {
    let mut result = path_name.to_string();
    for (key, value) in name_vars_longest_first(context) {
        if !value.is_empty() && result.contains(&value) {
            result = result.replace(&value, key);
        }
    }
    result
}

fn is_word_like(value: &str) -> bool {
    let stripped: String = value.chars().filter(|c| *c != '_' && *c != '-').collect();
    !stripped.is_empty() && stripped.chars().all(char::is_alphanumeric)
}

/// Replace interpolated values in file content with `$${variable}` syntax.
/// Files that are not UTF-8 are left alone.
pub fn reverse_interpolate_file_content(file_path: &Path, context: &Context) -> Result<()> {
    let Ok(mut content) = std::fs::read_to_string(file_path) else { return Ok(()) };

    let mut vars: Vec<(&String, String)> = context.iter().map(|(key, value)| (key, py_str(value))).collect();
    vars.sort_by_key(|entry| std::cmp::Reverse(entry.1.len()));

    for (key, value) in vars {
        if value.len() < 3
            || matches!(value.as_str(), "true" | "false" | "True" | "False" | "1" | "0")
            || value.chars().all(|c| c.is_ascii_digit())
        {
            continue;
        }
        let variable = if key.starts_with("NAME_") && key.chars().all(|c| !c.is_lowercase()) {
            key.to_lowercase()
        } else {
            key.clone()
        };
        let template_syntax = format!("$${{{variable}}}");
        if is_word_like(&value) {
            let pattern = Regex::new(&format!(r"\b{}\b", regex::escape(&value)))
                .map_err(|e| Error::Other(format!("invalid pattern for {value}: {e}")))?;
            content = pattern.replace_all(&content, regex::NoExpand(template_syntax.as_str())).into_owned();
        } else {
            content = content.replace(&value, &template_syntax);
        }
    }
    std::fs::write(file_path, content).map_err(|e| Error::io(file_path, e))
}

fn is_git_path(root: &Path, path: &Path) -> bool {
    path.file_name().is_some_and(|name| name == ".git")
        || fsx::posix(path.strip_prefix(root).unwrap_or(path)).contains(".git/")
}

fn collect_tree(dir: &Path, root: &Path, files: &mut Vec<PathBuf>, dirs: &mut Vec<PathBuf>) -> Result<()> {
    for entry in fsx::sorted_entries(dir)? {
        if is_git_path(root, &entry) {
            continue;
        }
        if entry.is_dir() && !entry.is_symlink() {
            dirs.push(entry.clone());
            collect_tree(&entry, root, files, dirs)?;
        } else if entry.is_file() {
            if entry.file_name().is_some_and(|name| name == ".boilersync") {
                continue;
            }
            files.push(entry);
        }
    }
    Ok(())
}

/// Reverse-interpolate every file (content and name) and directory name under
/// `target_dir`.
pub fn reverse_interpolate_project_files(target_dir: &Path, context: &Context) -> Result<()> {
    let mut files = Vec::new();
    let mut dirs = Vec::new();
    collect_tree(target_dir, target_dir, &mut files, &mut dirs)?;

    for file in files {
        reverse_interpolate_file_content(&file, context)?;
        let name = file.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let reversed = reverse_interpolate_path_name(&name, context);
        if reversed != name {
            let new_path = file.parent().unwrap_or(target_dir).join(reversed);
            if new_path.exists() {
                std::fs::remove_file(&new_path).map_err(|e| Error::io(&new_path, e))?;
            }
            std::fs::rename(&file, &new_path).map_err(|e| Error::io(&file, e))?;
        }
    }

    // Deepest directories first so renames never invalidate pending paths.
    dirs.sort_by_key(|dir| std::cmp::Reverse(dir.components().count()));
    for dir in dirs {
        let name = dir.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let reversed = reverse_interpolate_path_name(&name, context);
        if reversed != name {
            let new_path = dir.parent().unwrap_or(target_dir).join(reversed);
            if new_path.is_dir() {
                std::fs::remove_dir_all(&new_path).map_err(|e| Error::io(&new_path, e))?;
            }
            std::fs::rename(&dir, &new_path).map_err(|e| Error::io(&dir, e))?;
        }
    }
    Ok(())
}

fn ownership_or_leaf(ownership_map: &OwnershipMap, leaf: &TemplateSource, file_path: &str) -> TemplateOwnership {
    ownership_map.get(file_path).cloned().unwrap_or_else(|| TemplateOwnership {
        template_ref: leaf.ref_.clone(),
        template_dir: leaf.template_dir.clone(),
        template_repo_dir: leaf.local_repo_path.clone(),
        source_relative_path: file_path.to_string(),
    })
}

/// Files changed between the scratch repo's first commit and HEAD, plus any
/// explicitly added files.
fn changed_files_in_scratch_repo(temp_repo_dir: &Path, files_to_add: &[String]) -> Result<Vec<String>> {
    let roots = git::git(temp_repo_dir, &["rev-list", "--max-parents=0", "HEAD"])?;
    let initial = roots.lines().last().unwrap_or("").trim().to_string();
    let head = git::head_commit(temp_repo_dir)?;
    let mut changed: Vec<String> = if initial.is_empty() || initial == head {
        println!("📝 Only initial commit found, checking for any committed files...");
        git::git(temp_repo_dir, &["ls-tree", "-r", "--name-only", "HEAD"])?.lines().map(str::to_string).collect()
    } else {
        git::git(temp_repo_dir, &["diff", "--no-renames", "--name-only", &initial, &head])?
            .lines()
            .map(str::to_string)
            .collect()
    };
    for file in files_to_add {
        if !changed.contains(file) {
            changed.push(file.clone());
        }
    }
    Ok(changed)
}

/// Copy committed changes from the scratch repo back into the owning template
/// directories. Returns the updated files and the affected template repos.
pub fn copy_changed_files_to_template(
    temp_repo_dir: &Path,
    ownership_map: &OwnershipMap,
    leaf_template_source: &TemplateSource,
    files_to_add: &[String],
) -> Result<(Vec<String>, BTreeSet<PathBuf>)> {
    let changed_files = changed_files_in_scratch_repo(temp_repo_dir, files_to_add)?;
    if changed_files.is_empty() {
        println!("📝 No committed changes detected.");
        return Ok((Vec::new(), BTreeSet::new()));
    }

    println!("📋 Found {} file(s) to process:", changed_files.len());
    for file in &changed_files {
        if files_to_add.contains(file) {
            println!("  • {file} (added to template)");
        } else {
            println!("  • {file}");
        }
    }

    let mut blocked = Vec::new();
    for file in &changed_files {
        if !temp_repo_dir.join(file).exists() {
            continue;
        }
        let ownership = ownership_or_leaf(ownership_map, leaf_template_source, file);
        let source_template_file = ownership.template_dir.join(&ownership.source_relative_path);
        if source_template_file.exists() && template_file_contains_block_syntax(&source_template_file) {
            blocked.push(format!("- {file} (source: {})", source_template_file.display()));
        }
    }
    if !blocked.is_empty() {
        return Err(Error::Runtime(format!(
            "Refusing to push files derived from block-based template sources because BoilerSync cannot safely reconstruct the block structure:\n{}",
            blocked.join("\n")
        )));
    }

    let mut updated_files = Vec::new();
    let mut updated_repos = BTreeSet::new();
    for file in &changed_files {
        let source_file = temp_repo_dir.join(file);
        if !source_file.exists() {
            println!("⚠️  Skipping deleted file: {file}");
            continue;
        }
        let ownership = ownership_or_leaf(ownership_map, leaf_template_source, file);
        let final_target = ownership.template_dir.join(&ownership.source_relative_path);
        fsx::copy_file(&source_file, &final_target)?;
        updated_files.push(file.clone());
        updated_repos.insert(ownership.template_repo_dir.clone());
        let action = if files_to_add.contains(file) { "Added to template" } else { "Updated" };
        println!("✅ {action} ({} -> {}): {file}", ownership.template_ref, ownership.source_relative_path);
    }
    Ok((updated_files, updated_repos))
}

/// Copy extra project files into the scratch repo, preferring an existing
/// `.boilersync`-suffixed template counterpart as the destination.
pub fn copy_additional_files_to_temp(root_dir: &Path, temp_dir: &Path, files_to_add: &[String]) -> Result<()> {
    for file in files_to_add {
        let source = root_dir.join(file);
        if !source.exists() {
            println!("⚠️  File not found in project: {file}");
            continue;
        }
        let boilersync_target = temp_dir.join(format!("{file}.boilersync"));
        let (target, kind) = if boilersync_target.exists() {
            (boilersync_target, ".boilersync version")
        } else {
            (temp_dir.join(file), "regular file")
        };
        fsx::copy_file(&source, &target)?;
        println!("📋 Copied additional file ({kind}): {file}");
    }
    Ok(())
}

fn open_in_github_desktop(path: &Path) -> Result<()> {
    let status = Command::new("github")
        .arg(path)
        .status()
        .map_err(|e| Error::Runtime(format!("Could not run the GitHub Desktop command line tool 'github': {e}")))?;
    if !status.success() {
        return Err(Error::Runtime(format!("'github {}' exited with {status}", path.display())));
    }
    Ok(())
}

fn wait_for_enter() -> Result<()> {
    let mut line = String::new();
    std::io::stdin().lock().read_line(&mut line)?;
    Ok(())
}

/// Show the project's differences from its template chain in GitHub Desktop,
/// wait for the user to commit the changes worth keeping, then copy those
/// committed changes back into the template source repositories.
pub fn push(paths: &Paths, files_to_add: &[String]) -> Result<()> {
    let root_dir = paths.root_dir()?;
    let project_context = resolve_project_template_context(paths, &root_dir).map_err(|e| {
        Error::NotFound(format!(
            "Could not read template metadata from {}: {e}",
            root_dir.join(crate::paths::METADATA_FILENAME).display()
        ))
    })?;
    let leaf = project_context.leaf_template_source().clone();
    let template_ref = project_context.template_ref.clone();

    println!("🔍 Creating diff for template '{template_ref}'...");
    let project_temp_dir = std::env::temp_dir()
        .join(format!("boilersync-diff-{}", fsx::path_hash(&fsx::resolve(&root_dir))))
        .join("project");
    fsx::recreate_dir(&project_temp_dir)?;

    println!("📦 Copying fresh template inheritance chain without interpolation...");
    if !leaf.template_dir.exists() {
        return Err(Error::NotFound(format!("Template '{template_ref}' not found at {}", leaf.template_dir.display())));
    }
    let ownership_map =
        copy_template_chain_without_interpolation(&project_context.inheritance_chain, &project_temp_dir)?;

    println!("🔧 Setting up git repository...");
    git::init_scratch_repo(&project_temp_dir)?;
    git::commit_all(&project_temp_dir, &format!("Fresh template: {template_ref}"))?;

    println!("📋 Copying current project files...");
    copy_project_files_to_directory(&root_dir, &project_temp_dir, &[".boilersync", ".git"])?;
    if !files_to_add.is_empty() {
        println!("📋 Copying additional files to add to template...");
        copy_additional_files_to_temp(&root_dir, &project_temp_dir, files_to_add)?;
    }

    println!("🔄 Reverse interpolating project files...");
    let context = project_context.interpolation_context().get_context();
    reverse_interpolate_project_files(&project_temp_dir, &context)?;

    println!("🚀 Opening in GitHub Desktop...");
    open_in_github_desktop(&project_temp_dir)?;
    println!("📂 Persistent comparison directory: {}", project_temp_dir.display());
    println!("💡 This directory will be reused for future diffs of this project.");
    println!("⚠️  IMPORTANT: Please commit any changes you want to push back to the template!");
    println!("⏳ Press Enter when you're done reviewing and committing changes...");
    wait_for_enter()?;

    println!("\n🔄 Performing hard reset to last commit...");
    match git::git(&project_temp_dir, &["reset", "--hard", "HEAD"]) {
        Ok(_) => println!("✅ Reset to last commit completed"),
        Err(err) => {
            println!("❌ Failed to reset to last commit: {err}");
            println!("⚠️  Proceeding anyway, but uncommitted changes may be included");
        }
    }

    println!("\n🔄 Processing committed changes to update template...");
    let (updated_files, updated_repos) =
        copy_changed_files_to_template(&project_temp_dir, &ownership_map, &leaf, files_to_add)?;
    if updated_files.is_empty() {
        println!("\n📝 No files were updated in the template.");
        return Ok(());
    }
    println!("\n✅ Successfully updated {} file(s) in template '{template_ref}':", updated_files.len());
    for file in &updated_files {
        println!("  • {file}");
    }
    for repo_dir in &updated_repos {
        println!("\n📁 Updated template repository: {}", repo_dir.display());
    }
    for repo_dir in &updated_repos {
        println!("🚀 Opening template source repository in GitHub Desktop: {}", repo_dir.display());
        open_in_github_desktop(repo_dir)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{Value, json};

    fn ctx(value: Value) -> Context {
        match value {
            Value::Object(map) => map,
            _ => unreachable!(),
        }
    }

    #[test]
    fn reverses_path_names_longest_value_first() {
        let context = ctx(json!({"NAME_SNAKE": "demo", "NAME_KEBAB": "demo-app", "name_snake": "demo"}));
        assert_eq!(reverse_interpolate_path_name("demo-app_demo.txt", &context), "NAME_KEBAB_NAME_SNAKE.txt");
        assert_eq!(reverse_interpolate_path_name("plain.txt", &context), "plain.txt");
    }

    #[test]
    fn reverses_file_content_with_word_boundaries() {
        let tmp = tempfile::tempdir().unwrap();
        let file = tmp.path().join("a.txt");
        std::fs::write(&file, "demo_app demo_apple true 42 Jane Doe\n").unwrap();
        let context = ctx(json!({"NAME_SNAKE": "demo_app", "flag": true, "n": 42, "author_name": "Jane Doe"}));
        reverse_interpolate_file_content(&file, &context).unwrap();
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "$${name_snake} demo_apple true 42 $${author_name}\n");
    }
}
