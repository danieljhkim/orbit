use clap::{Args, Subcommand};
use orbit_core::{OrbitError, OrbitRuntime};

use crate::command::Execute;

use super::add::TaskAddArgs;
use super::artifact::TaskArtifactCommand;
use super::lifecycle::{
    TaskApproveArgs, TaskArchiveArgs, TaskDeleteArgs, TaskRejectArgs, TaskStartArgs,
    TaskUnarchiveArgs,
};
use super::lint::TaskLintArgs;
use super::list::{TaskListArgs, TaskLocksArgs};
use super::prune::TaskPruneContextArgs;
use super::review::ReviewThreadCommand;
use super::show::TaskShowArgs;
use super::templates::TaskTemplatesCommand;
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
    /// List tasks with optional filters
    List(TaskListArgs),
    /// Show files locked by active tasks
    Locks(TaskLocksArgs),
    /// Show detailed information about a task
    Show(TaskShowArgs),
    /// Lint a task for stale paths and vague acceptance criteria
    Lint(TaskLintArgs),
    /// Update task fields
    Update(TaskUpdateArgs),
    /// Start work on a task, approving proposed work when needed
    Start(TaskStartArgs),
    /// Approve a task (proposed → backlog, or review → done)
    Approve(TaskApproveArgs),
    /// Reject a task (proposed/friction/review/backlog/in-progress -> rejected)
    Reject(TaskRejectArgs),
    /// Archive a task
    Archive(TaskArchiveArgs),
    /// Unarchive a task (archived → backlog)
    Unarchive(TaskUnarchiveArgs),
    /// Delete a task permanently
    Delete(TaskDeleteArgs),
    /// Manage task templates
    Templates(TaskTemplatesCommand),
    /// Manage review threads on a task
    #[command(name = "review-thread")]
    ReviewThread(ReviewThreadCommand),
    /// Backfill: drop non-existent `context_files` entries from active tasks.
    /// Defaults to a dry-run report; pass `--write` to apply.
    #[command(name = "prune-context")]
    PruneContext(TaskPruneContextArgs),
}

impl Execute for TaskSubcommand {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        match self {
            TaskSubcommand::Add(args) => args.execute(runtime),
            TaskSubcommand::Artifact(cmd) => cmd.execute(runtime),
            TaskSubcommand::List(args) => args.execute(runtime),
            TaskSubcommand::Locks(args) => args.execute(runtime),
            TaskSubcommand::Show(args) => args.execute(runtime),
            TaskSubcommand::Lint(args) => args.execute(runtime),
            TaskSubcommand::Update(args) => args.execute(runtime),
            TaskSubcommand::Start(args) => args.execute(runtime),
            TaskSubcommand::Approve(args) => args.execute(runtime),
            TaskSubcommand::Reject(args) => args.execute(runtime),
            TaskSubcommand::Archive(args) => args.execute(runtime),
            TaskSubcommand::Unarchive(args) => args.execute(runtime),
            TaskSubcommand::Delete(args) => args.execute(runtime),
            TaskSubcommand::Templates(cmd) => cmd.execute(runtime),
            TaskSubcommand::ReviewThread(cmd) => cmd.execute(runtime),
            TaskSubcommand::PruneContext(args) => args.execute(runtime),
        }
    }
}

#[cfg(test)]
mod tests {
    use clap::{Parser, error::ErrorKind};

    use crate::command::Cli;

    #[test]
    fn task_help_describes_reject_transition_to_rejected() {
        let err = match Cli::try_parse_from(["orbit", "task", "--help"]) {
            Ok(_) => panic!("task help should exit before parsing a subcommand"),
            Err(err) => err,
        };

        assert_eq!(err.kind(), ErrorKind::DisplayHelp);

        let help = err.to_string();
        assert!(
            help.contains(
                "Reject a task (proposed/friction/review/backlog/in-progress -> rejected)"
            ),
            "{help}"
        );
        assert!(!help.contains("proposed → archived"), "{help}");
        assert!(!help.contains("review → backlog"), "{help}");
    }
}
