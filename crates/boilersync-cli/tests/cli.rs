//! End-to-end tests of the `boilersync` binary.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn git(cwd: &Path, args: &[&str]) {
    let output = Command::new("git").args(args).current_dir(cwd).output().unwrap();
    assert!(output.status.success(), "git {} failed: {}", args.join(" "), String::from_utf8_lossy(&output.stderr));
}

struct Fixture {
    _tmp: tempfile::TempDir,
    root: PathBuf,
    template_root: PathBuf,
}

fn fixture() -> Fixture {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().to_path_buf();
    let template_root = root.join("templates");
    let template_dir = template_root.join("acme/templates/service");
    std::fs::create_dir_all(&template_dir).unwrap();
    std::fs::write(template_dir.join("template.json"), r#"{"skip_git": true, "defaults": {"greeting": "hi"}}"#)
        .unwrap();
    std::fs::write(template_dir.join("README.md.boilersync"), "# $${name_pretty}\n$${greeting} from $${name_snake}\n")
        .unwrap();
    std::fs::write(template_dir.join("notes.starter.md"), "starter\n").unwrap();
    let repo = template_root.join("acme/templates");
    git(&repo, &["init", "-q"]);
    git(&repo, &["config", "user.name", "Tests"]);
    git(&repo, &["config", "user.email", "tests@example.invalid"]);
    git(&repo, &["config", "commit.gpgsign", "false"]);
    git(&repo, &["add", "-A"]);
    git(&repo, &["commit", "-q", "-m", "fixtures"]);
    Fixture { _tmp: tmp, root, template_root }
}

impl Fixture {
    fn run(&self, cwd: &Path, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_boilersync"))
            .args(args)
            .current_dir(cwd)
            .env("BOILERSYNC_TEMPLATE_DIR", &self.template_root)
            .env_remove("BOILERSYNC_ROOT_DIR")
            .output()
            .unwrap()
    }

    fn ok(&self, cwd: &Path, args: &[&str]) -> String {
        let output = self.run(cwd, args);
        assert!(
            output.status.success(),
            "boilersync {} failed: {}{}",
            args.join(" "),
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8_lossy(&output.stdout).to_string()
    }

    fn init_project(&self, name: &str) -> PathBuf {
        let project = self.root.join(name);
        std::fs::create_dir_all(&project).unwrap();
        self.ok(&project, &["init", "acme/templates#service", "--non-interactive", "--var", "name_snake=demo_app"]);
        project
    }
}

#[test]
fn version_and_help() {
    let fx = fixture();
    assert!(fx.ok(&fx.root, &["--version"]).starts_with("boilersync "));
    let help = fx.ok(&fx.root, &["--help"]);
    for command in ["init", "check-pull", "diff", "pull", "pull-proposal", "push", "templates", "skills"] {
        assert!(help.contains(command), "help lacks {command}: {help}");
    }
}

#[test]
fn init_non_interactive_renders_and_records_metadata() {
    let fx = fixture();
    let project = fx.init_project("project");
    assert_eq!(std::fs::read_to_string(project.join("README.md")).unwrap(), "# Demo App\nhi from demo_app\n");
    assert!(project.join("notes.md").exists());
    let metadata: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(project.join(".boilersync")).unwrap()).unwrap();
    assert_eq!(metadata["template"], "https://github.com/acme/templates.git#service");
    assert_eq!(metadata["variables"]["greeting"], "hi");
    assert_eq!(metadata["name_pretty"], "Demo App");

    // --no-input is the documented alias and the directory must be empty.
    let again = fx.run(&project, &["init", "acme/templates#service", "--no-input", "--var", "name_snake=demo_app"]);
    assert_eq!(again.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&again.stderr).contains("not empty"));
}

#[test]
fn check_pull_exit_code_and_diff_json() {
    let fx = fixture();
    let project = fx.init_project("project");
    let status = fx.run(&project, &["check-pull"]);
    assert!(status.status.success(), "{}", String::from_utf8_lossy(&status.stderr));
    assert!(String::from_utf8_lossy(&status.stdout).contains("Due for pull: no"));

    let template_file = fx.template_root.join("acme/templates/service/README.md.boilersync");
    std::fs::write(&template_file, "# $${name_pretty}\nchanged\n").unwrap();
    git(&fx.template_root.join("acme/templates"), &["commit", "-q", "-am", "update"]);
    let due = fx.run(&project, &["check-pull"]);
    assert_eq!(due.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&due.stdout).contains("Due for pull: yes"));

    let diff: serde_json::Value = serde_json::from_str(&fx.ok(&project, &["diff", "--json"])).unwrap();
    assert_eq!(diff["projects"][0]["changed_count"], 1);
    assert_eq!(diff["projects"][0]["changed_files"][0]["path"], "README.md");

    let conflicting = fx.run(&project, &["diff", "--json", "--patch"]);
    assert_eq!(conflicting.status.code(), Some(2));

    // Nested working directories resolve to the project root, and the
    // BOILERSYNC_ROOT_DIR override wins.
    let nested = project.join("deep/er");
    std::fs::create_dir_all(&nested).unwrap();
    assert!(fx.ok(&nested, &["diff", "--name-status"]).contains("M\tREADME.md"));
    let output = Command::new(env!("CARGO_BIN_EXE_boilersync"))
        .args(["check-pull"])
        .current_dir(&fx.root)
        .env("BOILERSYNC_TEMPLATE_DIR", &fx.template_root)
        .env("BOILERSYNC_ROOT_DIR", &project)
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
}

#[test]
fn pull_proposal_and_pull_apply_template_updates() {
    let fx = fixture();
    let project = fx.init_project("project");
    std::fs::write(fx.template_root.join("acme/templates/service/README.md.boilersync"), "# $${name_pretty}\nv2\n")
        .unwrap();
    git(&fx.template_root.join("acme/templates"), &["commit", "-q", "-am", "v2"]);

    let workspace = fx.root.join("proposal-ws");
    let proposal: serde_json::Value = serde_json::from_str(
        &fx.ok(&project, &["pull-proposal", "create", "--json", "--workspace-dir", workspace.to_str().unwrap()]),
    )
    .unwrap();
    assert_eq!(proposal["changed_count"], 2);
    assert_eq!(std::fs::read_to_string(project.join("README.md")).unwrap(), "# Demo App\nhi from demo_app\n");

    fx.ok(&project, &["pull-proposal", "apply-file", workspace.join("proposal").to_str().unwrap(), "README.md"]);
    assert_eq!(std::fs::read_to_string(project.join("README.md")).unwrap(), "# Demo App\nv2\n");

    // A direct pull in a clean project updates the recorded commit.
    git(&project, &["init", "-q"]);
    git(&project, &["config", "user.name", "Tests"]);
    git(&project, &["config", "user.email", "tests@example.invalid"]);
    git(&project, &["config", "commit.gpgsign", "false"]);
    git(&project, &["add", "-A"]);
    git(&project, &["commit", "-q", "-m", "baseline"]);
    fx.ok(&project, &["pull"]);
    assert!(fx.run(&project, &["check-pull"]).status.success());

    std::fs::write(project.join("dirty.txt"), "x").unwrap();
    let dirty = fx.run(&project, &["pull"]);
    assert_eq!(dirty.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&dirty.stderr).contains("uncommitted changes"));
}

#[test]
fn templates_commands_list_details_and_sources() {
    let fx = fixture();
    let list: serde_json::Value = serde_json::from_str(&fx.ok(&fx.root, &["templates", "list", "--json"])).unwrap();
    assert_eq!(list["count"], 1);
    assert_eq!(list["templates"][0]["template_ref"], "acme/templates#service");

    let details: serde_json::Value =
        serde_json::from_str(&fx.ok(&fx.root, &["templates", "details", "acme/templates#service", "--json"])).unwrap();
    let names: Vec<&str> =
        details["variables"].as_array().unwrap().iter().map(|v| v["name"].as_str().unwrap()).collect();
    assert_eq!(names, ["greeting", "name_pretty", "name_snake"]);

    let sources = fx.ok(&fx.root, &["templates", "sources"]);
    assert!(sources.contains("- acme/templates"), "{sources}");
    assert!(sources.contains("templates: 1"), "{sources}");

    let both = fx.run(&fx.root, &["templates", "init", "acme/x", "--repo-url", "acme/y"]);
    assert_eq!(both.status.code(), Some(2));
}
