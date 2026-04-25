use clap::{Args, Subcommand};
use orbit_core::{OrbitError, OrbitRuntime};

use crate::command::Execute;
use crate::command::{duel, job, ship};

const RUN_AFTER_HELP: &str = "\
Workflow entrypoints:
  orbit run ship <task_id> ...
  orbit run ship-auto
  orbit run duel-plan <task_id>
  orbit run job <job_id> [--input key=value] [--json] [--debug]

Direct form:
  orbit run <job_id> [--input key=value] [--json] [--debug]
    Equivalent to `orbit run job <job_id>`.

Run history:
  orbit job history <job_id>
  orbit job run-state <run_id>
";

#[derive(Args)]
#[command(
    about = "Run a job workflow (supports run ship / ship-auto / duel-plan / job / run <id>)",
    arg_required_else_help = true,
    args_conflicts_with_subcommands = true,
    override_usage = "orbit run <COMMAND>\n       orbit run <JOB_ID> [OPTIONS]",
    after_help = RUN_AFTER_HELP
)]
pub struct RunCommand {
    #[command(subcommand)]
    pub command: Option<RunSubcommand>,

    #[command(flatten)]
    pub positional: PositionalJobArgs,
}

impl Execute for RunCommand {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        match self.command {
            Some(command) => command.execute(runtime),
            None => execute_positional_job(self.positional, runtime),
        }
    }
}

#[derive(Subcommand)]
pub enum RunSubcommand {
    /// Ship explicitly selected tasks through the task pipeline
    Ship(ship::ShipCommand),
    /// Auto-select backlog tasks and ship them through the task pipeline
    #[command(name = "ship-auto")]
    ShipAuto(ship::ShipAutoCommand),
    /// Run a planning duel for one task
    #[command(name = "duel-plan")]
    DuelPlan(duel::DuelPlanCommand),
    /// Run an arbitrary job by ID
    Job(job::JobRunArgs),
}

impl Execute for RunSubcommand {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        match self {
            RunSubcommand::Ship(command) => command.execute(runtime),
            RunSubcommand::ShipAuto(command) => command.execute(runtime),
            RunSubcommand::DuelPlan(command) => command.execute(runtime),
            RunSubcommand::Job(command) => command.execute(runtime),
        }
    }
}

#[derive(Args, Default)]
pub struct PositionalJobArgs {
    /// Run the named job directly (equivalent to `orbit run job <JOB_ID>`)
    pub job_id: Option<String>,

    /// Input key=value pairs passed to all job steps (repeatable)
    #[arg(long)]
    pub input: Vec<String>,

    /// Output as JSON
    #[arg(long)]
    pub json: bool,

    /// Stream agent stderr to the terminal and tee stdout live for debugging
    #[arg(long)]
    pub debug: bool,
}

fn execute_positional_job(
    args: PositionalJobArgs,
    runtime: &OrbitRuntime,
) -> Result<(), OrbitError> {
    let Some(job_id) = args.job_id else {
        return Err(OrbitError::InvalidInput(
            "`orbit run` expects a workflow subcommand or job ID".to_string(),
        ));
    };

    ensure_positional_job_exists(runtime, &job_id)?;

    job::JobRunArgs {
        job_id,
        input: args.input,
        backend: None,
        json: args.json,
        debug: args.debug,
    }
    .execute(runtime)
}

fn ensure_positional_job_exists(runtime: &OrbitRuntime, job_id: &str) -> Result<(), OrbitError> {
    match runtime.show_job_catalog_entry(job_id) {
        Ok(_) => Ok(()),
        Err(OrbitError::JobNotFound(_)) => Err(OrbitError::InvalidInput(format!(
            "unknown `orbit run` target `{job_id}`\navailable subcommands: ship, ship-auto, duel-plan, job\ntip: use `orbit job list` to discover valid job ids"
        ))),
        Err(error) => Err(error),
    }
}

#[cfg(test)]
mod tests {
    use clap::Parser;

    use crate::command::{Cli, Commands};

    use super::*;

    fn parse_run(args: &[&str]) -> RunCommand {
        let cli = Cli::parse_from(args);
        match cli.command {
            Commands::Run(command) => command,
            _ => panic!("expected run command"),
        }
    }

    #[test]
    fn parses_explicit_ship_defaults() {
        let command = parse_run(&["orbit", "run", "ship", "T1", "T2"]);
        match command.command.expect("subcommand") {
            RunSubcommand::Ship(args) => {
                assert_eq!(args.task_ids, vec!["T1", "T2"]);
                assert_eq!(args.mode, ship::ShipMode::Pr);
                assert_eq!(args.base, "agent-main");
            }
            _ => panic!("expected ship"),
        }
    }

    #[test]
    fn parses_explicit_ship_mode_and_base() {
        let command = parse_run(&["orbit", "run", "ship", "-m", "local", "-b", "main", "T1"]);
        match command.command.expect("subcommand") {
            RunSubcommand::Ship(args) => {
                assert_eq!(args.task_ids, vec!["T1"]);
                assert_eq!(args.mode, ship::ShipMode::Local);
                assert_eq!(args.base, "main");
            }
            _ => panic!("expected ship"),
        }
    }

    #[test]
    fn parses_ship_auto_as_top_level_subcommand() {
        let command = parse_run(&["orbit", "run", "ship-auto", "-m", "pr", "-b", "main"]);
        match command.command.expect("subcommand") {
            RunSubcommand::ShipAuto(args) => {
                assert_eq!(args.mode, ship::ShipMode::Pr);
                assert_eq!(args.base, "main");
            }
            _ => panic!("expected ship-auto"),
        }
    }

    #[test]
    fn parses_duel_plan_as_top_level_subcommand() {
        let command = parse_run(&["orbit", "run", "duel-plan", "T1", "-b", "main"]);
        match command.command.expect("subcommand") {
            RunSubcommand::DuelPlan(args) => {
                assert_eq!(args.task_id, "T1");
                assert_eq!(args.base, "main");
            }
            _ => panic!("expected duel-plan"),
        }
    }

    #[test]
    fn parses_run_job_unchanged() {
        let command = parse_run(&["orbit", "run", "job", "task_auto_pipeline", "--json"]);
        match command.command.expect("subcommand") {
            RunSubcommand::Job(args) => {
                assert_eq!(args.job_id, "task_auto_pipeline");
                assert!(args.json);
            }
            _ => panic!("expected job"),
        }
    }

    #[test]
    fn parses_positional_job_fallback_unchanged() {
        let command = parse_run(&["orbit", "run", "task_auto_pipeline", "--json"]);
        assert!(command.command.is_none());
        assert_eq!(
            command.positional.job_id.as_deref(),
            Some("task_auto_pipeline")
        );
        assert!(command.positional.json);
    }

    #[test]
    fn rejects_removed_duel_history_forms() {
        assert!(Cli::try_parse_from(["orbit", "run", "duel", "list"]).is_err());
        assert!(Cli::try_parse_from(["orbit", "run", "duel", "show"]).is_err());
    }
}
