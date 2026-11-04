mod common;

use boilersync_core::commands::check_pull::check_pull;
use boilersync_core::commands::pull::{PullOptions, pull};
use common::*;
use serde_json::json;

#[test]
fn init_records_template_commit() {
    let fx = fixture();
    let head = fx.write_template(
        "service-template",
        &[("README.md.boilersync", "Service $${name_snake}\n")],
        Some(json!({"skip_git": true})),
    );
    let target = fx.root.join("project");
    fx.init_project("service-template", &target, json!({"name_snake": "demo_service"}));

    let data = read_json(&target.join(".boilersync"));
    assert_eq!(data["template_commit"], head);
    assert_eq!(data["template"], fx.canonical_ref("service-template"));
    assert_eq!(read(&target.join("README.md")), "Service demo_service\n");
}

#[test]
fn pull_updates_template_commit_and_preserves_children() {
    let fx = fixture();
    fx.write_template(
        "workspace-template",
        &[("README.md.boilersync", "Workspace v1\n")],
        Some(json!({"skip_git": true})),
    );
    let target = fx.root.join("workspace");
    fx.init_project("workspace-template", &target, json!({"name_snake": "demo_workspace"}));

    let metadata_path = target.join(".boilersync");
    let mut data = read_json(&metadata_path);
    data["children"] = json!(["apps/frontend"]);
    write(&metadata_path, &serde_json::to_string_pretty(&data).unwrap());

    let head = fx.write_template("workspace-template", &[("README.md.boilersync", "Workspace v2\n")], None);
    pull(&fx.paths(&target), PullOptions { allow_non_empty: true, no_input: true, ..PullOptions::default() }).unwrap();

    let updated = read_json(&metadata_path);
    assert_eq!(updated["template_commit"], head);
    assert_eq!(updated["children"], json!(["apps/frontend"]));
    assert_eq!(read(&target.join("README.md")), "Workspace v2\n");
}

#[test]
fn check_pull_reports_due_when_template_repo_head_changes() {
    let fx = fixture();
    let recorded = fx.write_template(
        "status-template",
        &[("README.md.boilersync", "Status v1\n")],
        Some(json!({"skip_git": true})),
    );
    let target = fx.root.join("status-project");
    fx.init_project("status-template", &target, json!({"name_snake": "status_project"}));

    let current = fx.write_template("status-template", &[("README.md.boilersync", "Status v2\n")], None);
    let status = check_pull(&fx.paths(&target), Some(&target)).unwrap();

    assert!(status.due_for_pull);
    assert_eq!(status.recorded_template_commit.as_deref(), Some(recorded.as_str()));
    assert_eq!(status.current_template_commit, current);

    let fresh = check_pull(&fx.paths(&target), None).unwrap();
    assert!(fresh.due_for_pull);
}
