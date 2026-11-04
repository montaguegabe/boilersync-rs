//! Template inheritance chains (`extends` in `template.json`).

use crate::error::{Error, Result};
use crate::paths::Paths;
use crate::template_config::get_parent_template;
use crate::template_sources::{TemplateSource, resolve_template_source};

/// Resolve `other_ref` relative to `base_ref`: a ref with its own `#` source
/// is used as is; a bare subdir lives in the same source repository as the
/// base template.
pub fn resolve_relative_ref(base_ref: &str, other_ref: &str, what: &str) -> Result<String> {
    if other_ref.contains('#') {
        return Ok(other_ref.to_string());
    }
    let Some((repo_locator, _)) = base_ref.split_once('#') else {
        return Err(Error::Invalid(format!("Relative {what} templates require a source-qualified template ref.")));
    };
    let subdir = other_ref.trim().trim_start_matches('/');
    if subdir.is_empty() {
        return Err(Error::Invalid(format!("{} template ref cannot be empty.", capitalize(what))));
    }
    Ok(format!("{repo_locator}#{subdir}"))
}

fn capitalize(word: &str) -> String {
    let mut chars = word.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

fn chain(paths: &Paths, template_ref: &str, visited: &mut Vec<String>) -> Result<Vec<TemplateSource>> {
    let source = resolve_template_source(paths, template_ref, true)?;
    let identifier = source.identifier();
    if visited.contains(&identifier) {
        return Err(Error::Invalid(format!("Circular dependency detected in template inheritance: {template_ref}")));
    }
    visited.push(identifier);

    let Some(parent_ref) = get_parent_template(&source.template_dir) else {
        return Ok(vec![source]);
    };
    let resolved_parent_ref = resolve_relative_ref(template_ref, &parent_ref, "parent")?;
    let mut result = chain(paths, &resolved_parent_ref, visited)?;
    result.push(source);
    Ok(result)
}

/// The full inheritance chain for a template, from the root parent to the
/// template itself.
pub fn get_template_inheritance_chain(paths: &Paths, template_ref: &str) -> Result<Vec<TemplateSource>> {
    chain(paths, template_ref, &mut Vec::new())
}
