mod common;

use boilersync_core::error::Error;
use boilersync_core::inheritance::get_template_inheritance_chain;
use boilersync_core::template_config::get_parent_template;
use boilersync_core::template_sources::{resolve_source_from_boilersync, resolve_template_source};
use common::*;
use serde_json::json;

fn refs(fx: &Fixture, name: &str) -> Vec<String> {
    get_template_inheritance_chain(&fx.paths(&fx.root), &fx.template_ref(name))
        .unwrap()
        .iter()
        .map(|s| s.ref_.clone())
        .collect()
}

#[test]
fn parent_template_is_read_from_extends_or_parent() {
    let fx = fixture();
    fx.write_uncommitted_template(
        "child",
        &[("README.md", "# x\n")],
        Some(json!({"parent": fx.template_ref("parent")})),
    );
    fx.write_uncommitted_template("standalone", &[("README.md", "# x\n")], None);
    fx.write_uncommitted_template("other", &[("README.md", "# x\n")], Some(json!({"other_key": "value"})));
    fx.write_uncommitted_template("broken", &[("README.md", "# x\n"), ("template.json", "invalid json")], None);
    fx.write_uncommitted_template("extended", &[("README.md", "# x\n")], Some(json!({"extends": "base"})));

    assert_eq!(get_parent_template(&fx.repo_dir().join("child")), Some(fx.template_ref("parent")));
    assert_eq!(get_parent_template(&fx.repo_dir().join("standalone")), None);
    assert_eq!(get_parent_template(&fx.repo_dir().join("other")), None);
    assert_eq!(get_parent_template(&fx.repo_dir().join("broken")), None);
    assert_eq!(get_parent_template(&fx.repo_dir().join("extended")), Some("base".to_string()));
}

#[test]
fn inheritance_chains_resolve_from_root_to_leaf() {
    let fx = fixture();
    fx.write_uncommitted_template("standalone", &[("README.md", "x")], None);
    fx.write_uncommitted_template("grandparent", &[("README.md", "x")], None);
    fx.write_uncommitted_template(
        "parent",
        &[("README.md", "x")],
        Some(json!({"parent": fx.template_ref("grandparent")})),
    );
    fx.write_uncommitted_template("child", &[("README.md", "x")], Some(json!({"parent": fx.template_ref("parent")})));
    fx.write_uncommitted_template("relative", &[("README.md", "x")], Some(json!({"extends": "parent"})));

    assert_eq!(refs(&fx, "standalone"), [fx.canonical_ref("standalone")]);
    assert_eq!(
        refs(&fx, "child"),
        [fx.canonical_ref("grandparent"), fx.canonical_ref("parent"), fx.canonical_ref("child")]
    );
    assert_eq!(
        refs(&fx, "relative"),
        [fx.canonical_ref("grandparent"), fx.canonical_ref("parent"), fx.canonical_ref("relative")]
    );
}

#[test]
fn circular_and_missing_templates_are_errors() {
    let fx = fixture();
    fx.write_uncommitted_template(
        "template_a",
        &[("README.md", "x")],
        Some(json!({"parent": fx.template_ref("template_b")})),
    );
    fx.write_uncommitted_template(
        "template_b",
        &[("README.md", "x")],
        Some(json!({"parent": fx.template_ref("template_a")})),
    );
    fx.write_uncommitted_template(
        "orphan",
        &[("README.md", "x")],
        Some(json!({"parent": fx.template_ref("nonexistent_parent")})),
    );

    match get_template_inheritance_chain(&fx.paths(&fx.root), &fx.template_ref("template_a")) {
        Err(Error::Invalid(message)) => assert!(message.contains("Circular dependency"), "{message}"),
        other => panic!("expected circular dependency error, got {other:?}"),
    }
    match get_template_inheritance_chain(&fx.paths(&fx.root), &fx.template_ref("orphan")) {
        Err(Error::NotFound(message)) => assert!(message.contains("nonexistent_parent"), "{message}"),
        other => panic!("expected not-found error, got {other:?}"),
    }
    match get_template_inheritance_chain(&fx.paths(&fx.root), &fx.template_ref("nonexistent_child")) {
        Err(Error::NotFound(message)) => assert!(message.contains("nonexistent_child"), "{message}"),
        other => panic!("expected not-found error, got {other:?}"),
    }
}

#[test]
fn resolves_source_refs_against_the_cache() {
    let fx = fixture();
    let local_repo = fx.template_root.join("acme").join("template-kit");
    std::fs::create_dir_all(local_repo.join(".git")).unwrap();
    std::fs::create_dir_all(local_repo.join("python/service-template")).unwrap();
    let paths = fx.paths(&fx.root);

    let source = resolve_template_source(&paths, "acme/template-kit#python/service-template", false).unwrap();
    assert_eq!(source.ref_, "https://github.com/acme/template-kit.git#python/service-template");
    assert_eq!(source.canonical_ref(), source.ref_);
    assert_eq!(
        (source.org.as_str(), source.repo.as_str(), source.subdir.as_str()),
        ("acme", "template-kit", "python/service-template")
    );
    assert_eq!(source.template_dir, local_repo.join("python/service-template"));
    assert_eq!(source.identifier(), "source:acme/template-kit#python/service-template");

    let from_url = resolve_source_from_boilersync(
        &paths,
        Some("https://github.com/acme/template-kit#python/service-template"),
        false,
    )
    .unwrap();
    assert_eq!(from_url.ref_, source.ref_);

    assert!(matches!(resolve_template_source(&paths, "acme/template-kit", false), Err(Error::Invalid(_))));
    assert!(matches!(resolve_source_from_boilersync(&paths, None, false), Err(Error::Invalid(_))));
    assert!(matches!(resolve_template_source(&paths, "acme/other#x", false), Err(Error::NotFound(_))));
}

#[test]
fn legacy_short_refs_use_the_default_repo() {
    let fx = fixture();
    let local_repo = fx.template_root.join("openbase-community").join("templates");
    std::fs::create_dir_all(local_repo.join(".git")).unwrap();
    std::fs::create_dir_all(local_repo.join("react-app")).unwrap();

    let source = resolve_source_from_boilersync(&fx.paths(&fx.root), Some("react-app"), false).unwrap();
    assert_eq!(source.ref_, "https://github.com/openbase-community/templates.git#react-app");
    assert_eq!(source.template_dir, local_repo.join("react-app"));
}
