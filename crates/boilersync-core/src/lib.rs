//! boilersync-core: scaffold projects from templates and keep templates alive
//! as projects evolve.
//!
//! The CLI in `boilersync-cli` is a thin layer over this crate. Behaviour and
//! file formats (`template.json`, `.boilersync`, the `$${...}` template syntax,
//! the `~/.boilersync/templates` cache) match the original Python
//! implementation so existing templates and projects keep working.

pub mod commands;
pub mod comparison;
pub mod conditions;
pub mod error;
pub mod file_ignores;
pub mod fsx;
pub mod git;
pub mod inheritance;
pub mod interpolation;
pub mod jinja;
pub mod logging;
pub mod names;
pub mod paths;
pub mod project_context;
pub mod project_metadata;
pub mod prompt;
pub mod template_config;
pub mod template_processor;
pub mod template_sources;
pub mod template_workspace;
pub mod variables;

pub use error::{Error, Result};
pub use paths::Paths;
