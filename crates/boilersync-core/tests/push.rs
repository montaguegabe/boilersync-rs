mod common;

use boilersync_core::commands::push::{copy_changed_files_to_template, reverse_interpolate_project_files};
use boilersync_core::error::Error;
use boilersync_core::inheritance::get_template_inheritance_chain;
use boilersync_core::template_workspace::copy_template_chain_without_interpolation;
use common::*;
use serde_json::json;

#[test]
fn copy_changed_files_routes_parent_owned_files_to_parent_template() {
    let fx = fixture();
    fx.write_uncommitted_template("parent", &[("common.txt.boilersync", "parent\n")], None);
    fx.write_uncommitted_template("child", &[("child.txt.boilersync", "child\n")], Some(json!({"extends": "parent"})));

    let chain = get_template_inheritance_chain(&fx.paths(&fx.root), &fx.template_ref("child")).unwrap();
    let temp_repo = fx.root.join("diff");
    std::fs::create_dir_all(&temp_repo).unwrap();
    let ownership = copy_template_chain_without_interpolation(&chain, &temp_repo).unwrap();
    assert_eq!(ownership["common.txt"].source_relative_path, "common.txt.boilersync");
    assert!(ownership["common.txt"].template_dir.ends_with("parent"));

    commit_template_repo(&temp_repo);
    write(&temp_repo.join("common.txt"), "updated parent\n");
    git(&temp_repo, &["add", "-A"]);
    git(&temp_repo, &["commit", "-q", "-m", "update common"]);

    let (updated, repos) = copy_changed_files_to_template(&temp_repo, &ownership, chain.last().unwrap(), &[]).unwrap();
    assert_eq!(updated, ["common.txt"]);
    assert_eq!(read(&fx.repo_dir().join("parent/common.txt.boilersync")), "updated parent\n");
    assert!(!fx.repo_dir().join("child/common.txt.boilersync").exists());
    assert_eq!(repos.into_iter().collect::<Vec<_>>(), [fx.repo_dir()]);
}

#[test]
fn copy_changed_files_refuses_block_based_round_trip() {
    let fx = fixture();
    fx.write_uncommitted_template("parent", &[("README.md.boilersync", "Parent heading\n")], None);
    fx.write_uncommitted_template(
        "child",
        &[("README.md.boilersync", "$${% block readme %}\n$${ super() }\nChild extra\n$${% endblock %}\n")],
        Some(json!({"extends": "parent"})),
    );
    let chain = get_template_inheritance_chain(&fx.paths(&fx.root), &fx.template_ref("child")).unwrap();
    let temp_repo = fx.root.join("diff-blocks");
    std::fs::create_dir_all(&temp_repo).unwrap();
    let ownership = copy_template_chain_without_interpolation(&chain, &temp_repo).unwrap();
    commit_template_repo(&temp_repo);
    write(&temp_repo.join("README.md"), "Flattened output\n");
    git(&temp_repo, &["add", "-A"]);
    git(&temp_repo, &["commit", "-q", "-m", "flattened change"]);

    match copy_changed_files_to_template(&temp_repo, &ownership, chain.last().unwrap(), &[]) {
        Err(Error::Runtime(message)) => {
            assert!(message.contains("Refusing to push files derived from block-based"), "{message}")
        }
        other => panic!("expected runtime error, got {other:?}"),
    }
}

#[test]
fn new_files_go_to_the_leaf_template_and_deleted_files_are_skipped() {
    let fx = fixture();
    fx.write_uncommitted_template("parent", &[("common.txt", "parent\n")], None);
    fx.write_uncommitted_template("child", &[("child.txt", "child\n")], Some(json!({"extends": "parent"})));
    let chain = get_template_inheritance_chain(&fx.paths(&fx.root), &fx.template_ref("child")).unwrap();
    let temp_repo = fx.root.join("diff-new");
    std::fs::create_dir_all(&temp_repo).unwrap();
    let ownership = copy_template_chain_without_interpolation(&chain, &temp_repo).unwrap();
    commit_template_repo(&temp_repo);
    write(&temp_repo.join("docs/guide.md"), "new\n");
    std::fs::remove_file(temp_repo.join("child.txt")).unwrap();
    git(&temp_repo, &["add", "-A"]);
    git(&temp_repo, &["commit", "-q", "-m", "add and delete"]);

    let (updated, _) = copy_changed_files_to_template(&temp_repo, &ownership, chain.last().unwrap(), &[]).unwrap();
    assert_eq!(updated, ["docs/guide.md"]);
    assert_eq!(read(&fx.repo_dir().join("child/docs/guide.md")), "new\n");
    assert!(fx.repo_dir().join("child/child.txt").exists());
}

#[test]
fn reverse_interpolation_restores_placeholders_in_names_and_content() {
    let fx = fixture();
    let project = fx.root.join("rev");
    write(
        &project.join("demo_app/demo_app_config.py"),
        "name = \"demo_app\"\ntitle = \"Demo App\"\nowner = \"Jane Doe\"\n",
    );
    write(&project.join(".boilersync"), "{}");
    let context = context(json!({
        "NAME_SNAKE": "demo_app", "NAME_PRETTY": "Demo App", "name_snake": "demo_app", "name_pretty": "Demo App",
        "author_name": "Jane Doe", "flag": true
    }));
    reverse_interpolate_project_files(&project, &context).unwrap();

    let file = project.join("NAME_SNAKE/NAME_SNAKE_config.py");
    assert!(file.exists(), "renamed file missing");
    assert_eq!(read(&file), "name = \"$${name_snake}\"\ntitle = \"$${name_pretty}\"\nowner = \"$${author_name}\"\n");
    assert_eq!(read(&project.join(".boilersync")), "{}");
}
