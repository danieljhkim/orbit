pub mod activity;
pub mod adr;
pub mod audit;
pub mod auto_task;
pub mod config;
pub mod docs;
pub mod doctor;
pub mod executor;
pub mod friction;
pub mod gc;
pub mod hook;
pub mod host;
pub mod init;
pub mod job;
pub mod learning;
pub mod locks;
pub mod log;
pub mod mcp;
pub mod migrate;
pub mod operation;
pub mod operation_args;
pub mod policy;
pub mod routine;
pub mod run;
pub mod search;
pub mod semantic;
pub mod skill;
pub mod sweep;
pub mod task;
pub mod tool;
pub mod web;
pub mod workspace;

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
  host        Register and manage hub hosts
  config      Show or update Orbit configuration
  semantic    Manage local orbit-search indexing
  migrate     Apply or inspect pending .orbit layout/schema migrations

Operate:
  run         Run a workflow (ship, duel-plan, job)
  gc          Inspect and explicitly reap Orbit-managed garbage
  task        Create, update, and manage tasks
  docs        Search and manage the indexed docs corpus
  adr         List and inspect Architecture Decision Records
  friction    Report, list, and triage friction records
  learning    Create, search, and curate project learnings

Observe:
  search      Search tasks, docs, learnings, and ADRs
  audit       Query the audit event log
  log         Tail the unified Orbit log feed
  doctor      Diagnose workspace health (config, database, disk, indexes)

Definitions:
  activity    View activity definitions
  job         View job definitions
  tool        View tool registry
  policy      View filesystem policies
  executor    View executors

Scheduler:
  sweep       Fire due routines on this host (the scheduler pass)
  routine     Inspect and control scheduled routines on this host
  auto-task   Define recurring auto-task templates (the scheduler primitive)

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
    Host(host::HostCommand),
    Config(config::ConfigCommand),
    Semantic(semantic::SemanticCommand),
    Migrate(migrate::MigrateCommand),

    // ── Operate ──
    Run(run::RunCommand),
    Gc(gc::GcCommand),
    Task(Box<task::TaskCommand>),
    Docs(docs::DocsCommand),
    Adr(adr::AdrCommand),
    Friction(friction::FrictionCommand),
    Learning(learning::LearningCommand),

    // ── Observe ──
    Search(search::SearchCommand),
    Audit(audit::AuditCommand),
    Log(log::LogCommand),
    Doctor(doctor::DoctorCommand),

    // ── Definitions ──
    Activity(activity::ActivityCommand),
    Job(job::JobCommand),
    Tool(tool::ToolCommand),
    Policy(policy::PolicyCommand),
    Executor(executor::ExecutorCommand),

    // ── Scheduler ──
    Sweep(sweep::SweepCommand),
    Routine(routine::RoutineCommand),
    #[command(name = "auto-task")]
    AutoTask(auto_task::AutoTaskCommand),

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

#[cfg(test)]
mod tests;
