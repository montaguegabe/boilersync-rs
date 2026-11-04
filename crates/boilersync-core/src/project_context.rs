//! Everything `diff`, `push` and `pull-proposal` need to know about an
//! existing project: its metadata, resolved template chain and variables.

use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::error::Result;
use crate::inheritance::get_template_inheritance_chain;
use crate::interpolation::InterpolationContext;
use crate::jinja::Context;
use crate::paths::Paths;
use crate::project_metadata::load_project_metadata;
use crate::template_sources::{TemplateSource, resolve_source_from_boilersync};

#[derive(Debug, Clone)]
pub struct ProjectTemplateContext {
    pub project_dir: PathBuf,
    pub metadata: Context,
    pub template_source: TemplateSource,
    pub inheritance_chain: Vec<TemplateSource>,
    /// Canonical ref of the leaf template.
    pub template_ref: String,
    pub name_snake: Option<String>,
    pub name_pretty: Option<String>,
    pub variables: Context,
}

impl ProjectTemplateContext {
    pub fn leaf_template_source(&self) -> &TemplateSource {
        self.inheritance_chain.last().expect("inheritance chains are never empty")
    }

    /// An interpolation context seeded with the saved names and variables.
    pub fn interpolation_context(&self) -> InterpolationContext {
        let mut ctx = InterpolationContext::new();
        if let Some(snake) = &self.name_snake {
            ctx.set_project_names(snake, self.name_pretty.as_deref());
        }
        ctx.set_collected_variables(&self.variables);
        ctx
    }
}

fn string_field(metadata: &Context, key: &str) -> Option<String> {
    metadata.get(key).and_then(Value::as_str).map(str::to_string)
}

/// Load a project's `.boilersync` and resolve its template chain.
pub fn resolve_project_template_context(paths: &Paths, project_dir: &Path) -> Result<ProjectTemplateContext> {
    let metadata = load_project_metadata(project_dir)?;
    let template_source =
        resolve_source_from_boilersync(paths, metadata.get("template").and_then(Value::as_str), true)?;
    let inheritance_chain = get_template_inheritance_chain(paths, &template_source.canonical_ref())?;
    let template_ref = inheritance_chain.last().map(TemplateSource::canonical_ref).unwrap_or_default();
    let variables = match metadata.get("variables") {
        Some(Value::Object(map)) => map.clone(),
        _ => Context::new(),
    };
    Ok(ProjectTemplateContext {
        project_dir: project_dir.to_path_buf(),
        name_snake: string_field(&metadata, "name_snake"),
        name_pretty: string_field(&metadata, "name_pretty"),
        metadata,
        template_source,
        inheritance_chain,
        template_ref,
        variables,
    })
}
