pub mod activity;
pub mod apply;
pub mod artifacts;
pub mod audit;
pub mod config;
pub mod describe;
pub mod duel;
pub mod executor;
pub mod get;
pub mod graph;
pub mod init;
pub mod job;
pub(crate) mod job_run_support;
pub mod logs;
pub mod mcp;
pub mod metrics;
pub mod policy;
pub mod run;
pub mod scoreboard;
pub mod serve;
pub mod ship;
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

Setup:
  init       Initialize the global Orbit root (~/.orbit)
  workspace  Initialize and manage workspaces
  mcp        Manage MCP client integrations
  config     Show or update Orbit configuration

Resources:
  task       Create, update, and manage tasks
  activity   Define, list, and run activities
  job        Define, list, and manage job workflows
  policy     Manage filesystem profile policies and runtime scoping
  executor   Manage executors
  tool       Manage tools and external MCP plugins

Workflows:
  run        Run a job workflow (supports run ship / run duel / run job / run <id>)

Inspect:
  audit      Query the audit event log
  metrics    Inspect token, tool-call, and knowledge-pack metrics
  scoreboard Generate read-only scoreboard summaries
  graph      Query the knowledge graph

Serve:
  serve      Serve Orbit outward (serve web / serve mcp)

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
    // ── setup ──
    Init(init::InitCommand),
    Workspace(workspace::WorkspaceCommand),
    Mcp(mcp::McpCommand),
    Config(config::ConfigCommand),

    // ── resources ──
    Task(task::TaskCommand),
    Activity(activity::ActivityCommand),
    Job(job::JobCommand),
    Tool(tool::ToolCommand),
    Executor(executor::ExecutorCommand),
    Policy(policy::PolicyCommand),

    // ── workflows ──
    Run(run::RunCommand),

    // ── inspect ──
    Audit(audit::AuditCommand),
    Metrics(metrics::MetricsCommand),
    Scoreboard(scoreboard::ScoreboardCommand),
    Graph(graph::GraphCommand),

    // ── serve ──
    Serve(serve::ServeCommand),

    // ── hidden compatibility commands ──
    #[command(hide = true)]
    Skill(skill::SkillCommand),
    #[command(hide = true)]
    Logs(logs::LogsCommand),
    #[command(hide = true)]
    Artifacts(artifacts::ArtifactsCommand),
}

impl Execute for Commands {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        match self {
            Commands::Init(cmd) => cmd.execute(runtime),
            Commands::Workspace(cmd) => cmd.execute(runtime),
            Commands::Mcp(cmd) => cmd.execute(runtime),
            Commands::Config(cmd) => cmd.execute(runtime),
            Commands::Task(cmd) => cmd.execute(runtime),
            Commands::Activity(cmd) => cmd.execute(runtime),
            Commands::Job(cmd) => cmd.execute(runtime),
            Commands::Tool(cmd) => cmd.execute(runtime),
            Commands::Executor(cmd) => cmd.execute(runtime),
            Commands::Policy(cmd) => cmd.execute(runtime),
            Commands::Run(cmd) => cmd.execute(runtime),
            Commands::Audit(cmd) => cmd.execute(runtime),
            Commands::Metrics(cmd) => cmd.execute(runtime),
            Commands::Scoreboard(cmd) => cmd.execute(runtime),
            Commands::Graph(cmd) => cmd.execute(runtime),
            Commands::Serve(cmd) => cmd.execute(runtime),
            Commands::Skill(cmd) => cmd.execute(runtime),
            Commands::Logs(cmd) => cmd.execute(runtime),
            Commands::Artifacts(cmd) => cmd.execute(runtime),
        }
    }
}

#[cfg(test)]
mod tests {
    use clap::Parser;

    use super::{Cli, Commands, mcp::McpSubcommand};

    #[test]
    fn cli_parses_top_level_mcp_command() {
        let cli = Cli::parse_from(["orbit", "mcp", "init"]);
        match cli.command {
            Commands::Mcp(command) => match command.command {
                McpSubcommand::Init(_) => {}
                _ => panic!("expected mcp init"),
            },
            _ => panic!("expected top-level mcp command"),
        }
    }

    #[test]
    fn cli_rejects_down_alias() {
        assert!(Cli::try_parse_from(["orbit", "mcp", "down"]).is_err());
    }
}
