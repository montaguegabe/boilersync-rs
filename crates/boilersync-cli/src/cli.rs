//! Argument definitions (clap) and dispatch into `boilersync-core`.

use std::path::PathBuf;

use boilersync_core::commands::check_pull::{check_pull, status_report};
use boilersync_core::commands::diff::{DiffOptions, OutputMode, diff};
use boilersync_core::commands::init::{InitOptions, init};
use boilersync_core::commands::pull::{PullOptions, pull};
use boilersync_core::commands::pull_proposal::{
    ProposalOptions, apply_patch_file, apply_proposed_file, create_pull_proposal, proposal_to_json, proposal_to_text,
};
use boilersync_core::commands::push::push;
use boilersync_core::commands::skills::{SkillsInstallOptions, install_skills};
use boilersync_core::commands::templates::{
    get_template_details, init_templates, list_local_templates, list_template_sources,
};
use boilersync_core::error::{Error, Result};
use boilersync_core::paths::Paths;
use boilersync_core::variables::parse_key_value_options;
use clap::{Args, Parser, Subcommand};

/// BoilerSync - a template CLI that generates projects and keeps templates
/// updated as derived projects evolve.
#[derive(Parser, Debug)]
#[command(name = "boilersync", version, about, long_about = None)]
pub struct Cli {
    /// Enable verbose output.
    #[arg(long, global = true)]
    pub verbose: bool,

    #[command(subcommand)]
    pub command: Command,
}

#[derive(Subcommand, Debug)]
pub enum Command {
    /// Initialize a new project from a template (empty directory only).
    ///
    /// TEMPLATE_REF is either a source-qualified ref (ORG/REPO#SUBDIR) or a
    /// GitHub URL ref (https://github.com/ORG/REPO.git#SUBDIR).
    ///
    /// For non-interactive usage, provide --non-interactive and any required
    /// template variables:
    ///
    ///   boilersync init your-org/your-templates#service --non-interactive --var name_snake=my_project --var author_name="John Doe"
    Init(InitArgs),
    /// Check whether the current project is due for a boilersync pull.
    ///
    /// Exits with status 1 when the project is behind the template repository.
    CheckPull,
    /// Show how the current project has diverged from its upstream template.
    Diff(DiffArgs),
    /// Pull template changes to the current project.
    ///
    /// TEMPLATE_REF is auto-detected from the nearest .boilersync file when
    /// omitted. Works in non-empty directories if the git repository is clean.
    /// Parent templates are processed first, then child templates. Starter
    /// files (.starter) are excluded unless --include-starter is given. Child
    /// projects listed in .boilersync are updated too unless --no-children.
    Pull(PullArgs),
    /// Create and apply reviewable BoilerSync pull proposals.
    #[command(subcommand)]
    PullProposal(PullProposalCommand),
    /// Show differences between the project and its template, then copy
    /// committed changes back to the template repo.
    ///
    /// Creates a temporary directory with a fresh template initialization,
    /// copies the current project files over it and opens the result in
    /// GitHub Desktop. Only changes committed there are copied back to the
    /// template source repository.
    Push(PushArgs),
    /// Manage local template source cache configuration and setup.
    #[command(subcommand)]
    Templates(TemplatesCommand),
    /// Manage the BoilerSync agent skills.
    #[command(subcommand)]
    Skills(SkillsCommand),
}

#[derive(Args, Debug)]
pub struct InitArgs {
    pub template_ref: String,
    /// Template variable in KEY=VALUE format (can be used multiple times).
    #[arg(short = 'v', long = "var", value_name = "KEY=VALUE")]
    pub variables: Vec<String>,
    /// Template runtime option in KEY=VALUE format (can be used multiple times).
    #[arg(short = 'o', long = "option", value_name = "KEY=VALUE")]
    pub runtime_options: Vec<String>,
    /// After init, convert the generated workspace to multi monorepo mode.
    #[arg(long)]
    pub monorepo: bool,
    /// Do not prompt for input (use defaults and --var for required values).
    #[arg(long = "no-input", visible_alias = "non-interactive")]
    pub no_input: bool,
}

#[derive(Args, Debug)]
pub struct DiffArgs {
    /// Show a diffstat summary. This is the default text mode.
    #[arg(long)]
    pub stat: bool,
    /// Show changed file names and statuses.
    #[arg(long = "name-status")]
    pub name_status: bool,
    /// Show the full patch.
    #[arg(long)]
    pub patch: bool,
    /// Emit JSON summary output.
    #[arg(long = "json")]
    pub json_output: bool,
    /// Include files derived from .starter template files.
    #[arg(long = "include-starter")]
    pub include_starter: bool,
    /// Include direct child projects listed in .boilersync.
    #[arg(long = "children")]
    pub include_children: bool,
    /// Include child projects recursively.
    #[arg(long)]
    pub recursive: bool,
    /// Write output to a file instead of stdout.
    #[arg(long = "output", value_name = "PATH")]
    pub output_path: Option<PathBuf>,
}

#[derive(Args, Debug)]
pub struct PullArgs {
    pub template_ref: Option<String>,
    /// Include starter files when pulling template changes.
    #[arg(long = "include-starter")]
    pub include_starter: bool,
    /// Skip updating child projects.
    #[arg(long = "no-children")]
    pub no_children: bool,
}

#[derive(Subcommand, Debug)]
pub enum PullProposalCommand {
    /// Create a temp git repo containing proposed pull changes.
    Create {
        /// Include starter files in the proposed pull.
        #[arg(long = "include-starter")]
        include_starter: bool,
        /// Include child projects in the proposed pull.
        #[arg(long = "children")]
        include_children: bool,
        /// Directory for the proposal workspace. Defaults to a per-project temp dir.
        #[arg(long = "workspace-dir", value_name = "DIR")]
        workspace_dir: Option<PathBuf>,
        /// Emit JSON output.
        #[arg(long = "json")]
        json_output: bool,
    },
    /// Copy one file from a proposal into the current project.
    ApplyFile { proposal_dir: PathBuf, path: PathBuf },
    /// Apply a selected patch to the current project with git apply.
    ApplyPatch { patch_path: PathBuf },
}

#[derive(Args, Debug)]
pub struct PushArgs {
    /// Additional files to add to the template (can be used multiple times).
    #[arg(long = "add-files", value_name = "FILE")]
    pub add_files: Vec<String>,
}

#[derive(Subcommand, Debug)]
pub enum TemplatesCommand {
    /// Clone a template source GitHub repository into the configured template cache.
    ///
    /// REPO_URL is optional. If omitted, boilersync will prompt for it.
    Init {
        repo_url: Option<String>,
        /// GitHub repository URL to clone.
        #[arg(long = "repo-url")]
        repo_url_option: Option<String>,
        /// Do not prompt for input.
        #[arg(long = "no-input")]
        no_input: bool,
    },
    /// List locally available templates in the template cache.
    List {
        /// Output as JSON.
        #[arg(long = "json")]
        json_output: bool,
    },
    /// Show local template source checkouts and duplicate remotes.
    Sources {
        /// Output as JSON.
        #[arg(long = "json")]
        json_output: bool,
    },
    /// Show input fields for a template reference.
    Details {
        template_ref: String,
        /// Output as JSON.
        #[arg(long = "json")]
        json_output: bool,
    },
}

#[derive(Subcommand, Debug)]
pub enum SkillsCommand {
    /// Install the BoilerSync skills for Codex and Claude Code.
    ///
    /// Without --dev this runs `npx skills add` for the published skills. With
    /// --dev it runs the skills repo's scripts/install-dev.sh, which symlinks
    /// the checkout into ~/.agents/skills and ~/.claude/skills so edits are live.
    Install {
        /// Symlink the local skills repo checkout instead of copying the published skills.
        #[arg(long)]
        dev: bool,
        /// Skills repo checkout to link with --dev (default: the boilersync-skills repo next to the current workspace).
        #[arg(long, value_name = "PATH")]
        source: Option<PathBuf>,
        /// Skill name to install. Repeatable. Default: all skills.
        #[arg(long = "skill", value_name = "NAME")]
        skills: Vec<String>,
    },
}

/// How a successfully dispatched command ended.
pub enum Outcome {
    Success,
    /// The command ran but reports a condition with exit status 1 (check-pull).
    Failure,
}

fn resolve_diff_mode(args: &DiffArgs) -> Result<OutputMode> {
    let selected: Vec<OutputMode> = [
        (OutputMode::Stat, args.stat),
        (OutputMode::NameStatus, args.name_status),
        (OutputMode::Patch, args.patch),
        (OutputMode::Json, args.json_output),
    ]
    .into_iter()
    .filter(|(_, enabled)| *enabled)
    .map(|(mode, _)| mode)
    .collect();
    if selected.len() > 1 {
        return Err(Error::Usage("Choose only one of --stat, --name-status, --patch, or --json.".to_string()));
    }
    Ok(selected.first().copied().unwrap_or(OutputMode::Stat))
}

fn none_if_empty(map: boilersync_core::jinja::Context) -> Option<boilersync_core::jinja::Context> {
    if map.is_empty() { None } else { Some(map) }
}

pub fn run(cli: &Cli) -> Result<Outcome> {
    let paths = Paths::from_env()?;
    match &cli.command {
        Command::Init(args) => {
            let options = InitOptions {
                template_variables: none_if_empty(parse_key_value_options(&args.variables)?),
                options: none_if_empty(parse_key_value_options(&args.runtime_options)?),
                no_input: args.no_input,
                monorepo: args.monorepo,
                ..InitOptions::new()
            };
            init(&paths, &args.template_ref, paths.cwd(), &options)?;
        }
        Command::CheckPull => {
            let status = check_pull(&paths, None)?;
            println!("{}", status_report(&status));
            if status.due_for_pull {
                return Ok(Outcome::Failure);
            }
        }
        Command::Diff(args) => {
            let options = DiffOptions {
                include_starter: args.include_starter,
                include_children: args.include_children,
                recursive: args.recursive,
                mode: resolve_diff_mode(args)?,
            };
            let result = diff(&paths, &options)?;
            if let Some(output_path) = &args.output_path {
                let output_path = boilersync_core::fsx::expanduser(&output_path.display().to_string());
                std::fs::write(&output_path, format!("{result}\n")).map_err(|e| Error::io(&output_path, e))?;
                println!("Wrote BoilerSync diff to {}", output_path.display());
            } else {
                println!("{result}");
            }
        }
        Command::Pull(args) => {
            pull(
                &paths,
                PullOptions {
                    template_ref: args.template_ref.clone(),
                    allow_non_empty: true,
                    include_starter: args.include_starter,
                    recursive: !args.no_children,
                    ..PullOptions::default()
                },
            )?;
        }
        Command::PullProposal(command) => match command {
            PullProposalCommand::Create { include_starter, include_children, workspace_dir, json_output } => {
                let proposal = create_pull_proposal(
                    &paths,
                    None,
                    &ProposalOptions {
                        include_starter: *include_starter,
                        include_children: *include_children,
                        workspace_dir: workspace_dir.clone(),
                    },
                )?;
                if *json_output {
                    println!("{}", proposal_to_json(&proposal)?);
                } else {
                    println!("{}", proposal_to_text(&proposal));
                }
            }
            PullProposalCommand::ApplyFile { proposal_dir, path } => {
                let target = apply_proposed_file(&paths, proposal_dir, path, None)?;
                println!("Applied proposed file: {}", target.display());
            }
            PullProposalCommand::ApplyPatch { patch_path } => {
                apply_patch_file(&paths, patch_path, None)?;
                println!("Applied patch: {}", patch_path.display());
            }
        },
        Command::Push(args) => push(&paths, &args.add_files)?,
        Command::Templates(command) => run_templates(&paths, command)?,
        Command::Skills(SkillsCommand::Install { dev, source, skills }) => {
            install_skills(
                &SkillsInstallOptions { dev: *dev, source: source.clone(), skills: skills.clone() },
                paths.cwd(),
            )?;
        }
    }
    Ok(Outcome::Success)
}

fn run_templates(paths: &Paths, command: &TemplatesCommand) -> Result<()> {
    match command {
        TemplatesCommand::Init { repo_url, repo_url_option, no_input } => {
            if repo_url.is_some() && repo_url_option.is_some() {
                return Err(Error::Usage("Provide either REPO_URL argument or --repo-url, not both.".to_string()));
            }
            init_templates(paths, repo_url_option.as_deref().or(repo_url.as_deref()), *no_input)
        }
        TemplatesCommand::List { json_output } => {
            let templates = list_local_templates(paths)?;
            if *json_output {
                let payload = serde_json::json!({
                    "template_root_dir": paths.template_root_dir(),
                    "count": templates.len(),
                    "templates": templates,
                });
                println!("{}", serde_json::to_string_pretty(&payload)?);
                return Ok(());
            }
            println!("Template root: {}", paths.template_root_dir().display());
            if templates.is_empty() {
                println!("No templates found.");
                return Ok(());
            }
            println!();
            for template in templates {
                println!("- {}", template.template_ref);
                println!("  path: {}", template.template_dir.display());
            }
            Ok(())
        }
        TemplatesCommand::Sources { json_output } => {
            let report = list_template_sources(paths)?;
            if *json_output {
                println!("{}", serde_json::to_string_pretty(&report)?);
                return Ok(());
            }
            println!("Template root: {}", report.template_root_dir.display());
            if report.sources.is_empty() {
                println!("No template sources found.");
                return Ok(());
            }
            println!();
            for source in &report.sources {
                println!("- {}/{}", source.org, source.repo);
                println!("  path: {}", source.path.display());
                println!("  remote: {}", source.remote_url.as_deref().unwrap_or("(none)"));
                println!("  branch: {}", source.branch.as_deref().unwrap_or("(unknown)"));
                println!("  commit: {}", source.commit.as_deref().unwrap_or("(unknown)"));
                println!("  templates: {}", source.template_count);
            }
            println!();
            if report.duplicate_remotes.is_empty() {
                println!("No duplicate remotes found.");
            } else {
                println!("Duplicate remotes:");
                for duplicate in &report.duplicate_remotes {
                    println!("- {}", duplicate.remote_url);
                    for path in &duplicate.paths {
                        println!("  - {}", path.display());
                    }
                }
            }
            Ok(())
        }
        TemplatesCommand::Details { template_ref, json_output } => {
            let details = get_template_details(paths, template_ref)?;
            if *json_output {
                println!("{}", serde_json::to_string_pretty(&details)?);
                return Ok(());
            }
            println!("Template: {}", details.template_ref);
            println!("Path: {}", details.template_dir.display());
            for (title, inputs) in [("Variables", &details.variables), ("Options", &details.options)] {
                println!();
                println!("{title}:");
                if inputs.is_empty() {
                    println!("- none");
                }
                for input in inputs {
                    println!("- {} ({})", input.name, if input.required { "required" } else { "optional" });
                }
            }
            Ok(())
        }
    }
}
