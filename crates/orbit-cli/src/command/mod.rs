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
pub mod reconcile;
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
  config     Show or update Orbit configuration

Run:
  reconcile  Reconcile pending/running job runs
  ship       Ship tasks through the pipeline
  duel       Cross-agent scoring and planning
  job        Run job workflows
  run        Run a job workflow

Manage work:
  task       Create, update, and manage tasks
  graph      graph
  executor   executor
  policy     policy
  activity   activity

Resource Management:
  apply      apply
  get        get
  describe   describe
  
Inspect:
  audit      Query the audit event log
  metrics    Inspect token, tool-call, and knowledge-pack metrics
  scoreboard Generate read-only scoreboard summaries
  serve      Serve a local read-only web dashboard
  mcp        Serve the Orbit tool registry over Model Context Protocol

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
    Config(config::ConfigCommand),

    // ── run ──
    Run(run::RunCommand),
    Ship(ship::ShipCommand),
    Duel(duel::DuelCommand),

    // ── manage work ──
    Task(task::TaskCommand),
    Activity(activity::ActivityCommand),
    Job(job::JobCommand),
    Tool(tool::ToolCommand),
    Skill(skill::SkillCommand),
    Executor(executor::ExecutorCommand),
    Policy(policy::PolicyCommand),
    Graph(graph::GraphCommand),
    Mcp(mcp::McpCommand),

    // ── resource management ──
    Apply(apply::ApplyCommand),
    Get(get::GetCommand),
    Describe(describe::DescribeCommand),

    // ── operate ──
    Reconcile(reconcile::ReconcileCommand),

    // ── inspect ──
    Logs(logs::LogsCommand),
    Artifacts(artifacts::ArtifactsCommand),
    Audit(audit::AuditCommand),
    Metrics(metrics::MetricsCommand),
    Scoreboard(scoreboard::ScoreboardCommand),
    Serve(serve::ServeCommand),
}

impl Execute for Commands {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        match self {
            Commands::Init(cmd) => cmd.execute(runtime),
            Commands::Workspace(cmd) => cmd.execute(runtime),
            Commands::Config(cmd) => cmd.execute(runtime),
            Commands::Run(cmd) => cmd.execute(runtime),
            Commands::Ship(cmd) => cmd.execute(runtime),
            Commands::Duel(cmd) => cmd.execute(runtime),
            Commands::Reconcile(cmd) => cmd.execute(runtime),
            Commands::Task(cmd) => cmd.execute(runtime),
            Commands::Activity(cmd) => cmd.execute(runtime),
            Commands::Job(cmd) => cmd.execute(runtime),
            Commands::Tool(cmd) => cmd.execute(runtime),
            Commands::Skill(cmd) => cmd.execute(runtime),
            Commands::Executor(cmd) => cmd.execute(runtime),
            Commands::Policy(cmd) => cmd.execute(runtime),
            Commands::Graph(cmd) => cmd.execute(runtime),
            Commands::Mcp(cmd) => cmd.execute(runtime),
            Commands::Apply(cmd) => cmd.execute(runtime),
            Commands::Get(cmd) => cmd.execute(runtime),
            Commands::Describe(cmd) => cmd.execute(runtime),
            Commands::Logs(cmd) => cmd.execute(runtime),
            Commands::Artifacts(cmd) => cmd.execute(runtime),
            Commands::Audit(cmd) => cmd.execute(runtime),
            Commands::Metrics(cmd) => cmd.execute(runtime),
            Commands::Scoreboard(cmd) => cmd.execute(runtime),
            Commands::Serve(cmd) => cmd.execute(runtime),
        }
    }
}
