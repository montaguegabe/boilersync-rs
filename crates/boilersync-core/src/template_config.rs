//! Reading `template.json`.

use std::path::Path;

use serde_json::Value;

use crate::jinja::Context;
use crate::template_sources::TemplateSource;

/// The `template.json` of a template, or an empty object when it is missing
/// or not valid JSON.
pub fn get_template_config(template_dir: &Path) -> Context {
    let path = template_dir.join("template.json");
    let Ok(text) = std::fs::read_to_string(&path) else { return Context::new() };
    match serde_json::from_str::<Value>(&text) {
        Ok(Value::Object(map)) => map,
        _ => Context::new(),
    }
}

fn non_empty_string(value: Option<&Value>) -> Option<String> {
    value.and_then(Value::as_str).filter(|text| !text.is_empty()).map(str::to_string)
}

/// The parent template ref from `extends` (or the legacy `parent` key).
pub fn get_parent_template(template_dir: &Path) -> Option<String> {
    let config = get_template_config(template_dir);
    non_empty_string(config.get("extends")).or_else(|| non_empty_string(config.get("parent")))
}

/// True when any template in the chain sets `skip_git`.
pub fn should_skip_git(inheritance_chain: &[TemplateSource]) -> bool {
    inheritance_chain
        .iter()
        .any(|source| crate::conditions::truthy(get_template_config(&source.template_dir).get("skip_git")))
}
