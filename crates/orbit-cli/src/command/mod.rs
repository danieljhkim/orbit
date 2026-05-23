pub mod definitions;
pub mod docs;
pub mod environment;
pub mod friction;
pub mod hook;
pub mod learning;
pub mod log;
pub mod mcp;
pub mod observe;
pub mod run;
pub mod search;
pub mod semantic;
pub mod task;
pub mod web;

pub use definitions::{activity, executor, job, policy, skill, tool};
pub use environment::{config, init, workspace};
pub use observe::{audit, graph};

use std::path::PathBuf;

use clap::{Parser, Subcommand};
use orbit_core::{OrbitError, OrbitRuntime};

pub trait Execute {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError>;
}

// Clap derive does not support per-variant subcommand `help_heading`
// (`next_help_heading` is args-only; `subcommand_help_heading` only renames
// the single `Commands:` block). To render grouped sections in `--help` we
// hand-roll the template below. Keep the variant order and the template's
// section order in sync when adding new commands — the variant order also
// determines where a missing-from-template command would otherwise appear.
#[derive(Parser)]
#[command(name = "orbit")]
#[command(about = "Orbit CLI", version)]
#[command(
    disable_help_subcommand = true,
    help_template = "\
{name} {version}

{usage-heading} {usage}

Environment:
  init        Initialize the global Orbit root (~/.orbit)
  workspace   Manage workspaces
  config      Show or update Orbit configuration
  semantic    Manage local orbit-search indexing

Operate:
  run         Run a workflow (ship, duel-plan, job)
  task        Create, update, and manage tasks
  docs        Search and manage the indexed docs corpus
  friction    Report, list, and triage friction records
  learning    Create, search, and curate project learnings

Observe:
  graph       Query the knowledge graph
  search      Search tasks, docs, learnings, and ADRs
  audit       Query the audit event log
  log         Tail the unified Orbit log feed

Definitions:
  activity    View activity definitions
  job         View job definitions
  tool        View tool registry
  policy      View filesystem policies
  executor    View executors

Services:
  mcp         Register MCP client integrations and run the MCP server
  hook        Run Orbit-owned editor hooks
  web         Run the Orbit dashboard

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
    // ── Environment ──
    Init(init::InitCommand),
    Workspace(workspace::WorkspaceCommand),
    Config(config::ConfigCommand),
    Semantic(semantic::SemanticCommand),

    // ── Operate ──
    Run(run::RunCommand),
    Task(Box<task::TaskCommand>),
    Search(search::SearchCommand),
    Docs(docs::DocsCommand),
    Friction(friction::FrictionCommand),
    Learning(learning::LearningCommand),

    // ── Observe ──
    Graph(graph::GraphCommand),
    Audit(audit::AuditCommand),
    Log(log::LogCommand),

    // ── Definitions ──
    Activity(activity::ActivityCommand),
    Job(job::JobCommand),
    Tool(tool::ToolCommand),
    Policy(policy::PolicyCommand),
    Executor(executor::ExecutorCommand),

    // ── Services ──
    Mcp(mcp::McpCommand),
    Hook(hook::HookCommand),
    Web(web::WebCommand),

    // ── hidden compatibility commands ──
    #[command(hide = true)]
    Skill(skill::SkillCommand),
    #[command(hide = true)]
    Logs(run::legacy_logs::LogsCommand),
    #[command(hide = true)]
    Artifacts(task::artifacts::ArtifactsCommand),
}

impl Execute for Commands {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        match self {
            Commands::Init(cmd) => cmd.execute(runtime),
            Commands::Workspace(cmd) => cmd.execute(runtime),
            Commands::Config(cmd) => cmd.execute(runtime),
            Commands::Semantic(cmd) => cmd.execute(runtime),
            Commands::Run(cmd) => cmd.execute(runtime),
            Commands::Task(cmd) => (*cmd).execute(runtime),
            Commands::Search(cmd) => cmd.execute(runtime),
            Commands::Docs(cmd) => cmd.execute(runtime),
            Commands::Friction(cmd) => cmd.execute(runtime),
            Commands::Learning(cmd) => cmd.execute(runtime),
            Commands::Graph(cmd) => cmd.execute(runtime),
            Commands::Audit(cmd) => cmd.execute(runtime),
            Commands::Log(cmd) => cmd.execute(runtime),
            Commands::Activity(cmd) => cmd.execute(runtime),
            Commands::Job(cmd) => cmd.execute(runtime),
            Commands::Tool(cmd) => cmd.execute(runtime),
            Commands::Policy(cmd) => cmd.execute(runtime),
            Commands::Executor(cmd) => cmd.execute(runtime),
            Commands::Mcp(cmd) => cmd.execute(runtime),
            Commands::Hook(cmd) => cmd.execute(runtime),
            Commands::Web(cmd) => cmd.execute(runtime),
            Commands::Skill(cmd) => cmd.execute(runtime),
            Commands::Logs(cmd) => cmd.execute(runtime),
            Commands::Artifacts(cmd) => cmd.execute(runtime),
        }
    }
}

#[cfg(test)]
mod tests;
