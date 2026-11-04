//! Rendering (or plainly copying) a whole inheritance chain into a scratch
//! directory while remembering which template owns each output file.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::error::{Error, Result};
use crate::fsx;
use crate::interpolation::InterpolationContext;
use crate::jinja::Context;
use crate::template_processor::{
    TEMPLATE_CONFIG_FILENAME, apply_template_defaults, copy_and_process_template, process_file_extensions,
};
use crate::template_sources::TemplateSource;

/// Which template (and which file inside it) produced an output file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TemplateOwnership {
    pub template_ref: String,
    pub template_dir: PathBuf,
    pub template_repo_dir: PathBuf,
    pub source_relative_path: String,
}

/// Output path (posix, relative to the target root) -> owning template file.
pub type OwnershipMap = BTreeMap<String, TemplateOwnership>;

fn record_template_ownership(
    ownership_map: &mut OwnershipMap,
    target_root: &Path,
    template_source: &TemplateSource,
    source_path: &Path,
    target_path: &Path,
) {
    let target_key = fsx::posix(target_path.strip_prefix(target_root).unwrap_or(target_path));
    let source_relative = source_path.strip_prefix(&template_source.template_dir).unwrap_or(source_path);
    ownership_map.insert(
        target_key,
        TemplateOwnership {
            template_ref: template_source.ref_.clone(),
            template_dir: template_source.template_dir.clone(),
            template_repo_dir: template_source.local_repo_path.clone(),
            source_relative_path: fsx::posix(source_relative),
        },
    );
}

/// Render every template in the chain into `target_dir` with `ctx` (applying
/// each template's defaults first) and return the ownership of each file.
pub fn copy_rendered_template_chain(
    ctx: &mut InterpolationContext,
    inheritance_chain: &[TemplateSource],
    target_dir: &Path,
) -> Result<OwnershipMap> {
    let mut ownership_map = OwnershipMap::new();
    for template_source in inheritance_chain {
        apply_template_defaults(ctx, &template_source.template_dir)?;
        let context: Context = ctx.get_context();
        let mut record = |src: &Path, dst: &Path| -> Result<()> {
            record_template_ownership(&mut ownership_map, target_dir, template_source, src, dst);
            Ok(())
        };
        copy_and_process_template(&template_source.template_dir, target_dir, &context, false, Some(&mut record))?;
    }
    Ok(ownership_map)
}

fn copy_item_raw(
    src: &Path,
    dst: &Path,
    target_dir: &Path,
    template_source: &TemplateSource,
    ownership_map: &mut OwnershipMap,
) -> Result<()> {
    if src.is_file() {
        if src.file_name().is_some_and(|name| name == TEMPLATE_CONFIG_FILENAME) {
            return Ok(());
        }
        let name = dst.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let final_dst = dst.parent().unwrap_or(target_dir).join(process_file_extensions(&name));
        fsx::copy_file(src, &final_dst)?;
        record_template_ownership(ownership_map, target_dir, template_source, src, &final_dst);
    } else if src.is_dir() {
        std::fs::create_dir_all(dst).map_err(|e| Error::io(dst, e))?;
        for child in fsx::sorted_entries(src)? {
            let name = child.file_name().map(PathBuf::from).unwrap_or_default();
            copy_item_raw(&child, &dst.join(name), target_dir, template_source, ownership_map)?;
        }
    }
    Ok(())
}

/// Copy every template in the chain into `target_dir` without rendering
/// (template syntax stays intact; only the special extensions are stripped).
pub fn copy_template_chain_without_interpolation(
    inheritance_chain: &[TemplateSource],
    target_dir: &Path,
) -> Result<OwnershipMap> {
    let mut ownership_map = OwnershipMap::new();
    for template_source in inheritance_chain {
        for item in fsx::sorted_entries(&template_source.template_dir)? {
            let name = item.file_name().map(PathBuf::from).unwrap_or_default();
            copy_item_raw(&item, &target_dir.join(name), target_dir, template_source, &mut ownership_map)?;
        }
    }
    Ok(ownership_map)
}
