// Migrated from file/friction_store.rs per ORB-00231
use super::super::*;
use chrono::TimeZone;
use orbit_common::test_fixtures::TEST_CODEX_MODEL;
use orbit_common::types::{TaskPriority, TaskType};

#[test]
fn hub_migration_publishes_complete_tree_and_is_idempotent() {
    let temp = tempfile::tempdir().expect("tempdir");
    let legacy = temp.path().join("legacy");
    fs::create_dir_all(legacy.join("2026-07")).expect("legacy month");
    fs::write(legacy.join("tags.yaml"), "tooling: Tools\n").expect("taxonomy");
    fs::write(legacy.join("2026-07/F001.md"), "record\n").expect("record");

    let canonical = prepare_hub_friction_root(temp.path(), "ws_test", Some(&legacy))
        .expect("publish migration");
    assert_eq!(
        fs::read(canonical.join("2026-07/F001.md")).unwrap(),
        b"record\n"
    );
    assert_eq!(
        prepare_hub_friction_root(temp.path(), "ws_test", Some(&legacy)).unwrap(),
        canonical
    );
    assert_eq!(
        readable_hub_friction_root(temp.path(), "ws_test", Some(&legacy)).unwrap(),
        canonical
    );
}

#[test]
fn hub_migration_accepts_identical_interrupted_publish_and_commits_marker() {
    let temp = tempfile::tempdir().expect("tempdir");
    let legacy = temp.path().join("legacy");
    let canonical = canonical_hub_friction_root(temp.path(), "ws_test").unwrap();
    fs::create_dir_all(&legacy).unwrap();
    fs::create_dir_all(&canonical).unwrap();
    fs::write(legacy.join("tags.yaml"), "same\n").unwrap();
    fs::write(canonical.join("tags.yaml"), "same\n").unwrap();

    assert_eq!(
        readable_hub_friction_root(temp.path(), "ws_test", Some(&legacy)).unwrap(),
        legacy
    );
    prepare_hub_friction_root(temp.path(), "ws_test", Some(&legacy)).unwrap();
    assert_eq!(
        readable_hub_friction_root(temp.path(), "ws_test", Some(&legacy)).unwrap(),
        canonical
    );
}

#[test]
fn checkoutless_prepare_does_not_commit_an_unknown_legacy_migration() {
    let temp = tempfile::tempdir().expect("tempdir");
    let canonical = prepare_hub_friction_root(temp.path(), "ws_test", None)
        .expect("checkoutless canonical root");
    let marker = canonical
        .parent()
        .unwrap()
        .join(".migration-markers/ws_test.complete");
    assert!(canonical.is_dir());
    assert!(!marker.exists());

    let legacy = temp.path().join("legacy");
    fs::create_dir_all(&legacy).unwrap();
    fs::write(legacy.join("tags.yaml"), "legacy: state\n").unwrap();
    prepare_hub_friction_root(temp.path(), "ws_test", Some(&legacy))
        .expect("later known legacy migration");

    assert!(marker.exists());
    assert_eq!(
        fs::read(canonical.join("tags.yaml")).unwrap(),
        b"legacy: state\n"
    );
}

#[test]
fn hub_migration_conflict_fails_closed_and_preserves_legacy_reads() {
    let temp = tempfile::tempdir().expect("tempdir");
    let legacy = temp.path().join("legacy");
    let canonical = canonical_hub_friction_root(temp.path(), "ws_test").unwrap();
    fs::create_dir_all(&legacy).unwrap();
    fs::create_dir_all(&canonical).unwrap();
    fs::write(legacy.join("tags.yaml"), "legacy\n").unwrap();
    fs::write(canonical.join("tags.yaml"), "different\n").unwrap();

    let error = prepare_hub_friction_root(temp.path(), "ws_test", Some(&legacy))
        .expect_err("conflict must fail");
    assert!(error.to_string().contains("migration conflict"));
    assert_eq!(
        readable_hub_friction_root(temp.path(), "ws_test", Some(&legacy)).unwrap(),
        legacy
    );
    assert_eq!(
        fs::read(canonical.join("tags.yaml")).unwrap(),
        b"different\n"
    );
}

#[test]
fn id_allocation_resets_across_month_boundary() {
    let temp = tempfile::tempdir().expect("tempdir");
    let root = temp.path();
    let may = Utc.with_ymd_and_hms(2026, 5, 31, 23, 59, 0).unwrap();
    let june = Utc.with_ymd_and_hms(2026, 6, 1, 0, 0, 0).unwrap();

    let first =
        add_friction(root, params(TEST_CODEX_MODEL, may, vec!["tooling"])).expect("first add");
    let second =
        add_friction(root, params(TEST_CODEX_MODEL, may, vec!["docs"])).expect("second add");
    let next_month =
        add_friction(root, params(TEST_CODEX_MODEL, june, vec!["build"])).expect("next month add");

    assert_eq!(first.record.id, "F2026-05-001");
    assert_eq!(second.record.id, "F2026-05-002");
    assert_eq!(next_month.record.id, "F2026-06-001");
}

#[test]
fn tag_validation_uses_taxonomy_file() {
    let temp = tempfile::tempdir().expect("tempdir");
    let root = temp.path();
    ensure_default_tag_taxonomy(root).expect("taxonomy");
    let err = add_friction(
        root,
        params(TEST_CODEX_MODEL, Utc::now(), vec!["surprise-tag"]),
    )
    .expect_err("unknown tag fails");
    assert!(err.to_string().contains("valid tags"), "{err}");

    fs::write(root.join(TAGS_FILENAME), "surprise-tag: allowed\n").expect("rewrite taxonomy");
    add_friction(
        root,
        params(TEST_CODEX_MODEL, Utc::now(), vec!["surprise-tag"]),
    )
    .expect("new taxonomy tag succeeds");
}

#[test]
fn stats_render_zero_task_model_rate_as_na() {
    let temp = tempfile::tempdir().expect("tempdir");
    let root = temp.path();
    add_friction(root, params("grok", Utc::now(), vec!["tooling"])).expect("add friction");
    let mut done = task("T1", TaskStatus::Done);
    done.implemented_by = Some("codex".to_string());

    let stats = friction_stats(root, &[done]).expect("stats");
    assert_eq!(
        stats["by_family"]["grok"]["frictions_per_10_tasks"],
        json!("n/a")
    );
    assert_eq!(
        stats["by_family"]["codex"]["frictions_per_10_tasks"],
        json!(0.0)
    );
}

#[test]
fn stats_render_zero_rows_for_known_grok_family() {
    let temp = tempfile::tempdir().expect("tempdir");
    let root = temp.path();

    let stats = friction_stats(root, &[]).expect("stats");

    assert_eq!(stats["by_family"]["grok"]["frictions"], json!(0));
    assert_eq!(stats["by_family"]["grok"]["tasks_done"], json!(0));
    assert_eq!(
        stats["by_family"]["grok"]["frictions_per_10_tasks"],
        json!("n/a")
    );
}

fn params(model: &str, created_at: DateTime<Utc>, tags: Vec<&str>) -> FrictionAddParams {
    FrictionAddParams {
        model: model.to_string(),
        body: "Body".to_string(),
        tags: tags.into_iter().map(str::to_string).collect(),
        during_task: None,
        created_at,
    }
}

fn task(id: &str, status: TaskStatus) -> Task {
    let now = Utc.with_ymd_and_hms(2026, 5, 10, 0, 0, 0).unwrap();
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
        created_at: now,
        updated_at: now,
    }
}
