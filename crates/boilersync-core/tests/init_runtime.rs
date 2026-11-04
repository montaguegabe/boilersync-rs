mod common;

use boilersync_core::commands::init::{
    InitOptions, RuntimeConfig, create_github_repo, init, inject_monorepo_conversion_hook, merge_runtime_config,
};
use boilersync_core::error::Error;
use boilersync_core::inheritance::get_template_inheritance_chain;
use common::*;
use serde_json::json;

#[test]
fn merge_runtime_config_inheritance() {
    let fx = fixture();
    fx.write_template(
        "parent-template",
        &[("README.md", "parent")],
        Some(json!({
            "hooks": {"post_init": [{"id": "parent_hook", "run": "echo parent"}]},
            "github": {"create_repo": true, "private": true},
            "children": [{"template": fx.template_ref("child-template"), "path": "child-dir"}]
        })),
    );
    fx.write_template(
        "child-template",
        &[("README.md", "child")],
        Some(json!({
            "extends": fx.template_ref("parent-template"),
            "hooks": {"post_init": [{"id": "child_hook", "run": "echo child"}]},
            "github": {"private": false}
        })),
    );

    let chain = get_template_inheritance_chain(&fx.paths(&fx.root), &fx.template_ref("child-template")).unwrap();
    let merged = merge_runtime_config(&chain).unwrap();
    assert_eq!(merged.children.len(), 1);
    let ids: Vec<&str> = merged.post_init.iter().map(|hook| hook["id"].as_str().unwrap()).collect();
    assert_eq!(ids, ["parent_hook", "child_hook"]);
    assert_eq!(merged.github["create_repo"], true);
    assert_eq!(merged.github["private"], false);
}

#[test]
fn merge_runtime_config_rejects_wrong_shapes() {
    let fx = fixture();
    fx.write_template("bad", &[("README.md", "x")], Some(json!({"children": {"template": "x"}})));
    let chain = get_template_inheritance_chain(&fx.paths(&fx.root), &fx.template_ref("bad")).unwrap();
    match merge_runtime_config(&chain) {
        Err(Error::Invalid(message)) => assert!(message.contains("'children' config: expected list"), "{message}"),
        other => panic!("expected invalid error, got {other:?}"),
    }
}

#[test]
fn init_with_children_and_hooks() {
    let fx = fixture();
    fx.write_template(
        "workspace-template",
        &[("README.md.boilersync", "Workspace $${name_snake}\n")],
        Some(json!({
            "children": [{
                "template": fx.template_ref("child-template"),
                "path": "$${name_kebab}-child",
                "condition": "with_frontend",
                "variables": {"child_message": "$${workspace_message}"}
            }],
            "hooks": {"post_init": [{
                "id": "write_hook_output",
                "run": "printf '%s' \"$HOOK_VALUE\" > hook-output.txt",
                "env": {"HOOK_VALUE": "$${workspace_message}"}
            }]},
            "skip_git": true
        })),
    );
    fx.write_template(
        "child-template",
        &[("child.txt.boilersync", "$${child_message}\n")],
        Some(json!({"skip_git": true})),
    );

    let target = fx.root.join("workspace");
    std::fs::create_dir_all(&target).unwrap();
    let options = InitOptions {
        template_variables: Some(context(json!({"name_snake": "demo_workspace", "workspace_message": "hello child"}))),
        options: Some(context(json!({"with_frontend": true}))),
        no_input: true,
        ..InitOptions::new()
    };
    init(&fx.paths(&target), &fx.template_ref("workspace-template"), &target, &options).unwrap();

    assert_eq!(read(&target.join("README.md")), "Workspace demo_workspace\n");
    assert_eq!(read(&target.join("hook-output.txt")), "hello child");
    let child_dir = target.join("demo-workspace-child");
    assert_eq!(read(&child_dir.join("child.txt")).trim(), "hello child");

    let data = read_json(&target.join(".boilersync"));
    assert_eq!(data["template"], fx.canonical_ref("workspace-template"));
    assert!(data.get("source").is_none());
    assert_eq!(data["children"], json!(["demo-workspace-child"]));
}

#[test]
fn conditional_children_and_hooks_are_skipped_when_false() {
    let fx = fixture();
    fx.write_template(
        "workspace-template",
        &[("README.md.boilersync", "Workspace\n")],
        Some(json!({
            "children": [{"template": "child-template", "path": "child", "condition": "with_child"}],
            "hooks": {"post_init": [{"id": "never", "run": "touch never.txt", "condition": "with_child"}]},
            "skip_git": true
        })),
    );
    fx.write_template("child-template", &[("child.txt", "child\n")], Some(json!({"skip_git": true})));
    let target = fx.root.join("workspace");
    fx.init_project("workspace-template", &target, json!({"name_snake": "demo"}));
    assert!(!target.join("child").exists());
    assert!(!target.join("never.txt").exists());
}

#[test]
fn failing_hooks_fail_init_unless_allowed() {
    let fx = fixture();
    fx.write_template(
        "strict",
        &[("README.md", "x")],
        Some(json!({"hooks": {"post_init": [{"id": "boom", "run": "exit 3"}]}, "skip_git": true})),
    );
    fx.write_template(
        "lenient",
        &[("README.md", "x")],
        Some(json!({"hooks": {"post_init": [{"id": "boom", "run": "exit 3", "allow_failure": true}, {"run": "touch after.txt"}]}, "skip_git": true})),
    );
    let strict_target = fx.root.join("strict");
    std::fs::create_dir_all(&strict_target).unwrap();
    let options = InitOptions {
        template_variables: Some(context(json!({"name_snake": "demo"}))),
        no_input: true,
        ..InitOptions::new()
    };
    match init(&fx.paths(&strict_target), &fx.template_ref("strict"), &strict_target, &options) {
        Err(Error::Runtime(message)) => assert!(message.contains("Hook 'boom' failed with exit code 3"), "{message}"),
        other => panic!("expected runtime error, got {other:?}"),
    }

    let lenient_target = fx.root.join("lenient");
    fx.init_project("lenient", &lenient_target, json!({"name_snake": "demo"}));
    assert!(lenient_target.join("after.txt").exists());
}

#[test]
fn template_defaults_are_applied_before_variable_collection() {
    let fx = fixture();
    fx.write_template(
        "defaulted-template",
        &[(
            "README.md.boilersync",
            "$${api_package_name} $${web_package_name} $${api_client_export_name} $${with_frontend}\n",
        )],
        Some(json!({
            "defaults": {
                "api_package_name": "$${name_snake}_api",
                "web_package_name": "$${name_kebab}-web",
                "api_client_export_name": "$${name_camel}",
                "with_frontend": true
            },
            "skip_git": true
        })),
    );
    let target = fx.root.join("defaulted-workspace");
    fx.init_project("defaulted-template", &target, json!({"name_snake": "demo_workspace"}));

    assert_eq!(read(&target.join("README.md")), "demo_workspace_api demo-workspace-web demoWorkspace True\n");
    let data = read_json(&target.join(".boilersync"));
    assert_eq!(data["variables"]["api_package_name"], "demo_workspace_api");
    assert_eq!(data["variables"]["with_frontend"], true);
}

#[test]
fn explicit_variables_win_over_defaults() {
    let fx = fixture();
    fx.write_template(
        "t",
        &[("out.txt.boilersync", "$${greeting}\n")],
        Some(json!({"defaults": {"greeting": "hello"}, "skip_git": true})),
    );
    let target = fx.root.join("p");
    fx.init_project("t", &target, json!({"name_snake": "demo", "greeting": "custom"}));
    assert_eq!(read(&target.join("out.txt")), "custom\n");
}

#[test]
fn missing_variables_fail_in_non_interactive_mode() {
    let fx = fixture();
    fx.write_template("t", &[("out.txt.boilersync", "$${author_name}\n")], Some(json!({"skip_git": true})));
    let target = fx.root.join("p");
    std::fs::create_dir_all(&target).unwrap();
    let options = InitOptions {
        template_variables: Some(context(json!({"name_snake": "demo"}))),
        no_input: true,
        ..InitOptions::new()
    };
    match init(&fx.paths(&target), &fx.template_ref("t"), &target, &options) {
        Err(Error::Invalid(message)) => assert!(message.contains("author_name"), "{message}"),
        other => panic!("expected invalid error, got {other:?}"),
    }
}

#[test]
fn init_refuses_non_empty_directory() {
    let fx = fixture();
    fx.write_template("t", &[("out.txt", "x\n")], Some(json!({"skip_git": true})));
    let target = fx.root.join("p");
    write(&target.join("existing.txt"), "x");
    let options = InitOptions {
        template_variables: Some(context(json!({"name_snake": "demo"}))),
        no_input: true,
        ..InitOptions::new()
    };
    assert!(matches!(init(&fx.paths(&target), &fx.template_ref("t"), &target, &options), Err(Error::AlreadyExists(_))));
}

#[test]
fn init_with_local_child_template_name() {
    let fx = fixture();
    fx.write_template(
        "workspace-template-local",
        &[("README.md.boilersync", "Workspace\n")],
        Some(json!({"children": [{"template": "child-template-local", "path": "child-project"}], "skip_git": true})),
    );
    fx.write_template("child-template-local", &[("child.txt.boilersync", "child\n")], Some(json!({"skip_git": true})));
    let target = fx.root.join("workspace-local-child");
    fx.init_project("workspace-template-local", &target, json!({"name_snake": "demo_workspace"}));
    assert!(target.join("child-project/child.txt").exists());
}

#[test]
fn child_path_can_use_derived_kebab_variable() {
    let fx = fixture();
    fx.write_template(
        "workspace-template",
        &[("README.md.boilersync", "Workspace\n")],
        Some(json!({"children": [{"template": fx.template_ref("child-template"), "path": "$${api_package_name_kebab}"}], "skip_git": true})),
    );
    fx.write_template("child-template", &[("child.txt.boilersync", "child\n")], Some(json!({"skip_git": true})));
    let target = fx.root.join("workspace");
    fx.init_project(
        "workspace-template",
        &target,
        json!({"name_snake": "demo_workspace", "api_package_name_snake": "demo_api"}),
    );
    assert!(target.join("demo-api/child.txt").exists());
}

#[test]
fn child_name_overrides_are_rendered() {
    let fx = fixture();
    fx.write_template(
        "ws",
        &[("README.md", "ws\n")],
        Some(json!({"children": [{"template": "svc", "path": "api", "name_snake": "$${name_snake}_api", "name_pretty": "$${name_pretty} API"}], "skip_git": true})),
    );
    fx.write_template(
        "svc",
        &[("name.txt.boilersync", "$${name_snake} / $${name_pretty}\n")],
        Some(json!({"skip_git": true})),
    );
    let target = fx.root.join("ws");
    fx.init_project("ws", &target, json!({"name_snake": "demo", "name_pretty": "Demo"}));
    assert_eq!(read(&target.join("api/name.txt")), "demo_api / Demo API\n");
}

#[test]
fn monorepo_flag_adds_conversion_hook_only_with_multi_json() {
    let fx = fixture();
    let with = fx.root.join("with");
    write(&with.join("multi.json"), "{\"repos\": []}\n");
    let mut config = RuntimeConfig::default();
    inject_monorepo_conversion_hook(&mut config, &with);
    assert_eq!(config.post_init, vec![json!({"id": "convert-monorepo", "run": "multi convert-monorepo --confirm"})]);

    let without = fx.root.join("without");
    std::fs::create_dir_all(&without).unwrap();
    let mut config = RuntimeConfig::default();
    inject_monorepo_conversion_hook(&mut config, &without);
    assert!(config.post_init.is_empty());
}

#[test]
fn create_github_repo_skips_when_condition_variable_missing_or_disabled() {
    let fx = fixture();
    let config =
        context(json!({"create_repo": true, "repo_name": "demo-repo", "private": false, "condition": "with_github"}));
    create_github_repo(&config, &fx.root, &context(json!({}))).unwrap();
    create_github_repo(&context(json!({"create_repo": false})), &fx.root, &context(json!({}))).unwrap();
    create_github_repo(&context(json!({})), &fx.root, &context(json!({}))).unwrap();
}

#[test]
fn init_initializes_git_unless_skipped() {
    let fx = fixture();
    fx.write_template("gitted", &[("README.md.boilersync", "$${name_snake}\n")], Some(json!({})));
    let target = fx.root.join("gitted");
    fx.init_project("gitted", &target, json!({"name_snake": "demo"}));
    assert!(target.join(".git").is_dir());
    assert!(git(&target, &["log", "--oneline"]).contains("Initial commit from template"));
    assert!(git(&target, &["status", "--porcelain"]).is_empty());
}

#[test]
fn children_inherit_parent_variables_and_render_methods() {
    let fx = fixture();
    fx.write_template(
        "ws",
        &[("README.md", "ws\n")],
        Some(json!({
            "defaults": {"python_version": "3.12", "publish": false},
            "children": [{"template": "svc", "path": "api", "variables": {"publish": "$${publish}"}}],
            "skip_git": true
        })),
    );
    fx.write_template(
        "svc",
        &[("token.txt.boilersync", "$${name_snake.upper()}_TOKEN $${python_version}\n")],
        Some(json!({"skip_git": true})),
    );
    let target = fx.root.join("ws");
    fx.init_project("ws", &target, json!({"name_snake": "demo", "author_name": "Jane"}));

    assert_eq!(read(&target.join("api/token.txt")), "DEMO_TOKEN 3.12\n");
    let child = read_json(&target.join("api/.boilersync"));
    assert_eq!(child["variables"]["author_name"], "Jane");
    assert_eq!(child["variables"]["python_version"], "3.12");
    assert_eq!(child["variables"]["publish"], "False");
    assert_eq!(child["name_snake"], "demo");
}
