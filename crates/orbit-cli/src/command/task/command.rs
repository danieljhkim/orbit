use clap::{Args, Subcommand};
use orbit_core::{OrbitError, OrbitRuntime};

use crate::command::Execute;
use crate::command::locks::LocksCommand;

use super::add::TaskAddArgs;
use super::artifact::TaskArtifactCommand;
use super::export::TaskExportArgs;
use super::import::TaskImportArgs;
use super::lifecycle::{TaskArchiveArgs, TaskStartArgs};
use super::lint::TaskLintArgs;
use super::list::TaskListArgs;
use super::reindex::TaskReindexArgs;
use super::show::TaskShowArgs;
use super::update::TaskUpdateArgs;

#[derive(Args)]
#[command(about = "Create, update, and manage tasks")]
pub struct TaskCommand {
    #[command(subcommand)]
    pub command: TaskSubcommand,
}

impl Execute for TaskCommand {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        self.command.execute(runtime)
    }
}

#[derive(Subcommand)]
pub enum TaskSubcommand {
    /// Create a new task
    Add(TaskAddArgs),
    /// Manage task artifact files
    Artifact(TaskArtifactCommand),
    /// Inspect and release task file locks
    Locks(LocksCommand),
    /// List tasks with optional filters
    List(TaskListArgs),
    /// Show detailed information about a task
    Show(TaskShowArgs),
    /// Lint tasks for stale paths and vague acceptance criteria; `--fix` prunes stale context files
    Lint(TaskLintArgs),
    /// Update task fields and perform guarded status transitions
    /// (approve: proposed -> backlog, review -> done; reject: -> rejected; unarchive: archived -> backlog)
    Update(TaskUpdateArgs),
    /// Start work on a task, approving proposed work when needed
    Start(TaskStartArgs),
    /// Archive a task
    Archive(TaskArchiveArgs),
    /// Export task bundles to a portable tar.zst archive
    Export(TaskExportArgs),
    /// Import task bundles from a tar.zst archive
    Import(TaskImportArgs),
    /// Rebuild the registry index from on-disk task bundles
    Reindex(TaskReindexArgs),
}

impl Execute for TaskSubcommand {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        match self {
            TaskSubcommand::Add(args) => args.execute(runtime),
            TaskSubcommand::Artifact(cmd) => cmd.execute(runtime),
            TaskSubcommand::Locks(cmd) => cmd.execute(runtime),
            TaskSubcommand::List(args) => args.execute(runtime),
            TaskSubcommand::Show(args) => args.execute(runtime),
            TaskSubcommand::Lint(args) => args.execute(runtime),
            TaskSubcommand::Update(args) => args.execute(runtime),
            TaskSubcommand::Start(args) => args.execute(runtime),
            TaskSubcommand::Archive(args) => args.execute(runtime),
            TaskSubcommand::Export(args) => args.execute(runtime),
            TaskSubcommand::Import(args) => args.execute(runtime),
            TaskSubcommand::Reindex(args) => args.execute(runtime),
        }
    }
}
