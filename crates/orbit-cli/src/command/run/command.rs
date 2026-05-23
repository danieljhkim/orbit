use clap::{Args, Subcommand};
use orbit_core::{OrbitError, OrbitRuntime};

use crate::command::Execute;

use super::duel;
use super::events::RunEventsArgs;
use super::history::RunHistoryArgs;
use super::job::JobRunArgs;
use super::logs::RunLogsArgs;
use super::ship;
use super::show::RunShowArgs;
use super::trace::RunTraceArgs;

const RUN_AFTER_HELP: &str = "\
Workflow entrypoints:
  orbit run ship [task_id ...]
  orbit run duel-plan <task_id>
  orbit run job <job_id> [--input key=value] [--json] [--debug]

Run history:
  orbit run history [--limit 50]
  orbit run history -j <job_id>
  orbit run show [run_id] [-s step_id] [--json]
  orbit run logs [run_id] [-s step_id] [--json]
  orbit run events [run_id] [-s step_id] [--type event_type] [--json]
  orbit run trace [run_id] [--json]
";

#[derive(Args)]
#[command(
    about = "Run a job workflow (supports run ship / duel-plan / job)",
    arg_required_else_help = true,
    subcommand_required = true,
    override_usage = "orbit run <COMMAND>",
    after_help = RUN_AFTER_HELP,
    help_template = "\
{about}

{usage-heading} {usage}

Workflows:
  ship       Ship backlog or explicitly selected tasks through the gated task pipeline
  duel-plan  Run a planning duel for one task
  job        Run an arbitrary job by ID

Audits:
  history    Show recent job runs, optionally filtered to one job
  show       Show structured state and step summary for a job run
  logs       Print raw stdout/stderr captured for a job run
  events     Show audit events recorded for a job run
  trace      Show audit event parent/child trace for a job run

Options:
{options}
{after-help}"
)]
pub struct RunCommand {
    #[command(subcommand)]
    pub command: RunSubcommand,
}

impl Execute for RunCommand {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        self.command.execute(runtime)
    }
}

#[derive(Subcommand)]
pub enum RunSubcommand {
    /// Ship backlog or explicitly selected tasks through the gated task pipeline
    Ship(ship::ShipCommand),
    /// Deprecated alias for `orbit run ship --mode local`
    #[command(name = "ship-local", hide = true)]
    ShipLocal(ship::LegacyShipLocalCommand),
    /// Run a planning duel for one task
    #[command(name = "duel-plan")]
    DuelPlan(duel::DuelPlanCommand),
    /// Show recent job runs, optionally filtered to one job
    History(RunHistoryArgs),
    /// Show structured state and step summary for a job run
    Show(RunShowArgs),
    /// Print raw stdout/stderr captured for a job run
    Logs(RunLogsArgs),
    /// Show audit events recorded for a job run
    Events(RunEventsArgs),
    /// Show audit event parent/child trace for a job run
    Trace(RunTraceArgs),
    /// Run an arbitrary job by ID
    Job(JobRunArgs),
}

impl Execute for RunSubcommand {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        match self {
            RunSubcommand::Ship(command) => command.execute(runtime),
            RunSubcommand::ShipLocal(command) => command.execute(runtime),
            RunSubcommand::DuelPlan(command) => command.execute(runtime),
            RunSubcommand::History(command) => command.execute(runtime),
            RunSubcommand::Show(command) => command.execute(runtime),
            RunSubcommand::Logs(command) => command.execute(runtime),
            RunSubcommand::Events(command) => command.execute(runtime),
            RunSubcommand::Trace(command) => command.execute(runtime),
            RunSubcommand::Job(command) => command.execute(runtime),
        }
    }
}
