use super::*;

#[test]
fn merges_object_defaults_with_explicit_object_input() {
    let defaults = json!({
        "mode": "pr",
        "base_branch": "agent-main",
        "max_tasks": 50,
        "max_bundle_size": 5
    });
    let explicit = json!({
        "base_branch": "main",
        "task_ids": ["T123"]
    });

    let merged = merge_job_input(Some(&defaults), &explicit);

    assert_eq!(
        merged,
        json!({
            "mode": "pr",
            "base_branch": "main",
            "max_tasks": 50,
            "max_bundle_size": 5,
            "task_ids": ["T123"]
        })
    );
}

#[test]
fn preserves_non_object_explicit_input_without_merging() {
    let defaults = json!({
        "mode": "pr",
        "max_tasks": 50
    });
    let explicit = json!("override");

    let merged = merge_job_input(Some(&defaults), &explicit);

    assert_eq!(merged, explicit);
}

#[test]
fn emit_job_event_dual_writes_step_lifecycle_to_audit_and_tracing() {
    let writer = test_writer("run-step-lifecycle");
    let captured = capture(|| {
        emit_job_event(
            &writer,
            Some("T-build"),
            V2AuditEventKind::StepStarted {
                step_id: "build".to_string(),
            },
        )
        .expect("StepStarted emit");
        emit_job_event(
            &writer,
            Some("T-build"),
            V2AuditEventKind::StepFinished {
                step_id: "build".to_string(),
                outcome: "success".to_string(),
            },
        )
        .expect("StepFinished emit");
    });

    assert_eq!(
        captured.targets(),
        vec![
            ("orbit.job.step_started", Level::INFO),
            ("orbit.job.step_finished", Level::INFO),
        ]
    );
    // Field schema: job_run_id matches V2AuditWriter::run_id() and aligns
    // with the cli_runner producer's naming so JSONL consumers can join
    // job-step events with subprocess events on the same key.
    assert_eq!(
        captured.events[0].field("job_run_id"),
        Some("run-step-lifecycle")
    );
    assert_eq!(captured.events[0].field("task_id"), Some("T-build"));
    assert_eq!(captured.events[0].field("step_id"), Some("build"));
    // The legacy `run_id` field name is not emitted: prevent regression.
    assert_eq!(captured.events[0].field("run_id"), None);
    assert_eq!(captured.events[1].field("step_id"), Some("build"));
    assert_eq!(captured.events[1].field("outcome"), Some("success"));
    assert_eq!(captured.events[1].field("success"), Some("true"));
    assert_eq!(captured.events[1].field("task_id"), Some("T-build"));

    let snapshot = writer.events_snapshot().expect("audit snapshot");
    assert!(matches!(
        snapshot[0].kind,
        V2AuditEventKind::StepStarted { ref step_id } if step_id == "build"
    ));
    assert!(matches!(
        snapshot[1].kind,
        V2AuditEventKind::StepFinished { ref step_id, ref outcome }
            if step_id == "build" && outcome == "success"
    ));
}

#[test]
fn emit_job_event_omits_task_id_when_none() {
    let writer = test_writer("run-no-task");
    let captured = capture(|| {
        emit_job_event(
            &writer,
            None,
            V2AuditEventKind::StepStarted {
                step_id: "anon".to_string(),
            },
        )
        .expect("StepStarted emit");
    });

    assert_eq!(captured.events[0].field("job_run_id"), Some("run-no-task"));
    // None should NOT serialize a `task_id` field at all.
    assert_eq!(captured.events[0].field("task_id"), None);
}

#[test]
fn emit_job_event_routes_step_finished_failure_to_error_level() {
    let writer = test_writer("run-fail");
    let captured = capture(|| {
        emit_job_event(
            &writer,
            None,
            V2AuditEventKind::StepFinished {
                step_id: "deploy".to_string(),
                outcome: "failed".to_string(),
            },
        )
        .expect("StepFinished failed emit");
    });

    assert_eq!(
        captured.targets(),
        vec![("orbit.job.step_finished", Level::ERROR)]
    );
    assert_eq!(captured.events[0].field("success"), Some("false"));
}

#[test]
fn emit_job_event_uses_warn_for_retry_skip_no_converge_and_error_for_denied() {
    let writer = test_writer("run-warn-error");
    let captured = capture(|| {
        emit_job_event(
            &writer,
            None,
            V2AuditEventKind::StepRetry {
                step_id: "flaky".to_string(),
                attempt: 2,
                next_backoff_ms: 250,
            },
        )
        .expect("StepRetry emit");
        emit_job_event(
            &writer,
            None,
            V2AuditEventKind::StepSkipped {
                step_id: "flaky".to_string(),
                reason: "when:false".to_string(),
            },
        )
        .expect("StepSkipped emit");
        emit_job_event(
            &writer,
            None,
            V2AuditEventKind::StepDenied {
                step_id: "flaky".to_string(),
                reason: "policy".to_string(),
            },
        )
        .expect("StepDenied emit");
        emit_job_event(
            &writer,
            None,
            V2AuditEventKind::LoopDidNotConverge {
                step_id: "loopy".to_string(),
                max_iterations: 5,
            },
        )
        .expect("LoopDidNotConverge emit");
    });

    assert_eq!(
        captured.targets(),
        vec![
            ("orbit.job.step_retry", Level::WARN),
            ("orbit.job.step_skipped", Level::WARN),
            ("orbit.job.step_denied", Level::ERROR),
            ("orbit.job.loop_did_not_converge", Level::WARN),
        ]
    );
    assert_eq!(captured.events[0].field("attempt"), Some("2"));
    assert_eq!(captured.events[0].field("next_backoff_ms"), Some("250"));
    assert_eq!(captured.events[1].field("reason"), Some("when:false"));
    assert_eq!(captured.events[3].field("max_iterations"), Some("5"));
}

#[test]
fn emit_job_event_projects_fanout_loop_and_join_phases() {
    let writer = test_writer("run-fanout");
    let captured = capture(|| {
        emit_job_event(
            &writer,
            None,
            V2AuditEventKind::FanoutDispatched {
                step_id: "scatter".to_string(),
                worker_count: 3,
            },
        )
        .expect("FanoutDispatched emit");
        emit_job_event(
            &writer,
            None,
            V2AuditEventKind::WorkerState {
                step_id: "scatter.worker".to_string(),
                worker_index: 1,
                state: "dispatched".to_string(),
            },
        )
        .expect("WorkerState emit");
        emit_job_event(
            &writer,
            None,
            V2AuditEventKind::FaninJoined {
                step_id: "scatter".to_string(),
                collected: 3,
                failed: 0,
            },
        )
        .expect("FaninJoined emit");
        emit_job_event(
            &writer,
            None,
            V2AuditEventKind::StepJoin {
                step_id: "merge".to_string(),
                mode: "all".to_string(),
                branch_outcomes: vec![
                    BranchOutcome {
                        branch_id: "a".to_string(),
                        outcome: "success".to_string(),
                    },
                    BranchOutcome {
                        branch_id: "b".to_string(),
                        outcome: "success".to_string(),
                    },
                ],
            },
        )
        .expect("StepJoin emit");
        emit_job_event(
            &writer,
            None,
            V2AuditEventKind::LoopIterationStart {
                step_id: "spin".to_string(),
                iteration: 1,
            },
        )
        .expect("LoopIterationStart emit");
        emit_job_event(
            &writer,
            None,
            V2AuditEventKind::LoopIterationEnd {
                step_id: "spin".to_string(),
                iteration: 1,
                broke: true,
            },
        )
        .expect("LoopIterationEnd emit");
    });

    assert_eq!(
        captured.targets(),
        vec![
            ("orbit.job.fanout", Level::INFO),
            ("orbit.job.worker_state", Level::INFO),
            ("orbit.job.fanout", Level::INFO),
            ("orbit.job.step_join", Level::INFO),
            ("orbit.job.loop_iteration", Level::INFO),
            ("orbit.job.loop_iteration", Level::INFO),
        ]
    );
    assert_eq!(captured.events[0].field("phase"), Some("dispatched"));
    assert_eq!(captured.events[0].field("worker_count"), Some("3"));
    assert_eq!(captured.events[1].field("worker_index"), Some("1"));
    assert_eq!(captured.events[1].field("state"), Some("dispatched"));
    assert_eq!(captured.events[2].field("phase"), Some("joined"));
    assert_eq!(captured.events[2].field("collected"), Some("3"));
    assert_eq!(captured.events[2].field("failed"), Some("0"));
    assert_eq!(captured.events[3].field("mode"), Some("all"));
    assert_eq!(captured.events[3].field("branch_count"), Some("2"));
    assert_eq!(captured.events[4].field("phase"), Some("start"));
    assert_eq!(captured.events[4].field("iteration"), Some("1"));
    assert_eq!(captured.events[5].field("phase"), Some("end"));
    assert_eq!(captured.events[5].field("broke"), Some("true"));
}

#[test]
fn emit_job_event_audit_snapshot_matches_direct_emit_for_same_kinds() {
    // The dual-write helper must not perturb the audit-store representation.
    // Compare the snapshot from emit_job_event against a writer that calls
    // V2AuditWriter::emit directly with the same kinds in the same order.
    let kinds = || {
        vec![
            V2AuditEventKind::StepStarted {
                step_id: "build".to_string(),
            },
            V2AuditEventKind::StepRetry {
                step_id: "build".to_string(),
                attempt: 1,
                next_backoff_ms: 100,
            },
            V2AuditEventKind::StepFinished {
                step_id: "build".to_string(),
                outcome: "success".to_string(),
            },
        ]
    };

    let dual = test_writer("run-snapshot");
    for k in kinds() {
        emit_job_event(&dual, None, k).expect("dual emit");
    }
    let direct = test_writer("run-snapshot");
    for k in kinds() {
        direct.emit(k).expect("direct emit");
    }

    let dual_snapshot = dual.events_snapshot().expect("dual snapshot");
    let direct_snapshot = direct.events_snapshot().expect("direct snapshot");
    let dual_json = serde_json::to_value(&dual_snapshot).expect("dual json");
    let direct_json = serde_json::to_value(&direct_snapshot).expect("direct json");
    // The envelope timestamps and event_ids will differ; strip those before
    // comparing the body shape.
    let stripped = |value: serde_json::Value| -> serde_json::Value {
        let mut value = value;
        if let serde_json::Value::Array(items) = &mut value {
            for item in items {
                if let serde_json::Value::Object(map) = item {
                    map.remove("ts");
                    map.remove("event_id");
                }
            }
        }
        value
    };
    assert_eq!(stripped(dual_json), stripped(direct_json));
}
