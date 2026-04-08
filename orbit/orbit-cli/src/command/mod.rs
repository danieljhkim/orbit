pub mod activity;
pub mod audit;
pub mod config;
pub mod init;
pub mod job;
pub mod job_run;
pub mod run;
pub mod skill;
pub mod task;
pub mod tool;
pub mod workspace;

use std::path::PathBuf;

use clap::{Parser, Subcommand};
use orbit_core::{OrbitError, OrbitRuntime};

pub trait Execute {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError>;
}

#[derive(Parser)]
#[command(name = "orbit")]
#[command(about = "Orbit CLI", version)]
#[command(
    disable_help_subcommand = true,
    help_template = "\
{name} {version}

{usage-heading} {usage}

Run workflows:
  run        Run a first-class workflow
  job        Define and run automation jobs
  job-run    Inspect and manage job run history

Manage work:
  task       Create, update, and manage tasks
  activity   Manage activity definitions
  skill      Manage agent skill definitions
  tool       Manage and run Orbit tools

Configure and inspect:
  config     Show or update Orbit configuration
  init       Initialize the global Orbit root (~/.orbit)
  workspace  Initialize and manage workspaces
  audit      Query the audit event log

Options:
{options}"
)]
pub struct Cli {
    /// Override the Orbit root directory (highest precedence)
    #[arg(long, global = true)]
    pub root: Option<PathBuf>,

    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Subcommand)]
pub enum Commands {
    // ── run workflows ──
    Run(run::RunCommand),
    Job(job::JobCommand),
    JobRun(job_run::JobRunCommand),

    // ── manage work ──
    Task(task::TaskCommand),
    Activity(activity::ActivityCommand),
    Skill(skill::SkillCommand),
    Tool(tool::ToolCommand),

    // ── configure and inspect ──
    Config(config::ConfigCommand),
    Init(init::InitCommand),
    Workspace(workspace::WorkspaceCommand),
    Audit(audit::AuditCommand),
}

impl Execute for Commands {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        match self {
            Commands::Config(cmd) => cmd.execute(runtime),
            Commands::Init(cmd) => cmd.execute(runtime),
            Commands::Tool(cmd) => cmd.execute(runtime),
            Commands::Task(cmd) => cmd.execute(runtime),
            Commands::Audit(cmd) => cmd.execute(runtime),
            Commands::Activity(cmd) => cmd.execute(runtime),
            Commands::Skill(cmd) => cmd.execute(runtime),
            Commands::Job(cmd) => cmd.execute(runtime),
            Commands::JobRun(cmd) => cmd.execute(runtime),
            Commands::Run(cmd) => cmd.execute(runtime),
            Commands::Workspace(cmd) => cmd.execute(runtime),
        }
    }
}
