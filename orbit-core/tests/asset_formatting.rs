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
}

#[test]
fn dispatch_task_asset_checks_context_files_for_conflict() {
    let raw = include_str!("../assets/activities/dispatch_task.yaml");
    assert!(
        raw.contains("context_files"),
        "dispatch_task should explicitly reference context_files for conflict detection"
    );
    assert!(
        raw.contains("blocked file set") || raw.contains("conflict"),
        "dispatch_task should describe the conflict check logic"
    );
    assert!(
        raw.contains("empty context_files") || raw.contains("empty"),
        "dispatch_task should document that tasks with empty context_files are not skipped"
    );
}

#[test]
fn pipeline_cli_activity_assets_use_task_id_input() {
    let run_tests = include_str!("../assets/activities/run_tests.yaml");
    assert!(
        run_tests.contains("task_id:\n        type: string"),
        "run_tests should accept task_id input for template context resolution"
    );
    assert!(
        !run_tests.contains("workspace_path:\n        type: string"),
        "run_tests should not declare workspace_path input (resolved from task via engine)"
    );
}

#[test]
fn worktree_pipeline_assets_use_task_id_spine() {
    let create_branch = include_str!("../assets/activities/create_branch.yaml");
    assert!(
        create_branch.contains("task_id:\n        type: string"),
        "create_branch should require task_id input"
    );
    assert!(
        create_branch.contains("properties: {}"),
        "create_branch output should be empty (workspace_path/repo_root written to task)"
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
        commit_changes.contains("task_id:\n        type: string"),
        "commit_changes should require task_id input"
    );
    assert!(
        commit_changes.contains("properties: {}"),
        "commit_changes output should be empty (all fields on the task)"
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
fn task_pipeline_creates_branch_then_implements() {
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
            "target_id: implement_change",
        ],
    );
    // start_task was removed — create_branch now transitions to in-progress.
    assert!(
        !raw.contains("target_id: start_task"),
        "start_task step should no longer exist in the pipeline"
    );
}
