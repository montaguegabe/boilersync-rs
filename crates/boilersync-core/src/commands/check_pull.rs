//! `boilersync check-pull`: is the project behind its template repository?

use std::path::Path;

use crate::error::Result;
use crate::paths::Paths;
use crate::project_metadata::{PullCheckStatus, get_pull_check_status};

pub fn check_pull(paths: &Paths, project_dir: Option<&Path>) -> Result<PullCheckStatus> {
    get_pull_check_status(paths, project_dir)
}

/// The human-readable report printed by the CLI.
pub fn status_report(status: &PullCheckStatus) -> String {
    format!(
        "Template: {}\nRecorded template commit: {}\nCurrent template commit: {}\nDue for pull: {}",
        status.template_ref,
        status.recorded_template_commit.as_deref().unwrap_or("missing"),
        status.current_template_commit,
        if status.due_for_pull { "yes" } else { "no" }
    )
}
