// Migrated from file/scoreboard/planning_duel_scoreboard.rs per ORB-00231
use std::collections::BTreeSet;
use std::sync::{Arc, Barrier};
use std::thread;

use chrono::Utc;
use orbit_common::types::{
    AgentFamily, EfficiencyMetrics, PlannerSlot, PlanningEfficiency, PlanningOutcome,
    PlanningRoleAssignment, PlanningRoles,
};

use super::super::*;

#[test]
fn append_run_keeps_all_concurrent_writes() {
    let temp = tempfile::tempdir().expect("create tempdir");
    let scoreboard_dir = Arc::new(temp.path().to_path_buf());
    let writers = 32;
    let barrier = Arc::new(Barrier::new(writers));

    let handles: Vec<_> = (0..writers)
        .map(|index| {
            let scoreboard_dir = Arc::clone(&scoreboard_dir);
            let barrier = Arc::clone(&barrier);
            thread::spawn(move || {
                let run = test_run(format!("run-{index:02}"));
                barrier.wait();
                append_run(&scoreboard_dir, &run).expect("append run");
            })
        })
        .collect();

    for handle in handles {
        handle.join().expect("join writer thread");
    }

    let runs = load_runs(&scoreboard_dir).expect("load runs");
    assert_eq!(runs.len(), writers);

    let run_ids: BTreeSet<_> = runs.into_iter().map(|run| run.run_id).collect();
    let expected: BTreeSet<_> = (0..writers)
        .map(|index| format!("run-{index:02}"))
        .collect();
    assert_eq!(run_ids, expected);
}

#[test]
fn aggregate_emits_zero_rows_for_known_families() {
    let aggregates = aggregate(&[], AggregateFilter::default());

    assert!(
        aggregates
            .rows
            .iter()
            .any(|row| row.role == "planner_a" && row.family == "grok" && row.runs == 0)
    );
    assert!(
        aggregates
            .rows
            .iter()
            .any(|row| row.role == "planner_b" && row.family == "grok" && row.runs == 0)
    );
    assert!(
        aggregates
            .rows
            .iter()
            .any(|row| row.role == "arbiter" && row.family == "grok" && row.runs == 0)
    );
}

#[test]
fn aggregate_head_to_head_records_asymmetric_family_outcomes() {
    let runs = vec![
        test_run_with(
            "run-1",
            AgentFamily::Codex,
            AgentFamily::Claude,
            PlannerSlot::PlannerA,
        ),
        test_run_with(
            "run-2",
            AgentFamily::Codex,
            AgentFamily::Claude,
            PlannerSlot::PlannerB,
        ),
        test_run_with(
            "run-3",
            AgentFamily::Grok,
            AgentFamily::Codex,
            PlannerSlot::PlannerA,
        ),
    ];

    let matrix = aggregate_head_to_head(&runs);

    assert_eq!(
        matrix.families,
        vec![
            "codex".to_string(),
            "claude".to_string(),
            "gemini".to_string(),
            "grok".to_string(),
        ]
    );
    let codex_vs_claude = &matrix.cells["codex"]["claude"];
    assert_eq!(codex_vs_claude.wins, 1);
    assert_eq!(codex_vs_claude.losses, 1);
    assert_eq!(codex_vs_claude.runs, 2);

    let claude_vs_codex = &matrix.cells["claude"]["codex"];
    assert_eq!(claude_vs_codex.wins, 1);
    assert_eq!(claude_vs_codex.losses, 1);
    assert_eq!(claude_vs_codex.runs, 2);

    let grok_vs_codex = &matrix.cells["grok"]["codex"];
    assert_eq!(grok_vs_codex.wins, 1);
    assert_eq!(grok_vs_codex.losses, 0);
    assert_eq!(grok_vs_codex.runs, 1);

    let codex_vs_grok = &matrix.cells["codex"]["grok"];
    assert_eq!(codex_vs_grok.wins, 0);
    assert_eq!(codex_vs_grok.losses, 1);
    assert_eq!(codex_vs_grok.runs, 1);
}

fn test_run(run_id: String) -> PlanningDuelRun {
    test_run_with(
        &run_id,
        AgentFamily::Codex,
        AgentFamily::Claude,
        PlannerSlot::PlannerA,
    )
}

fn test_run_with(
    run_id: &str,
    planner_a: AgentFamily,
    planner_b: AgentFamily,
    winner: PlannerSlot,
) -> PlanningDuelRun {
    PlanningDuelRun {
        run_id: run_id.to_string(),
        task_id: "T-test".to_string(),
        completed_at: Utc::now(),
        roles: PlanningRoles {
            planner_a: role(planner_a),
            planner_b: role(planner_b),
            arbiter: role(AgentFamily::Gemini),
        },
        planner_a_artifact_path: "artifacts/planner-a.md".to_string(),
        planner_b_artifact_path: "artifacts/planner-b.md".to_string(),
        outcome: PlanningOutcome {
            winner,
            arbiter_rationale: "test winner".to_string(),
        },
        efficiency: PlanningEfficiency {
            planner_a: metrics(),
            planner_b: metrics(),
            arbiter: metrics(),
        },
    }
}

fn role(family: AgentFamily) -> PlanningRoleAssignment {
    PlanningRoleAssignment { family }
}

fn metrics() -> EfficiencyMetrics {
    EfficiencyMetrics {
        wall_clock_ms: 1_000,
        tool_call_count: 1,
        token_usage: None,
        byte_proxy_total: None,
    }
}
