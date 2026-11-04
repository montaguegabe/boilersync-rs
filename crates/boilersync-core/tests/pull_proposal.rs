mod common;

use boilersync_core::commands::pull_proposal::{
    ProposalOptions, apply_patch_file, apply_proposed_file, create_pull_proposal, proposal_to_text,
};
use boilersync_core::fsx;
use common::*;
use serde_json::json;

fn init_project(fx: &Fixture) -> std::path::PathBuf {
    fx.write_template(
        "service-template",
        &[("app.txt.boilersync", "App v1 $${name_snake}\n")],
        Some(json!({"skip_git": true})),
    );
    let target = fx.root.join("project");
    fx.init_project("service-template", &target, json!({"name_snake": "demo_service"}));
    fx.write_template(
        "service-template",
        &[("app.txt.boilersync", "App v2 $${name_snake}\n"), ("new.txt.boilersync", "New $${name_snake}\n")],
        None,
    );
    target
}

#[test]
fn create_pull_proposal_does_not_mutate_project() {
    let fx = fixture();
    let target = init_project(&fx);
    git(&target, &["init", "-q"]);
    write(&target.join(".gitignore"), "ignored.txt\n");
    write(&target.join("ignored.txt"), "ignored local output\n");
    write(&target.join("node_modules/left-pad/index.js"), "module.exports = null;\n");
    let workspace_dir = fx.root.join("proposal-workspace");

    let proposal = create_pull_proposal(
        &fx.paths(&target),
        Some(&target),
        &ProposalOptions { workspace_dir: Some(workspace_dir.clone()), ..ProposalOptions::default() },
    )
    .unwrap();

    assert_eq!(read(&target.join("app.txt")), "App v1 demo_service\n");
    assert!(!target.join("new.txt").exists());
    assert_eq!(proposal.proposal_dir, workspace_dir.join("proposal"));
    assert_eq!(read(&proposal.proposal_dir.join("app.txt")), "App v2 demo_service\n");
    assert_eq!(read(&proposal.proposal_dir.join("new.txt")), "New demo_service\n");
    assert!(!proposal.proposal_dir.join("ignored.txt").exists());
    assert!(!proposal.proposal_dir.join("node_modules").exists());

    let changed: Vec<(String, String)> =
        proposal.changed_files.iter().map(|e| (e.status.clone(), e.path.clone())).collect();
    assert_eq!(
        changed,
        [
            ("M".to_string(), ".boilersync".to_string()),
            ("M".to_string(), "app.txt".to_string()),
            ("A".to_string(), "new.txt".to_string())
        ]
    );
    let text = proposal_to_text(&proposal);
    assert!(text.contains("Changed files: 3"), "{text}");
    assert!(text.contains("A\tnew.txt"), "{text}");
}

#[test]
fn apply_proposed_file_copies_one_file_to_project() {
    let fx = fixture();
    let target = init_project(&fx);
    let proposal = create_pull_proposal(
        &fx.paths(&target),
        Some(&target),
        &ProposalOptions { workspace_dir: Some(fx.root.join("proposal-workspace")), ..ProposalOptions::default() },
    )
    .unwrap();

    let applied =
        apply_proposed_file(&fx.paths(&target), &proposal.proposal_dir, std::path::Path::new("new.txt"), Some(&target))
            .unwrap();
    assert_eq!(applied, fsx::resolve(&target).join("new.txt"));
    assert_eq!(read(&target.join("new.txt")), "New demo_service\n");
    assert_eq!(read(&target.join("app.txt")), "App v1 demo_service\n");

    assert!(
        apply_proposed_file(
            &fx.paths(&target),
            &proposal.proposal_dir,
            std::path::Path::new("../escape.txt"),
            Some(&target)
        )
        .is_err()
    );
    assert!(
        apply_proposed_file(
            &fx.paths(&target),
            &proposal.proposal_dir,
            std::path::Path::new("missing.txt"),
            Some(&target)
        )
        .is_err()
    );
}

#[test]
fn apply_patch_applies_selected_hunk_to_project() {
    let fx = fixture();
    let target = init_project(&fx);
    let proposal = create_pull_proposal(
        &fx.paths(&target),
        Some(&target),
        &ProposalOptions { workspace_dir: Some(fx.root.join("proposal-workspace")), ..ProposalOptions::default() },
    )
    .unwrap();
    let patch_path = fx.root.join("app.patch");
    let patch = git(&proposal.proposal_dir, &["diff", "--", "app.txt"]);
    write(&patch_path, &format!("{patch}\n"));

    apply_patch_file(&fx.paths(&target), &patch_path, Some(&target)).unwrap();
    assert_eq!(read(&target.join("app.txt")), "App v2 demo_service\n");
    assert!(!target.join("new.txt").exists());
}
