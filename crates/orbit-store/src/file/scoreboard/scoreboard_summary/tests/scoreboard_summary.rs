// Migrated from file/scoreboard/scoreboard_summary.rs per ORB-00231
use super::super::*;

#[test]
fn summary_overlays_audit_tool_call_counts_by_normalized_model() {
    let temp = tempfile::tempdir().expect("create tempdir");

    let summary = generate_summary_with_audit_tool_calls(
        temp.path(),
        &[],
        &[
            AuditToolCallCountsByRole {
                role: "codex / gpt-5".to_string(),
                total: 2,
                failed: 1,
            },
            AuditToolCallCountsByRole {
                role: "gpt-5".to_string(),
                total: 1,
                failed: 1,
            },
        ],
    )
    .expect("generate summary");

    let codex = summary.agents.get("codex").expect("codex summary");
    assert_eq!(codex.tool_calls, 3);
    assert_eq!(codex.failed_tool_calls, 2);
}

#[test]
fn summary_includes_zero_rows_for_known_families() {
    let temp = tempfile::tempdir().expect("create tempdir");

    let summary = generate_summary(temp.path(), &[]).expect("generate summary");

    let grok = summary.agents.get("grok").expect("grok summary");
    assert_eq!(grok.tasks_completed, 0);
    assert_eq!(grok.duels.participated, 0);
    assert_eq!(grok.task_review.threads, 0);
}

#[test]
fn summary_agent_keys_are_families_not_models() {
    let temp = tempfile::tempdir().expect("create tempdir");
    fs::create_dir_all(temp.path()).expect("create scoreboard dir");
    fs::write(
        temp.path().join("tokens.json"),
        r#"{
              "agents": [
                { "agent": "codex", "model": "gpt-5.5", "total_tokens": 1 },
                { "agent": "claude", "model": "claude-opus-4-7", "total_tokens": 1 },
                { "agent": "grok", "model": "grok-4", "total_tokens": 1 }
              ]
            }"#,
    )
    .expect("write tokens scoreboard");

    let summary = generate_summary(temp.path(), &[]).expect("generate summary");
    for forbidden in ["grok-4", "claude-opus-4-7", "gpt-5.5"] {
        assert!(
            !summary.agents.contains_key(forbidden),
            "model key leaked into summary agents: {forbidden}"
        );
    }
    for family in ["codex", "claude", "gemini", "grok"] {
        assert!(summary.agents.contains_key(family));
    }
}

#[test]
fn audit_tool_calls_do_not_double_count_token_scoreboard_tool_calls() {
    let temp = tempfile::tempdir().expect("create tempdir");
    fs::create_dir_all(temp.path()).expect("create scoreboard dir");
    fs::write(
        temp.path().join("tokens.json"),
        r#"{
              "agents": [
                {
                  "agent": "codex",
                  "model": "gpt-5",
                  "total_tokens": 10,
                  "total_output_tokens": 4,
                  "total_tool_calls": 5
                }
              ]
            }"#,
    )
    .expect("write tokens scoreboard");

    let summary = generate_summary_with_audit_tool_calls(
        temp.path(),
        &[],
        &[AuditToolCallCountsByRole {
            role: "gpt-5".to_string(),
            total: 3,
            failed: 2,
        }],
    )
    .expect("generate summary");

    let codex = summary.agents.get("codex").expect("codex summary");
    assert_eq!(codex.tokens.total, 10);
    assert_eq!(codex.tokens.output, 4);
    assert_eq!(codex.tool_calls, 5);
    assert_eq!(codex.failed_tool_calls, 2);
}

#[test]
fn audit_tool_calls_win_when_larger_than_token_scoreboard_tool_calls() {
    let temp = tempfile::tempdir().expect("create tempdir");
    fs::create_dir_all(temp.path()).expect("create scoreboard dir");
    fs::write(
        temp.path().join("tokens.json"),
        r#"{
              "agents": [
                {
                  "agent": "codex",
                  "model": "gpt-5",
                  "total_tokens": 10,
                  "total_output_tokens": 4,
                  "total_tool_calls": 2
                }
              ]
            }"#,
    )
    .expect("write tokens scoreboard");

    let summary = generate_summary_with_audit_tool_calls(
        temp.path(),
        &[],
        &[AuditToolCallCountsByRole {
            role: "gpt-5".to_string(),
            total: 7,
            failed: 3,
        }],
    )
    .expect("generate summary");

    let codex = summary.agents.get("codex").expect("codex summary");
    assert_eq!(codex.tokens.total, 10);
    assert_eq!(codex.tokens.output, 4);
    assert_eq!(codex.tool_calls, 7);
    assert_eq!(codex.failed_tool_calls, 3);
}

#[test]
fn summary_exposes_task_review_threads_separately_from_pr_comments() {
    let temp = tempfile::tempdir().expect("create tempdir");
    fs::create_dir_all(temp.path()).expect("create scoreboard dir");
    fs::write(
        temp.path().join("task_review.json"),
        r#"{"task-review-threads":{"gpt-reviewer":2}}"#,
    )
    .expect("write task review scoreboard");
    fs::write(
        temp.path().join("pr.json"),
        r#"{"pr-review-comments":{"gpt-reviewer":1}}"#,
    )
    .expect("write pr scoreboard");

    let summary = generate_summary(temp.path(), &[]).expect("generate summary");

    assert_eq!(summary.schema_version, CURRENT_SCHEMA_VERSION);
    let reviewer = summary.agents.get("codex").expect("reviewer summary");
    assert_eq!(reviewer.task_review.threads, 2);
    assert_eq!(reviewer.pr.review_comments, 1);
}

#[test]
fn summary_counts_tasks_created_and_planned_across_all_statuses() {
    let temp = tempfile::tempdir().expect("create tempdir");

    // Mix of statuses including ones excluded from `tasks_completed`.
    let tasks = vec![
        test_task("T1", TaskStatus::Done, "claude-opus-4-7", "claude-opus-4-7"),
        test_task("T2", TaskStatus::Backlog, "claude-opus-4-7", "gpt-5.5"),
        test_task(
            "T3",
            TaskStatus::Rejected,
            "claude-opus-4-7",
            "claude-opus-4-7",
        ),
        test_task("T4", TaskStatus::Friction, "gpt-5.5", "gpt-5.5"),
        test_task_no_attrib("T5", TaskStatus::Done),
    ];

    let summary = generate_summary(temp.path(), &tasks).expect("generate summary");

    let claude = summary.agents.get("claude").expect("claude summary");
    // Three tasks were created by claude (Done, Backlog, Rejected).
    assert_eq!(claude.tasks_created, 3);
    // Two were planned by claude (Done, Rejected).
    assert_eq!(claude.tasks_planned, 2);
    // Only Done counts toward Completed (no `task.model` here, so it
    // attributes via `implemented_by`-equivalent — but we left model None;
    // verify the attribution still ignores Backlog/Rejected/Friction).
    // T1 (Done) has implemented_by=None and model=None, so it does not
    // attribute to Completed.
    assert_eq!(claude.tasks_completed, 0);

    let codex = summary.agents.get("codex").expect("codex summary");
    assert_eq!(codex.tasks_created, 1); // T4
    assert_eq!(codex.tasks_planned, 2); // T2, T4

    // T5 has no created_by/planned_by — must not crash and must not
    // create a phantom agent bucket.
    assert!(!summary.agents.contains_key(""));
}

#[test]
fn summary_counts_knowledge_artifacts_by_author_family() {
    let temp = tempfile::tempdir().expect("create tempdir");
    let learnings = vec![
        test_learning("L-0015", Some("gpt-5.5")),
        test_learning("L-0016", Some("claude-opus-4-7")),
        test_learning("L-0003", None),
    ];
    let learning_votes = vec![("L-0015".to_string(), 2), ("L-0016".to_string(), 1)];
    let now = Utc::now();
    let adrs = vec![
        test_adr("ADR-0001", "codex", AdrStatus::Accepted, Some(now)),
        test_adr("ADR-0002", "gpt-5.5", AdrStatus::Proposed, None),
        test_adr(
            "ADR-0003",
            "claude-opus-4-7",
            AdrStatus::Superseded,
            Some(now),
        ),
    ];

    let summary = generate_summary_with_inputs(
        temp.path(),
        &[],
        &ScoreboardInputs {
            learnings: &learnings,
            learning_vote_counts: &learning_votes,
            adrs: &adrs,
            ..ScoreboardInputs::default()
        },
    )
    .expect("generate summary");

    let codex = summary.agents.get("codex").expect("codex summary");
    assert_eq!(codex.knowledge.learnings_created, 1);
    assert_eq!(codex.knowledge.learning_votes_received, 2);
    assert_eq!(codex.knowledge.adrs_created, 2);
    assert_eq!(codex.knowledge.adrs_accepted, 1);
    assert_eq!(codex.knowledge.adrs_proposed_open, 1);

    let claude = summary.agents.get("claude").expect("claude summary");
    assert_eq!(claude.knowledge.learnings_created, 1);
    assert_eq!(claude.knowledge.learning_votes_received, 1);
    assert_eq!(claude.knowledge.adrs_created, 1);
    assert_eq!(claude.knowledge.adrs_accepted, 1);
    assert_eq!(claude.knowledge.adrs_proposed_open, 0);
}

#[test]
fn summary_overlays_per_surface_tool_call_counts() {
    let temp = tempfile::tempdir().expect("create tempdir");

    let surface_rows = vec![
        AuditToolCallCountsBySurfaceAndRole {
            surface: "graph".to_string(),
            role: "claude-opus-4-7".to_string(),
            total: 56,
            failed: 2,
        },
        AuditToolCallCountsBySurfaceAndRole {
            surface: "graph".to_string(),
            role: "gpt-5.5".to_string(),
            total: 697,
            failed: 5,
        },
        AuditToolCallCountsBySurfaceAndRole {
            surface: "task".to_string(),
            role: "gpt-5.5".to_string(),
            total: 410,
            failed: 1,
        },
    ];

    let summary = generate_summary_with_inputs(
        temp.path(),
        &[],
        &ScoreboardInputs {
            audit_tool_calls_by_surface: &surface_rows,
            ..ScoreboardInputs::default()
        },
    )
    .expect("generate summary");

    let claude = summary.agents.get("claude").expect("claude summary");
    assert_eq!(claude.tool_calls_by_surface.get("graph").copied(), Some(56));
    assert_eq!(claude.tool_calls_by_surface.get("task"), None);

    let codex = summary.agents.get("codex").expect("codex summary");
    assert_eq!(codex.tool_calls_by_surface.get("graph").copied(), Some(697));
    assert_eq!(codex.tool_calls_by_surface.get("task").copied(), Some(410));
}

#[test]
fn summary_aggregates_workflows_run_for_successful_runs() {
    let temp = tempfile::tempdir().expect("create tempdir");

    let now = Utc::now();
    let runs = vec![
        test_job_run("r1", "task_local_pipeline", JobRunState::Success, now),
        test_job_run("r2", "task_local_pipeline", JobRunState::Success, now),
        test_job_run("r3", "task_local_pipeline", JobRunState::Failed, now),
        test_job_run("r4", "task_auto_pipeline", JobRunState::Success, now),
        test_job_run("r5", "task_pr_pipeline", JobRunState::Cancelled, now),
    ];

    let summary = generate_summary_with_inputs(
        temp.path(),
        &[],
        &ScoreboardInputs {
            job_runs: &runs,
            ..ScoreboardInputs::default()
        },
    )
    .expect("generate summary");

    // Sorted descending by count, then job_id ascending.
    assert_eq!(
        summary.workflows_run,
        vec![
            WorkflowRunCount {
                job_id: "task_local_pipeline".to_string(),
                count: 2,
            },
            WorkflowRunCount {
                job_id: "task_auto_pipeline".to_string(),
                count: 1,
            },
        ]
    );
}

#[test]
fn recent_7d_filters_tasks_workflows_and_surface_calls_by_window() {
    let temp = tempfile::tempdir().expect("create tempdir");

    let now = Utc::now();
    let inside = now - chrono::Duration::days(3);
    let outside = now - chrono::Duration::days(30);

    // Two created in-window, one outside.
    let mut t_inside = test_task(
        "T-in",
        TaskStatus::Done,
        "claude-opus-4-7",
        "claude-opus-4-7",
    );
    t_inside.created_at = inside;
    t_inside.updated_at = inside;

    let mut t_inside2 = test_task("T-in2", TaskStatus::Backlog, "gpt-5.5", "gpt-5.5");
    t_inside2.created_at = inside;

    let mut t_outside = test_task(
        "T-out",
        TaskStatus::Done,
        "claude-opus-4-7",
        "claude-opus-4-7",
    );
    t_outside.created_at = outside;
    t_outside.updated_at = outside; // legacy: no history transition
    // No history on t_outside — task_done_at falls back to updated_at.

    let tasks = vec![t_inside, t_inside2, t_outside];

    let surface_recent = vec![AuditToolCallCountsBySurfaceAndRole {
        surface: "graph".to_string(),
        role: "claude-opus-4-7".to_string(),
        total: 12,
        failed: 0,
    }];

    let runs = vec![
        test_job_run(
            "r-recent",
            "task_local_pipeline",
            JobRunState::Success,
            inside,
        ),
        test_job_run(
            "r-old",
            "task_local_pipeline",
            JobRunState::Success,
            outside,
        ),
    ];

    let summary = generate_summary_with_inputs(
        temp.path(),
        &tasks,
        &ScoreboardInputs {
            audit_tool_calls_by_surface_recent: &surface_recent,
            job_runs: &runs,
            now: Some(now),
            ..ScoreboardInputs::default()
        },
    )
    .expect("generate summary");

    let recent = summary
        .recent_7d
        .expect("recent_7d populated when now is set");
    // Two tasks created in window (T-in, T-in2). T-out is older.
    assert_eq!(recent.tasks_created, 2);
    // One task transitioned to Done in window (T-in). T-out's
    // updated_at is older than the window.
    assert_eq!(recent.tasks_completed, 1);
    // Surface row total flows through.
    assert_eq!(recent.tool_calls_by_surface.get("graph").copied(), Some(12));
    // Only the recent run counts.
    assert_eq!(recent.workflows_run, 1);
}

#[test]
fn summary_passes_top_tools_through_unchanged() {
    let temp = tempfile::tempdir().expect("create tempdir");

    let rows = vec![
        AuditTopToolCall {
            role: "gpt-5.5".to_string(),
            tool_name: "orbit.graph.show".to_string(),
            total: 355,
        },
        AuditTopToolCall {
            role: "claude-opus-4-7".to_string(),
            tool_name: "orbit.graph.search".to_string(),
            total: 45,
        },
    ];

    let summary = generate_summary_with_inputs(
        temp.path(),
        &[],
        &ScoreboardInputs {
            top_tool_calls: &rows,
            ..ScoreboardInputs::default()
        },
    )
    .expect("generate summary");

    assert_eq!(
        summary.top_tools,
        vec![
            TopToolCall {
                role: "gpt-5.5".to_string(),
                tool_name: "orbit.graph.show".to_string(),
                count: 355,
            },
            TopToolCall {
                role: "claude-opus-4-7".to_string(),
                tool_name: "orbit.graph.search".to_string(),
                count: 45,
            },
        ]
    );
}

#[test]
fn recent_7d_absent_when_now_not_provided() {
    let temp = tempfile::tempdir().expect("create tempdir");
    let summary = generate_summary(temp.path(), &[]).expect("generate summary");
    assert!(summary.recent_7d.is_none());
}

fn test_task(
    id: &str,
    status: TaskStatus,
    created_by: &str,
    planned_by: &str,
) -> orbit_common::types::Task {
    let mut task = test_task_no_attrib(id, status);
    task.created_by = Some(created_by.to_string());
    task.planned_by = Some(planned_by.to_string());
    task
}

fn test_task_no_attrib(id: &str, status: TaskStatus) -> orbit_common::types::Task {
    use orbit_common::types::{Task, TaskPriority, TaskType};
    Task {
        id: id.to_string(),
        title: id.to_string(),
        description: String::new(),
        acceptance_criteria: Vec::new(),
        tags: Vec::new(),
        plan: String::new(),
        execution_summary: String::new(),
        context_files: Vec::new(),
        created_by: None,
        planned_by: None,
        implemented_by: None,
        status,
        priority: TaskPriority::Medium,
        complexity: None,
        task_type: TaskType::Chore,
        pr_status: None,
        external_refs: Vec::new(),
        relations: Vec::new(),
        job_run_id: None,
        crew: None,
        created_at: Utc::now(),
        updated_at: Utc::now(),
    }
}

fn test_learning(id: &str, created_by: Option<&str>) -> Learning {
    use orbit_common::types::{LearningScope, LearningStatus};
    Learning {
        id: id.to_string(),
        status: LearningStatus::Active,
        scope: LearningScope::default(),
        summary: id.to_string(),
        body: String::new(),
        evidence: Vec::new(),
        supersedes: None,
        superseded_by: None,
        legacy_ids: Vec::new(),
        created_at: Utc::now(),
        updated_at: Utc::now(),
        created_by: created_by.map(str::to_string),
        priority: None,
    }
}

fn test_adr(id: &str, owner: &str, status: AdrStatus, accepted_at: Option<DateTime<Utc>>) -> Adr {
    Adr {
        id: id.to_string(),
        title: id.to_string(),
        status,
        owner: owner.to_string(),
        created_at: Utc::now(),
        accepted_at,
        last_updated: Utc::now(),
        related_features: Vec::new(),
        related_tasks: Vec::new(),
        tags: Vec::new(),
        paths: Vec::new(),
        supersedes: Vec::new(),
        superseded_by: None,
        legacy_ids: Vec::new(),
        validation_warnings: Vec::new(),
        legacy_validation: Default::default(),
    }
}

fn test_job_run(
    run_id: &str,
    job_id: &str,
    state: JobRunState,
    finished_at: chrono::DateTime<Utc>,
) -> JobRun {
    JobRun {
        run_id: run_id.to_string(),
        job_id: job_id.to_string(),
        attempt: 1,
        state,
        scheduled_at: finished_at,
        started_at: Some(finished_at),
        finished_at: Some(finished_at),
        duration_ms: Some(0),
        created_at: finished_at,
        pid: None,
        pid_start_time: None,
        input: None,
        retry_source_run_id: None,
        knowledge_metrics: None,
        resolved_crew: None,
        planner_model: None,
        implementer_model: None,
        reviewer_model: None,
        steps: Vec::new(),
    }
}

#[test]
fn summary_reads_legacy_task_review_messages_as_threads() {
    let temp = tempfile::tempdir().expect("create tempdir");
    fs::create_dir_all(temp.path()).expect("create scoreboard dir");
    fs::write(
        temp.path().join("task_review.json"),
        r#"{"task-review-messages":{"gpt-reviewer":2}}"#,
    )
    .expect("write legacy task review scoreboard");

    let summary = generate_summary(temp.path(), &[]).expect("generate summary");

    let reviewer = summary.agents.get("codex").expect("reviewer summary");
    assert_eq!(reviewer.task_review.threads, 2);
}

#[test]
fn summary_exposes_friction_reported_counts_from_records() {
    // Deterministic test per ORB-00143: seeds friction records for >=2 families
    // and asserts the generated scoreboard exposes nonzero `friction.reported`
    // (and zero for families with none). Uses the inputs path so it does not
    // depend on disk state or legacy task.status=friction.
    let temp = tempfile::tempdir().expect("create tempdir");

    let frictions: Vec<crate::friction_store::StoredFrictionRecord> = vec![
        crate::friction_store::StoredFrictionRecord {
            record: orbit_common::types::FrictionRecord {
                id: "F001".to_string(),
                model: "codex".to_string(),
                created_at: Utc::now(),
                status: orbit_common::types::FrictionStatus::Open,
                tags: vec![],
                resolved_at: None,
                during_task: None,
                resolved_by_task: None,
                body: "seed for codex family".to_string(),
            },
            path: std::path::PathBuf::from("frictions/2026-05/F001.md"),
        },
        crate::friction_store::StoredFrictionRecord {
            record: orbit_common::types::FrictionRecord {
                id: "F002".to_string(),
                model: "claude-3-opus".to_string(),
                created_at: Utc::now(),
                status: orbit_common::types::FrictionStatus::Resolved,
                tags: vec!["test".to_string()],
                resolved_at: Some(Utc::now()),
                during_task: None,
                resolved_by_task: None,
                body: "seed for claude family (normalized)".to_string(),
            },
            path: std::path::PathBuf::from("frictions/2026-05/F002.md"),
        },
    ];

    let summary = generate_summary_with_inputs(
        temp.path(),
        &[],
        &ScoreboardInputs {
            frictions: &frictions,
            ..ScoreboardInputs::default()
        },
    )
    .expect("generate summary with seeded frictions");

    let codex = summary.agents.get("codex").expect("codex summary");
    assert_eq!(
        codex.friction.reported, 1,
        "codex should report 1 friction record"
    );

    let claude = summary.agents.get("claude").expect("claude summary");
    assert_eq!(
        claude.friction.reported, 1,
        "claude (from claude-3-opus) should report 1"
    );

    let gemini = summary.agents.get("gemini").expect("gemini summary");
    assert_eq!(
        gemini.friction.reported, 0,
        "gemini with no records must expose 0, not fall back"
    );

    let grok = summary.agents.get("grok").expect("grok summary");
    assert_eq!(grok.friction.reported, 0);
}
