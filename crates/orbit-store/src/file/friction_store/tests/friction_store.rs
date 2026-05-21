// Migrated from file/friction_store.rs per ORB-00231
use super::super::*;
use chrono::TimeZone;
use orbit_common::types::{TaskPriority, TaskType};

#[test]
fn id_allocation_resets_across_month_boundary() {
    let temp = tempfile::tempdir().expect("tempdir");
    let root = temp.path();
    let may = Utc.with_ymd_and_hms(2026, 5, 31, 23, 59, 0).unwrap();
    let june = Utc.with_ymd_and_hms(2026, 6, 1, 0, 0, 0).unwrap();

    let first = add_friction(root, params("gpt-5.5", may, vec!["tooling"])).expect("first add");
    let second = add_friction(root, params("gpt-5.5", may, vec!["docs"])).expect("second add");
    let next_month =
        add_friction(root, params("gpt-5.5", june, vec!["build"])).expect("next month add");

    assert_eq!(first.record.id, "F2026-05-001");
    assert_eq!(second.record.id, "F2026-05-002");
    assert_eq!(next_month.record.id, "F2026-06-001");
}

#[test]
fn tag_validation_uses_taxonomy_file() {
    let temp = tempfile::tempdir().expect("tempdir");
    let root = temp.path();
    ensure_default_tag_taxonomy(root).expect("taxonomy");
    let err = add_friction(root, params("gpt-5.5", Utc::now(), vec!["surprise-tag"]))
        .expect_err("unknown tag fails");
    assert!(err.to_string().contains("valid tags"), "{err}");

    fs::write(root.join(TAGS_FILENAME), "surprise-tag: allowed\n").expect("rewrite taxonomy");
    add_friction(root, params("gpt-5.5", Utc::now(), vec!["surprise-tag"]))
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
