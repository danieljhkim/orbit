fn assert_in_order(raw: &str, patterns: &[&str]) {
    let mut cursor = 0usize;
    for pattern in patterns {
        let relative = raw[cursor..].find(pattern).unwrap_or_else(|| {
            panic!("missing pattern '{pattern}' after byte offset {cursor}\n{raw}")
        });
        cursor += relative + pattern.len();
    }
}

#[test]
fn activity_assets_use_grouped_sections_and_literal_instruction_blocks() {
    let assets = [
        (
            "checkout_branch",
            include_str!("../assets/activities/checkout_branch.yaml"),
        ),
        (
            "commit_changes",
            include_str!("../assets/activities/commit_changes.yaml"),
        ),
        (
            "create_branch",
            include_str!("../assets/activities/create_branch.yaml"),
        ),
        (
            "start_task",
            include_str!("../assets/activities/start_task.yaml"),
        ),
        (
            "update_task",
            include_str!("../assets/activities/update_task.yaml"),
        ),
        (
            "dispatch_task",
            include_str!("../assets/activities/dispatch_task.yaml"),
        ),
        (
            "implement_change",
            include_str!("../assets/activities/implement_change.yaml"),
        ),
        (
            "merge_pr",
            include_str!("../assets/activities/merge_pr.yaml"),
        ),
        ("open_pr", include_str!("../assets/activities/open_pr.yaml")),
        (
            "oversee_orbit_operations",
            include_str!("../assets/activities/oversee_orbit_operations.yaml"),
        ),
        (
            "perform_maintenance",
            include_str!("../assets/activities/perform_maintenance.yaml"),
        ),
        (
            "review_pr",
            include_str!("../assets/activities/review_pr.yaml"),
        ),
        (
            "review_tasks",
            include_str!("../assets/activities/review_tasks.yaml"),
        ),
        (
            "run_tests",
            include_str!("../assets/activities/run_tests.yaml"),
        ),
    ];

    for (name, raw) in assets {
        assert_in_order(
            raw,
            &[
                "schema_version: 1",
                "# ---- metadata ----",
                "# ---- activity ----",
                "  # ---- registration ----",
                "  # ---- content ----",
                "  # ---- interface ----",
                "  # ---- execution ----",
            ],
        );

        if raw.contains("\n  instruction:") {
            assert!(
                raw.contains("\n  instruction: |\n"),
                "activity asset '{name}' should use a literal block for instruction"
            );
        }
    }
}

#[test]
fn dispatch_task_asset_accepts_shared_pipeline_base_input() {
    let raw = include_str!("../assets/activities/dispatch_task.yaml");
    assert!(
        raw.contains("base:\n        type: string"),
        "dispatch_task should accept the shared pipeline base input"
    );
    assert!(
        raw.contains("Output the selected task_id and the rationale comment."),
        "dispatch_task should instruct the agent to return only the selected task id and rationale"
    );
    assert!(
        raw.contains("Only use the tools listed in this activity's tools section."),
        "dispatch_task should explicitly prohibit unlisted tools"
    );
    assert!(
        raw.contains("Failed to retrieve open PRs. Cannot safely select task without PR overlap data."),
        "dispatch_task should fail fast when PR overlap data is unavailable"
    );
    assert!(
        raw.contains("- github.pr.list"),
        "dispatch_task should declare github.pr.list as an allowed tool"
    );
    assert!(
        raw.contains("Use the literal string \"None\" only when no rejected or backlog tasks exist"),
        "dispatch_task output schema should reserve task_id None for the no-tasks case"
    );
}

#[test]
fn pipeline_cli_activity_assets_document_workspace_path_input() {
    let assets = [
        include_str!("../assets/activities/run_tests.yaml"),
        include_str!("../assets/activities/checkout_branch.yaml"),
    ];

    for raw in assets {
        assert!(
            raw.contains("workspace_path:\n        type: string"),
            "pipeline CLI activity assets should document workspace_path input"
        );
    }
}

#[test]
fn worktree_pipeline_assets_document_isolated_worktree_contract() {
    let create_branch = include_str!("../assets/activities/create_branch.yaml");
    assert!(
        create_branch.contains("repo_root:\n        type: string"),
        "create_branch should declare repo_root output for downstream finalization"
    );
    assert!(
        create_branch.contains("branch:\n        type: string"),
        "create_branch should declare the task branch it checked out"
    );

    let checkout_branch = include_str!("../assets/activities/checkout_branch.yaml");
    assert!(
        checkout_branch.contains("cleanup_strategy:\n        type: string"),
        "checkout_branch should describe how the task worktree was finalized"
    );
    assert!(
        checkout_branch.contains("required:\n      - workspace_path\n      - repo_root"),
        "checkout_branch should require the worktree and repo_root inputs it consumes"
    );

    let commit_changes = include_str!("../assets/activities/commit_changes.yaml");
    assert!(
        commit_changes.contains("commit_message:\n        type: string"),
        "commit_changes should declare the deterministic commit message output"
    );
    assert!(
        commit_changes.contains("commit_sha:\n        type: string"),
        "commit_changes should declare the commit sha output"
    );
}

#[test]
fn job_assets_use_grouped_sections() {
    let assets = [
        (
            "job_oversee_orbit_operations",
            include_str!("../assets/jobs/job_oversee_orbit_operations.yaml"),
        ),
        (
            "job_perform_maintenance",
            include_str!("../assets/jobs/job_perform_maintenance.yaml"),
        ),
        (
            "job_review_tasks",
            include_str!("../assets/jobs/job_review_tasks.yaml"),
        ),
        (
            "job_task_pipeline",
            include_str!("../assets/jobs/job_task_pipeline.yaml"),
        ),
    ];

    for (name, raw) in assets {
        assert_in_order(
            raw,
            &[
                "schemaVersion: 1",
                "# ---- job ----",
                "job:",
                "  # ---- registration ----",
                "  # ---- execution ----",
                "  steps:",
            ],
        );
        assert!(
            raw.contains("\n  job_id: "),
            "job asset '{name}' should declare job_id inside the registration group"
        );
    }
}

#[test]
fn task_pipeline_starts_task_after_worktree_creation() {
    let raw = include_str!("../assets/jobs/job_task_pipeline.yaml");
    assert!(
        raw.contains("max_active_runs: 4"),
        "task pipeline should opt into explicit parallel run capacity"
    );
    assert_in_order(
        raw,
        &[
            "target_id: dispatch_task",
            "target_id: create_branch",
            "target_id: start_task",
            "target_id: implement_change",
        ],
    );
}
