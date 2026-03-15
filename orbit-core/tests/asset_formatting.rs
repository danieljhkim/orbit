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
            "create_branch",
            include_str!("../assets/activities/create_branch.yaml"),
        ),
        (
            "dispatch_task",
            include_str!("../assets/activities/dispatch_task.yaml"),
        ),
        (
            "implement_change",
            include_str!("../assets/activities/implement_change.yaml"),
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
                "  # ---- identity ----",
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
fn identity_assets_use_grouped_sections_and_literal_description_blocks() {
    let assets = [
        ("lamport", include_str!("../assets/identities/lamport.yaml")),
        ("linus", include_str!("../assets/identities/linus.yaml")),
        ("prii", include_str!("../assets/identities/prii.yaml")),
        ("steve", include_str!("../assets/identities/steve.yaml")),
    ];

    for (name, raw) in assets {
        assert_in_order(
            raw,
            &[
                "# ---- identity ----",
                "identity:",
                "# ---- personality ----",
                "personality:",
            ],
        );
        assert!(
            raw.contains("\n  description: |\n"),
            "identity asset '{name}' should use a literal block for description"
        );
        if raw.contains("\nbehavior:\n") {
            assert!(
                raw.contains("\n# ---- behavior ----\nbehavior:\n"),
                "identity asset '{name}' should group behavior under a section header"
            );
        }
    }
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
                "  # ---- identity ----",
                "  # ---- execution ----",
                "  steps:",
            ],
        );
        assert!(
            raw.contains("\n  job_id: "),
            "job asset '{name}' should declare job_id inside the identity group"
        );
    }
}
