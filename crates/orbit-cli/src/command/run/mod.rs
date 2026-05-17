pub mod duel;
mod events;
mod format;
mod history;
pub mod job;
pub mod legacy_logs;
mod logs;
pub mod ship;
mod show;
mod steps;
pub(crate) mod support;
mod trace;

pub use events::RunEventsArgs;
pub use history::RunHistoryArgs;
pub(crate) use job::job_run_to_json;
pub use job::{JobReplayArgs, JobRunArgs, JobRunPipelineWorkerArgs};
pub use logs::RunLogsArgs;
pub use show::RunShowArgs;
pub(crate) use show::{print_legacy_logs_summary, print_run_show};
pub use trace::RunTraceArgs;

use clap::{Args, Subcommand};
use orbit_core::{OrbitError, OrbitRuntime};

use crate::command::Execute;

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
    /// Deprecated alias for `orbit run ship`
    #[command(name = "ship-auto", hide = true)]
    ShipAuto(ship::LegacyShipAutoCommand),
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
            RunSubcommand::ShipAuto(command) => command.execute(runtime),
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

#[cfg(test)]
mod tests {
    use clap::Parser;
    use orbit_core::OrbitRuntime;
    use orbit_core::runtime::run_audit::RunAuditEvent;
    use serde_json::{Value, json};

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
    fn parses_ship_auto_mode_defaults() {
        let command = parse_run(&["orbit", "run", "ship"]);
        match command.command {
            RunSubcommand::Ship(args) => {
                assert!(args.task_ids.is_empty());
                assert_eq!(args.mode, ship::ShipMode::Pr);
                assert_eq!(args.base, None);
            }
            _ => panic!("expected ship"),
        }
    }

    #[test]
    fn parses_explicit_ship_defaults() {
        let command = parse_run(&["orbit", "run", "ship", "T1", "T2"]);
        match command.command {
            RunSubcommand::Ship(args) => {
                assert_eq!(args.task_ids, vec!["T1", "T2"]);
                assert_eq!(args.mode, ship::ShipMode::Pr);
                assert_eq!(args.base, None);
            }
            _ => panic!("expected ship"),
        }
    }

    #[test]
    fn parses_explicit_ship_mode_and_base() {
        let command = parse_run(&["orbit", "run", "ship", "-m", "local", "-b", "main", "T1"]);
        match command.command {
            RunSubcommand::Ship(args) => {
                assert_eq!(args.task_ids, vec!["T1"]);
                assert_eq!(args.mode, ship::ShipMode::Local);
                assert_eq!(args.base.as_deref(), Some("main"));
            }
            _ => panic!("expected ship"),
        }
    }

    #[test]
    fn parses_ship_auto_as_deprecated_top_level_subcommand() {
        let command = parse_run(&["orbit", "run", "ship-auto", "-m", "pr", "-b", "main"]);
        match command.command {
            RunSubcommand::ShipAuto(args) => {
                assert_eq!(args.mode, ship::ShipMode::Pr);
                assert_eq!(args.base.as_deref(), Some("main"));
            }
            _ => panic!("expected ship-auto"),
        }
    }

    #[test]
    fn parses_ship_local_as_deprecated_top_level_subcommand() {
        let command = parse_run(&["orbit", "run", "ship-local", "-b", "main", "T1"]);
        match command.command {
            RunSubcommand::ShipLocal(args) => {
                assert_eq!(args.task_ids, vec!["T1"]);
                assert_eq!(args.base.as_deref(), Some("main"));
            }
            _ => panic!("expected ship-local"),
        }
    }

    #[test]
    fn parses_duel_plan_as_top_level_subcommand() {
        let command = parse_run(&["orbit", "run", "duel-plan", "T1", "-b", "main"]);
        match command.command {
            RunSubcommand::DuelPlan(args) => {
                assert_eq!(args.task_id, "T1");
                assert_eq!(args.base.as_deref(), Some("main"));
            }
            _ => panic!("expected duel-plan"),
        }
    }

    #[test]
    fn parses_run_job_unchanged() {
        let command = parse_run(&["orbit", "run", "job", "task_auto_pipeline", "--json"]);
        match command.command {
            RunSubcommand::Job(args) => {
                assert_eq!(args.job_id, "task_auto_pipeline");
                assert!(args.json);
            }
            _ => panic!("expected job"),
        }
    }

    #[test]
    fn rejects_positional_job_fallback() {
        assert!(Cli::try_parse_from(["orbit", "run", "task_auto_pipeline", "--json"]).is_err());
    }

    #[test]
    fn parses_run_history_defaults() {
        let command = parse_run(&["orbit", "run", "history"]);
        match command.command {
            RunSubcommand::History(args) => {
                assert_eq!(args.job_id, None);
                assert_eq!(args.limit, history::DEFAULT_HISTORY_LIMIT);
                assert!(!args.json);
            }
            _ => panic!("expected history"),
        }
    }

    #[test]
    fn parses_run_history_job_filter() {
        let command = parse_run(&["orbit", "run", "history", "-j", "task_auto_pipeline"]);
        match command.command {
            RunSubcommand::History(args) => {
                assert_eq!(args.job_id.as_deref(), Some("task_auto_pipeline"));
                assert_eq!(args.limit, history::DEFAULT_HISTORY_LIMIT);
            }
            _ => panic!("expected history"),
        }
    }

    #[test]
    fn parses_run_show_latest() {
        let command = parse_run(&["orbit", "run", "show"]);
        match command.command {
            RunSubcommand::Show(args) => {
                assert_eq!(args.run_id, None);
                assert_eq!(args.step_id, None);
            }
            _ => panic!("expected show"),
        }
    }

    #[test]
    fn parses_run_show_run_id() {
        let command = parse_run(&["orbit", "run", "show", "jrun-1"]);
        match command.command {
            RunSubcommand::Show(args) => {
                assert_eq!(args.run_id.as_deref(), Some("jrun-1"));
                assert_eq!(args.step_id, None);
            }
            _ => panic!("expected show"),
        }
    }

    #[test]
    fn parses_run_show_step() {
        let command = parse_run(&["orbit", "run", "show", "jrun-1", "-s", "implement_one"]);
        match command.command {
            RunSubcommand::Show(args) => {
                assert_eq!(args.run_id.as_deref(), Some("jrun-1"));
                assert_eq!(args.step_id.as_deref(), Some("implement_one"));
            }
            _ => panic!("expected show"),
        }
    }

    #[test]
    fn parses_run_logs_latest() {
        let command = parse_run(&["orbit", "run", "logs"]);
        match command.command {
            RunSubcommand::Logs(args) => {
                assert_eq!(args.run_id, None);
                assert_eq!(args.step_id, None);
            }
            _ => panic!("expected logs"),
        }
    }

    #[test]
    fn parses_run_logs_run_id() {
        let command = parse_run(&["orbit", "run", "logs", "jrun-1"]);
        match command.command {
            RunSubcommand::Logs(args) => {
                assert_eq!(args.run_id.as_deref(), Some("jrun-1"));
                assert_eq!(args.step_id, None);
            }
            _ => panic!("expected logs"),
        }
    }

    #[test]
    fn parses_run_logs_step() {
        let command = parse_run(&["orbit", "run", "logs", "jrun-1", "-s", "implement_one"]);
        match command.command {
            RunSubcommand::Logs(args) => {
                assert_eq!(args.run_id.as_deref(), Some("jrun-1"));
                assert_eq!(args.step_id.as_deref(), Some("implement_one"));
            }
            _ => panic!("expected logs"),
        }
    }

    #[test]
    fn parses_run_events_latest() {
        let command = parse_run(&["orbit", "run", "events"]);
        match command.command {
            RunSubcommand::Events(args) => {
                assert_eq!(args.run_id, None);
                assert_eq!(args.step_id, None);
                assert_eq!(args.event_type, None);
                assert!(!args.json);
            }
            _ => panic!("expected events"),
        }
    }

    #[test]
    fn parses_run_events_filters() {
        let command = parse_run(&[
            "orbit",
            "run",
            "events",
            "jrun-1",
            "-s",
            "implement_one",
            "--type",
            "cli.invocation.finished",
            "--json",
        ]);
        match command.command {
            RunSubcommand::Events(args) => {
                assert_eq!(args.run_id.as_deref(), Some("jrun-1"));
                assert_eq!(args.step_id.as_deref(), Some("implement_one"));
                assert_eq!(args.event_type.as_deref(), Some("cli.invocation.finished"));
                assert!(args.json);
            }
            _ => panic!("expected events"),
        }
    }

    #[test]
    fn parses_run_trace_latest() {
        let command = parse_run(&["orbit", "run", "trace"]);
        match command.command {
            RunSubcommand::Trace(args) => {
                assert_eq!(args.run_id, None);
                assert!(!args.json);
            }
            _ => panic!("expected trace"),
        }
    }

    #[test]
    fn parses_run_trace_json() {
        let command = parse_run(&["orbit", "run", "trace", "jrun-1", "--json"]);
        match command.command {
            RunSubcommand::Trace(args) => {
                assert_eq!(args.run_id.as_deref(), Some("jrun-1"));
                assert!(args.json);
            }
            _ => panic!("expected trace"),
        }
    }

    #[test]
    fn run_events_filter_by_step_and_type() {
        let events = vec![
            test_audit_event("evt-run", None, "run.started", None),
            test_audit_event(
                "evt-step",
                Some("evt-run"),
                "step.started",
                Some("implement_one"),
            ),
            test_audit_event(
                "evt-cli",
                Some("evt-step"),
                "cli.invocation.finished",
                Some("implement_one"),
            ),
            test_audit_event(
                "evt-review",
                Some("evt-run"),
                "step.started",
                Some("review"),
            ),
        ];

        let filtered = events::filter_run_audit_events(
            events,
            Some("implement_one"),
            Some("cli.invocation.finished"),
        );
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].event_id, "evt-cli");
    }

    #[test]
    fn run_trace_tree_nests_children_and_keeps_orphans() {
        let events = vec![
            test_audit_event("evt-run", None, "run.started", None),
            test_audit_event(
                "evt-step",
                Some("evt-run"),
                "step.started",
                Some("implement_one"),
            ),
            test_audit_event(
                "evt-activity",
                Some("evt-step"),
                "activity.started",
                Some("implement_one"),
            ),
            test_audit_event("evt-orphan", Some("evt-missing"), "tool.denied", None),
        ];

        let tree = trace::build_trace_tree(&events);
        assert_eq!(tree.roots.len(), 1);
        assert_eq!(tree.roots[0].event.event_id, "evt-run");
        assert_eq!(tree.roots[0].children[0].event.event_id, "evt-step");
        assert_eq!(
            tree.roots[0].children[0].children[0].event.event_id,
            "evt-activity"
        );
        assert_eq!(tree.orphans.len(), 1);
        assert_eq!(tree.orphans[0].event.event_id, "evt-orphan");
    }

    #[test]
    fn resolve_run_step_prefers_audit_step_id() {
        let runtime = OrbitRuntime::in_memory().expect("build runtime");
        let yaml_path = runtime.data_root().join("qa_step_id.yaml");
        std::fs::write(
            &yaml_path,
            r#"schemaVersion: 2
kind: Job
metadata:
  name: qa_step_id
spec:
  state: enabled
  kind: workflow
  steps:
    - id: nap
      spec:
        type: deterministic
        action: sleep
        config: {}
"#,
        )
        .expect("write job yaml");
        let result = runtime
            .run_job_v2_from_yaml(&yaml_path, json!({ "seconds": 0 }), None)
            .expect("run job");
        let run = runtime.show_job_run(&result.run_id).expect("show run");

        let resolved = steps::resolve_run_step(&runtime, &run, "nap").expect("resolve step");
        assert_eq!(resolved.target_id, "nap");
        assert_eq!(resolved.target_type, "activity");
    }

    #[test]
    fn rejects_removed_duel_history_forms() {
        assert!(Cli::try_parse_from(["orbit", "run", "duel", "list"]).is_err());
        assert!(Cli::try_parse_from(["orbit", "run", "duel", "show"]).is_err());
    }

    fn test_audit_event(
        event_id: &str,
        parent_event_id: Option<&str>,
        event_type: &str,
        step_id: Option<&str>,
    ) -> RunAuditEvent {
        let body_kind = event_type.replace('.', "_");
        let mut raw = json!({
            "schemaVersion": 1,
            "event_type": event_type,
            "event_id": event_id,
            "ts": "2026-04-26T07:00:00Z",
            "run_id": "jrun-test",
            "agent_identity": "codex",
            "body_kind": body_kind,
        });
        if let Some(parent_event_id) = parent_event_id {
            raw.as_object_mut().unwrap().insert(
                "parent_event_id".to_string(),
                Value::String(parent_event_id.to_string()),
            );
        }
        if let Some(step_id) = step_id {
            raw.as_object_mut()
                .unwrap()
                .insert("step_id".to_string(), Value::String(step_id.to_string()));
        }
        RunAuditEvent {
            raw,
            event_id: event_id.to_string(),
            parent_event_id: parent_event_id.map(str::to_string),
            event_type: Some(event_type.to_string()),
            body_kind: Some(body_kind),
            timestamp: None,
            step_id: step_id.map(str::to_string),
        }
    }
}
