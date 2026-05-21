// Migrated from file/scoreboard/duel_scoreboard.rs per ORB-00231
use std::collections::BTreeSet;
use std::sync::{Arc, Barrier};
use std::thread;

use chrono::Utc;
use orbit_common::types::{
    Cost, ImplementerStats, Outcome, ReviewerStats, RoleAssignment, Roles, Scores, TaskClass,
    ValidIssuesBySeverity,
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

    assert!(aggregates.rows.iter().any(|row| row.role == "implementer"
        && row.agent == "grok"
        && row.model == "grok"
        && row.runs == 0));
    assert!(aggregates.rows.iter().any(|row| row.role == "reviewer"
        && row.agent == "grok"
        && row.model == "grok"
        && row.runs == 0));
    assert!(aggregates.rows.iter().any(|row| row.role == "arbiter"
        && row.agent == "grok"
        && row.model == "grok"
        && row.runs == 0));
}

fn test_run(run_id: String) -> DuelRun {
    DuelRun {
        run_id,
        task_id: "T-test".to_string(),
        completed_at: Utc::now(),
        task_class: TaskClass {
            scope: TaskScope::SingleFile,
            ambiguity: Some(Ambiguity::WellSpecified),
            source: "test".to_string(),
        },
        roles: Roles {
            implementer: role("codex", "gpt-5.5"),
            reviewer: role("claude", "opus"),
            arbiter: role("gemini", "pro"),
        },
        outcome: Outcome {
            decision: Decision::Approved,
            fix_loop_iterations: 0,
            fix_loop_exhausted: false,
            pr_number: Some(1),
            merged: true,
        },
        scores: Scores {
            implementer_score: 1.0,
            reviewer_score: 1.0,
        },
        reviewer_stats: ReviewerStats {
            total_comments: 0,
            valid: 0,
            invalid: 0,
            out_of_scope: 0,
            nitpick: 0,
            precision: 0.0,
            arbiter_override_rate: 0.0,
        },
        implementer_stats: ImplementerStats {
            valid_issues_against: ValidIssuesBySeverity::default(),
        },
        cost: Cost {
            wall_clock_seconds: 1,
            tokens_in: None,
            tokens_out: None,
        },
    }
}

fn role(agent: &str, model: &str) -> RoleAssignment {
    RoleAssignment {
        agent: agent.to_string(),
        model: model.to_string(),
    }
}
