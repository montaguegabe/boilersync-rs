mod common;

use boilersync_core::commands::diff::{DiffOptions, OutputMode, diff};
use common::*;
use serde_json::{Value, json};

fn run_diff(
    fx: &Fixture,
    target: &std::path::Path,
    mode: OutputMode,
    include_starter: bool,
    children: bool,
    recursive: bool,
) -> String {
    diff(&fx.paths(target), &DiffOptions { include_starter, include_children: children, recursive, mode }).unwrap()
}

fn run_diff_json(fx: &Fixture, target: &std::path::Path, children: bool, recursive: bool) -> Value {
    serde_json::from_str(&run_diff(fx, target, OutputMode::Json, false, children, recursive)).unwrap()
}

#[test]
fn name_status_reports_inherited_parent_file_changes() {
    let fx = fixture();
    fx.write_template("parent", &[("common.txt.boilersync", "Parent $${name_snake}\n")], None);
    fx.write_template(
        "child",
        &[("child.txt.boilersync", "Child $${name_snake}\n")],
        Some(json!({"extends": "parent", "skip_git": true})),
    );
    let target = fx.root.join("project");
    fx.init_project("child", &target, json!({"name_snake": "demo_project"}));
    write(&target.join("common.txt"), "Updated demo_project\n");

    let output = run_diff(&fx, &target, OutputMode::NameStatus, false, false, false);
    assert!(output.contains("M\tcommon.txt"), "{output}");
}

#[test]
fn diff_compares_against_rendered_template() {
    let fx = fixture();
    fx.write_template(
        "rendered-template",
        &[("app.py.boilersync", "class $${name_pascal}Config:\n    name = \"$${name_snake}\"\n")],
        Some(json!({"skip_git": true})),
    );
    let target = fx.root.join("rendered-project");
    fx.init_project("rendered-template", &target, json!({"name_snake": "rendered_project"}));

    let clean = run_diff(&fx, &target, OutputMode::NameStatus, false, false, false);
    assert!(clean.contains("No divergence from template."), "{clean}");

    write(&target.join("app.py"), "class RenderedProjectConfig:\n    name = \"changed_project\"\n");
    let patch = run_diff(&fx, &target, OutputMode::Patch, false, false, false);
    assert!(patch.contains("name = \"rendered_project\""), "{patch}");
    assert!(patch.contains("name = \"changed_project\""), "{patch}");
    assert!(!patch.contains("$${name_snake}"), "{patch}");

    let stat = run_diff(&fx, &target, OutputMode::Stat, false, false, false);
    assert!(stat.contains("app.py |"), "{stat}");
}

#[test]
fn diff_ignores_dependency_directories() {
    let fx = fixture();
    fx.write_template(
        "node-template",
        &[("package.json.boilersync", "{\"name\": \"$${name_kebab}\"}\n")],
        Some(json!({"skip_git": true})),
    );
    let target = fx.root.join("node-project");
    fx.init_project("node-template", &target, json!({"name_snake": "node_project"}));
    write(&target.join("node_modules/left-pad/index.js"), "module.exports = null;\n");

    let output = run_diff(&fx, &target, OutputMode::NameStatus, false, false, false);
    assert!(output.contains("No divergence from template."), "{output}");
}

#[test]
fn diff_respects_project_gitignore() {
    let fx = fixture();
    fx.write_template(
        "ignored-template",
        &[(".gitignore.boilersync", "ignored.txt\n"), ("package.json.boilersync", "{\"name\": \"$${name_kebab}\"}\n")],
        Some(json!({"skip_git": true})),
    );
    let target = fx.root.join("ignored-project");
    fx.init_project("ignored-template", &target, json!({"name_snake": "ignored_project"}));
    git(&target, &["init", "-q"]);
    write(&target.join("ignored.txt"), "ignored local output\n");

    let output = run_diff(&fx, &target, OutputMode::NameStatus, false, false, false);
    assert!(output.contains("No divergence from template."), "{output}");
}

#[cfg(unix)]
#[test]
fn diff_ignores_claude_agents_compatibility_symlink() {
    let fx = fixture();
    fx.write_template(
        "agents-template",
        &[("AGENTS.md.boilersync", "$${name_pretty} instructions\n")],
        Some(json!({"skip_git": true})),
    );
    let target = fx.root.join("agents-project");
    fx.init_project(
        "agents-template",
        &target,
        json!({"name_snake": "agents_project", "name_pretty": "Agents Project"}),
    );
    std::os::unix::fs::symlink("AGENTS.md", target.join("CLAUDE.md")).unwrap();

    let output = run_diff(&fx, &target, OutputMode::NameStatus, false, false, false);
    assert!(output.contains("No divergence from template."), "{output}");
}

#[test]
fn starter_files_are_excluded_by_default_and_included_on_request() {
    let fx = fixture();
    fx.write_template(
        "starter-template",
        &[("README.starter.md", "Starter $${name_snake}\n")],
        Some(json!({"skip_git": true})),
    );
    let target = fx.root.join("starter-project");
    fx.init_project("starter-template", &target, json!({"name_snake": "starter_project"}));
    write(&target.join("README.md"), "Project-owned README\n");

    let default = run_diff(&fx, &target, OutputMode::NameStatus, false, false, false);
    assert!(default.contains("No divergence from template."), "{default}");
    let included = run_diff(&fx, &target, OutputMode::NameStatus, true, false, false);
    assert!(included.contains("M\tREADME.md"), "{included}");
}

#[test]
fn unregistered_extra_folder_remains_parent_drift() {
    let fx = fixture();
    fx.write_template(
        "parent-template",
        &[("parent.txt.boilersync", "Parent $${name_snake}\n")],
        Some(json!({"skip_git": true})),
    );
    let parent = fx.root.join("parent-project");
    fx.init_project("parent-template", &parent, json!({"name_snake": "parent_project"}));
    write(&parent.join("extra/owned.txt"), "Project-owned extra file\n");

    let payload = run_diff_json(&fx, &parent, false, false);
    assert_eq!(payload["projects"][0]["changed_count"], 1);
    assert_eq!(payload["projects"][0]["changed_files"][0], json!({"status": "A", "path": "extra/owned.txt"}));
    assert_eq!(payload["include_starter"], false);
}

#[test]
fn children_option_reports_direct_child_changes_and_parent_excludes_registered_children() {
    let fx = fixture();
    fx.write_template(
        "parent-template",
        &[("parent.txt.boilersync", "Parent $${name_snake}\n")],
        Some(json!({"skip_git": true})),
    );
    fx.write_template(
        "child-template",
        &[("child.txt.boilersync", "Child $${name_snake}\n")],
        Some(json!({"skip_git": true})),
    );
    let parent = fx.root.join("parent-project");
    let child = parent.join("child");
    fx.init_project("parent-template", &parent, json!({"name_snake": "parent_project"}));
    fx.init_project("child-template", &child, json!({"name_snake": "child_project"}));
    assert_eq!(read_json(&parent.join(".boilersync"))["children"], json!(["child"]));
    write(&child.join("child.txt"), "Changed child_project\n");

    let with_children = run_diff_json(&fx, &parent, true, false);
    assert_eq!(with_children["projects"].as_array().unwrap().len(), 2);
    assert_eq!(with_children["projects"][0]["changed_count"], 0);
    assert_eq!(with_children["projects"][1]["changed_files"][0]["path"], "child.txt");

    let alone = run_diff_json(&fx, &parent, false, false);
    assert_eq!(alone["projects"].as_array().unwrap().len(), 1);
    assert_eq!(alone["projects"][0]["changed_count"], 0);
}

#[test]
fn recursive_includes_descendants_but_children_does_not() {
    let fx = fixture();
    for (name, file) in
        [("parent-template", "parent.txt"), ("child-template", "child.txt"), ("grandchild-template", "grandchild.txt")]
    {
        fx.write_template(
            name,
            &[(&format!("{file}.boilersync"), "X $${name_snake}\n")],
            Some(json!({"skip_git": true})),
        );
    }
    let parent = fx.root.join("parent-project");
    let child = parent.join("child");
    let grandchild = child.join("grandchild");
    fx.init_project("parent-template", &parent, json!({"name_snake": "parent_project"}));
    fx.init_project("child-template", &child, json!({"name_snake": "child_project"}));
    fx.init_project("grandchild-template", &grandchild, json!({"name_snake": "grandchild_project"}));
    write(&grandchild.join("grandchild.txt"), "Changed grandchild_project\n");

    let children = run_diff_json(&fx, &parent, true, false);
    let counts: Vec<u64> =
        children["projects"].as_array().unwrap().iter().map(|p| p["changed_count"].as_u64().unwrap()).collect();
    assert_eq!(counts, [0, 0]);

    let recursive = run_diff_json(&fx, &parent, false, true);
    let counts: Vec<u64> =
        recursive["projects"].as_array().unwrap().iter().map(|p| p["changed_count"].as_u64().unwrap()).collect();
    assert_eq!(counts, [0, 0, 1]);
    assert_eq!(recursive["projects"][2]["changed_files"][0]["path"], "grandchild.txt");
}

#[test]
fn child_errors_are_reported_per_project() {
    let fx = fixture();
    fx.write_template("parent-template", &[("parent.txt.boilersync", "Parent\n")], Some(json!({"skip_git": true})));
    let parent = fx.root.join("parent-project");
    fx.init_project("parent-template", &parent, json!({"name_snake": "parent_project"}));
    let broken = parent.join("broken");
    write(&broken.join(".boilersync"), "{\"template\": \"acme/templates#missing\"}\n");
    let mut data = read_json(&parent.join(".boilersync"));
    data["children"] = json!(["broken"]);
    write(&parent.join(".boilersync"), &data.to_string());

    let payload = run_diff_json(&fx, &parent, true, false);
    assert_eq!(payload["projects"][0]["changed_count"], 0);
    assert!(payload["projects"][1]["error"].as_str().unwrap().contains("missing"));
}
