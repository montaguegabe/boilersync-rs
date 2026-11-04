mod common;

use boilersync_core::commands::templates::{
    get_template_details, init_templates, list_local_templates, list_template_sources,
};
use boilersync_core::error::Error;
use boilersync_core::paths::{Paths, add_child_to_parent, children_from_boilersync, find_parent_boilersync};
use common::*;
use serde_json::json;

fn refs(fx: &Fixture) -> Vec<String> {
    list_local_templates(&fx.paths(&fx.root)).unwrap().into_iter().map(|t| t.template_ref).collect()
}

#[test]
fn list_local_templates_requires_template_json_and_stops_at_template_roots() {
    let fx = fixture();
    fx.write_uncommitted_template(
        "python/service",
        &[("README.starter.md", "nested"), ("src/main.py", "print('hello')\n")],
        Some(json!({})),
    );
    fx.write_uncommitted_template(
        "with-metadata",
        &[("README.starter.md", "m"), ("src/main.py", "x\n")],
        Some(json!({})),
    );
    fx.write_uncommitted_template("plain-template", &[("README.starter.md", "p"), ("src/index.ts", "x\n")], None);
    fx.write_uncommitted_template("node_modules/pkg", &[("x", "x")], Some(json!({})));
    fx.write_uncommitted_template(".hidden", &[("x", "x")], Some(json!({})));

    let found = refs(&fx);
    assert_eq!(found, ["acme/templates#python/service", "acme/templates#with-metadata"]);
    let templates = list_local_templates(&fx.paths(&fx.root)).unwrap();
    assert_eq!(templates[0].template_dir, fx.repo_dir().join("python/service"));
    assert_eq!(templates[0].display_name, "acme/templates#python/service");
}

#[test]
fn list_template_sources_reports_duplicate_remotes() {
    let fx = fixture();
    for repo in ["old-name", "new-name"] {
        let repo_dir = fx.template_root.join("acme").join(repo);
        std::fs::create_dir_all(&repo_dir).unwrap();
        git(&repo_dir, &["init", "-q"]);
        git(&repo_dir, &["remote", "add", "origin", "https://github.com/acme/templates.git"]);
        write(&repo_dir.join("service/template.json"), "{}");
    }
    let report = list_template_sources(&fx.paths(&fx.root)).unwrap();
    assert_eq!(report.source_count, 2);
    assert_eq!(report.template_root_dir, fx.template_root);
    assert_eq!(report.sources[0].template_count, 1);
    assert_eq!(report.sources[0].remote_url.as_deref(), Some("https://github.com/acme/templates.git"));
    assert_eq!(report.duplicate_remotes.len(), 1);
    assert_eq!(report.duplicate_remotes[0].remote_url, "https://github.com/acme/templates.git");
    assert_eq!(
        report.duplicate_remotes[0].paths,
        [fx.template_root.join("acme/new-name"), fx.template_root.join("acme/old-name")]
    );
}

#[test]
fn templates_init_rejects_duplicates_bad_input_and_non_github_urls() {
    let fx = fixture();
    let existing = fx.template_root.join("acme").join("current-templates");
    std::fs::create_dir_all(&existing).unwrap();
    git(&existing, &["init", "-q"]);
    git(&existing, &["remote", "add", "origin", "https://github.com/acme/old-templates.git"]);
    let paths = fx.paths(&fx.root);

    match init_templates(&paths, Some("acme/old-templates"), true) {
        Err(Error::Usage(message)) => {
            assert!(message.contains("Template source remote is already initialized"), "{message}");
            assert!(message.contains(&existing.display().to_string()), "{message}");
        }
        other => panic!("expected usage error, got {other:?}"),
    }
    assert!(matches!(init_templates(&paths, None, true), Err(Error::Usage(_))));
    assert!(matches!(
        init_templates(&paths, Some("https://example.com/acme/templates.git"), true),
        Err(Error::Usage(_))
    ));

    // An already initialized source is a no-op.
    init_templates(&paths, Some("acme/current-templates"), true).unwrap();

    // A non-empty, non-git target is refused.
    write(&fx.template_root.join("acme/occupied/file.txt"), "x");
    assert!(matches!(init_templates(&paths, Some("acme/occupied"), true), Err(Error::Usage(_))));
}

#[test]
fn template_details_report_variables_and_options() {
    let fx = fixture();
    fx.write_uncommitted_template("base", &[("README.md.boilersync", "Base $${company_name}\n")], Some(json!({})));
    fx.write_uncommitted_template(
        "service",
        &[("main.py.boilersync", "tier = '$${service_tier}'\nname = '$${name_snake}'\n"), ("NAME_KEBAB.cfg", "")],
        Some(json!({
            "extends": "acme/templates#base",
            "variables": {
                "company_name": {"description": "Company name", "default": "Acme"},
                "region": {"type": "string", "choices": ["us", "eu"], "default": "us"},
                "size": ["s", "m"],
                "retries": 3
            },
            "options": {"with_ci": {"type": "boolean", "description": "Enable CI workflow", "default": true}}
        })),
    );

    let details = get_template_details(&fx.paths(&fx.root), "acme/templates#service").unwrap();
    assert_eq!(details.template_ref, fx.canonical_ref("service"));
    assert_eq!(details.inheritance_chain, [fx.canonical_ref("base"), fx.canonical_ref("service")]);
    let variables: std::collections::BTreeMap<_, _> = details.variables.iter().map(|v| (v.name.clone(), v)).collect();
    assert!(!variables["company_name"].required);
    assert_eq!(variables["company_name"].default, Some(json!("Acme")));
    assert_eq!(variables["company_name"].description, "Company name");
    assert!(variables["service_tier"].required);
    assert_eq!(variables["service_tier"].label, "Service Tier");
    assert_eq!(variables["region"].choices, Some(vec![json!("us"), json!("eu")]));
    assert_eq!(variables["size"].choices, Some(vec![json!("s"), json!("m")]));
    assert_eq!(variables["retries"].default, Some(json!(3)));
    assert!(variables.contains_key("name_snake"));
    assert!(!variables["name_snake"].required);
    assert_eq!(variables["name_pretty"].label, "Pretty Name");
    // NAME_KEBAB in a path is a built-in naming variable, so it is not an input.
    assert!(!variables.contains_key("name_kebab"));

    assert_eq!(details.options.len(), 1);
    assert_eq!(details.options[0].name, "with_ci");
    assert_eq!(details.options[0].value_type, "boolean");
    assert_eq!(details.options[0].default, Some(json!(true)));

    let json = serde_json::to_value(&details).unwrap();
    assert_eq!(json["variables"][0]["type"], "string");
    assert!(json["options"][0].get("choices").is_none());
}

#[test]
fn paths_find_project_roots_and_parents() {
    let fx = fixture();
    let workspace = fx.root.join("workspace");
    std::fs::create_dir_all(workspace.join(".boilersync")).unwrap(); // a directory, not metadata
    let child = workspace.join("child");
    std::fs::create_dir_all(&child).unwrap();
    assert_eq!(find_parent_boilersync(&child), None);
    assert!(matches!(Paths::new(&fx.template_root, &child).root_dir(), Err(Error::NotFound(_))));

    let project = fx.root.join("project");
    write(&project.join(".boilersync"), "{\"template\": \"x#y\"}");
    let nested = project.join("a/b");
    std::fs::create_dir_all(&nested).unwrap();
    assert_eq!(Paths::new(&fx.template_root, &nested).root_dir().unwrap(), project);
    assert_eq!(find_parent_boilersync(&nested), Some(project.clone()));
    assert_eq!(find_parent_boilersync(&project), None);
    assert_eq!(Paths::new(&fx.template_root, &nested).with_root_override(&fx.root).root_dir().unwrap(), fx.root);
}

#[test]
fn children_are_registered_once_and_unreadable_parents_are_skipped() {
    let fx = fixture();
    let parent = fx.root.join("parent");
    let metadata = parent.join(".boilersync");
    write(&metadata, "{\"template\": \"x#y\"}");
    let child = parent.join("apps/web");
    std::fs::create_dir_all(&child).unwrap();

    add_child_to_parent(&child, &metadata).unwrap();
    add_child_to_parent(&child, &metadata).unwrap();
    assert_eq!(read_json(&metadata)["children"], json!(["apps/web"]));
    assert!(read(&metadata).ends_with('\n'));
    assert_eq!(children_from_boilersync(&metadata), std::slice::from_ref(&child));

    let alias = fx.root.join("alias").join(".boilersync");
    write(&alias, "book\u{0}\u{0}mark\u{fc}");
    add_child_to_parent(&child, &alias).unwrap();
    assert_eq!(children_from_boilersync(&alias), Vec::<std::path::PathBuf>::new());
}
