//! The variables available while rendering a template: project names in every
//! convention, template-provided values and values collected from the user.

use serde_json::Value;

use crate::jinja::Context;
use crate::names::{
    ProjectNames, create_project_names, default_project_snake_from_directory_name, normalize_to_snake, snake_to_kebab,
};

const NAME_VARIANT_SUFFIXES: [(&str, &str); 4] =
    [("_snake", "_kebab"), ("_kebab", "_snake"), ("_SNAKE", "_KEBAB"), ("_KEBAB", "_SNAKE")];

/// Context for template interpolation with project names and other variables.
#[derive(Debug, Clone, Default)]
pub struct InterpolationContext {
    names: Option<ProjectNames>,
    custom_vars: Context,
    collected_vars: Context,
}

impl InterpolationContext {
    pub fn new() -> Self {
        Self::default()
    }

    /// Set project names based on a directory name.
    pub fn set_project_name_from_directory(&mut self, directory: &std::path::Path) {
        let directory_name = directory.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        self.names = Some(create_project_names(&default_project_snake_from_directory_name(&directory_name), None));
    }

    /// Set project names from user input.
    pub fn set_project_names(&mut self, snake_name: &str, pretty_name: Option<&str>) {
        self.names = Some(create_project_names(snake_name, pretty_name));
    }

    /// Set a variable provided by the application rather than the user.
    pub fn set_custom_variable(&mut self, key: &str, value: Value) {
        self.custom_vars.insert(key.to_string(), value);
    }

    /// Set a variable collected from user input, template defaults or `--var`.
    pub fn set_collected_variable(&mut self, key: &str, value: Value) {
        self.collected_vars.insert(key.to_string(), value);
    }

    /// Set several collected variables at once.
    pub fn set_collected_variables(&mut self, variables: &Context) {
        for (key, value) in variables {
            self.collected_vars.insert(key.clone(), value.clone());
        }
    }

    /// The complete interpolation context: upper-case `NAME_*` variables for
    /// file and folder names, lower-case `name_*` variables for content,
    /// custom variables, collected variables and derived `_snake` / `_kebab`
    /// siblings.
    pub fn get_context(&self) -> Context {
        let mut context = Context::new();
        if let Some(names) = &self.names {
            for (key, value) in [
                ("NAME_SNAKE", &names.snake),
                ("NAME_PASCAL", &names.pascal),
                ("NAME_KEBAB", &names.kebab),
                ("NAME_CAMEL", &names.camel),
                ("NAME_PRETTY", &names.pretty),
                ("name_snake", &names.snake),
                ("name_pascal", &names.pascal),
                ("name_kebab", &names.kebab),
                ("name_camel", &names.camel),
                ("name_pretty", &names.pretty),
            ] {
                context.insert(key.to_string(), Value::String(value.clone()));
            }
        }
        for (key, value) in &self.custom_vars {
            context.insert(key.clone(), value.clone());
        }
        for (key, value) in &self.collected_vars {
            context.insert(key.clone(), value.clone());
        }
        add_name_variants(&mut context);
        context
    }

    pub fn has_variable(&self, key: &str) -> bool {
        self.get_context().contains_key(key)
    }

    pub fn names(&self) -> Option<&ProjectNames> {
        self.names.as_ref()
    }

    pub fn clear(&mut self) {
        self.names = None;
        self.custom_vars.clear();
        self.collected_vars.clear();
    }

    /// The variables collected from user input, defaults and `--var`.
    pub fn get_collected_variables(&self) -> Context {
        self.collected_vars.clone()
    }
}

/// Populate missing `*_snake` / `*_kebab` sibling variables from string values.
fn add_name_variants(context: &mut Context) {
    let mut derived = Context::new();
    for (key, value) in context.iter() {
        let Value::String(text) = value else { continue };
        for (source_suffix, target_suffix) in NAME_VARIANT_SUFFIXES {
            let Some(stem) = key.strip_suffix(source_suffix) else { continue };
            let sibling_key = format!("{stem}{target_suffix}");
            if context.contains_key(&sibling_key) || derived.contains_key(&sibling_key) {
                break;
            }
            let snake_value = normalize_to_snake(text);
            let sibling_value =
                if source_suffix.to_lowercase() == "_snake" { snake_to_kebab(&snake_value) } else { snake_value };
            derived.insert(sibling_key, Value::String(sibling_value));
            break;
        }
    }
    for (key, value) in derived {
        context.insert(key, value);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn generates_missing_snake_and_kebab_variants() {
        let mut ctx = InterpolationContext::new();
        ctx.set_custom_variable("api_package_name_snake", json!("demo_api"));
        ctx.set_custom_variable("worker_name_kebab", json!("demo-worker"));
        let context = ctx.get_context();
        assert_eq!(context["api_package_name_kebab"], "demo-api");
        assert_eq!(context["worker_name_snake"], "demo_worker");
    }

    #[test]
    fn does_not_override_explicit_variant() {
        let mut ctx = InterpolationContext::new();
        ctx.set_custom_variable("api_package_name_snake", json!("demo_api"));
        ctx.set_collected_variable("api_package_name_kebab", json!("custom-kebab"));
        assert_eq!(ctx.get_context()["api_package_name_kebab"], "custom-kebab");
    }

    #[test]
    fn exposes_upper_and_lower_case_names() {
        let mut ctx = InterpolationContext::new();
        ctx.set_project_names("my_app", Some("My App"));
        let context = ctx.get_context();
        assert_eq!(context["NAME_PASCAL"], "MyApp");
        assert_eq!(context["name_kebab"], "my-app");
        assert_eq!(context["name_pretty"], "My App");
        assert!(ctx.has_variable("NAME_CAMEL"));
    }
}
