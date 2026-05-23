#![allow(missing_docs)]

mod duel;
mod format;
mod job;
mod ship;
mod support;

// Content moved from inline #[cfg(test)] mod tests in run/mod.rs per ORB-00221.

use clap::{Parser, error::ErrorKind};
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

fn assert_cli_rejects(args: &[&str], kind: ErrorKind, expected: &str) {
    let error = match Cli::try_parse_from(args.iter().copied()) {
        Ok(_) => panic!("form should be rejected"),
        Err(error) => error,
    };
    assert_eq!(error.kind(), kind, "{error}");
    let message = error.to_string();
    assert!(message.contains(expected), "{message}");
}

#[test]
fn parses_ship_auto_mode_defaults() {
    let command = parse_run(&["orbit", "run", "ship"]);
    match command.command {
        RunSubcommand::Ship(args) => {
            assert!(args.task_ids.is_empty());
            assert_eq!(args.mode, super::ship::ShipMode::Pr);
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
            assert_eq!(args.mode, super::ship::ShipMode::Pr);
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
            assert_eq!(args.mode, super::ship::ShipMode::Local);
            assert_eq!(args.base.as_deref(), Some("main"));
        }
        _ => panic!("expected ship"),
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
            assert!(!args.wait);
        }
        _ => panic!("expected duel-plan"),
    }
}

#[test]
fn parses_duel_plan_wait_flag() {
    let command = parse_run(&["orbit", "run", "duel-plan", "T1", "--wait"]);
    match command.command {
        RunSubcommand::DuelPlan(args) => {
            assert_eq!(args.task_id, "T1");
            assert!(args.wait);
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
    assert_cli_rejects(
        &["orbit", "run", "task_auto_pipeline", "--json"],
        ErrorKind::InvalidSubcommand,
        "unrecognized subcommand 'task_auto_pipeline'",
    );
}

#[test]
fn parses_run_history_defaults() {
    let command = parse_run(&["orbit", "run", "history"]);
    match command.command {
        RunSubcommand::History(args) => {
            assert_eq!(args.job_id, None);
            assert_eq!(args.limit, super::history::DEFAULT_HISTORY_LIMIT);
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
            assert_eq!(args.limit, super::history::DEFAULT_HISTORY_LIMIT);
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

    let filtered = super::events::filter_run_audit_events(
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

    let tree = super::trace::build_trace_tree(&events);
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

    let resolved = super::steps::resolve_run_step(&runtime, &run, "nap").expect("resolve step");
    assert_eq!(resolved.target_id, "nap");
    assert_eq!(resolved.target_type, "activity");
}

#[test]
fn rejects_removed_duel_history_forms() {
    for args in [
        &["orbit", "run", "duel", "list"][..],
        &["orbit", "run", "duel", "show"][..],
    ] {
        assert_cli_rejects(
            args,
            ErrorKind::InvalidSubcommand,
            "unrecognized subcommand 'duel'",
        );
    }
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
