use orbit_common::types::{Task, TaskPriority, TaskStatus, TaskType};
use serde_json::{Value, json};

use super::super::task_pilot::{apply, prepare};
use crate::OrbitRuntime;
use crate::command::task::TaskAddParams;
use crate::runtime::v2_host::test_support::{runtime_with_workspace_layout, write_workspace_file};

fn seed_task(
    runtime: &OrbitRuntime,
    title: &str,
    status: TaskStatus,
    tags: &[&str],
    context_files: &[&str],
) -> Task {
    runtime
        .add_task(TaskAddParams {
            title: title.to_string(),
            description: format!("Fixture task: {title}"),
            acceptance_criteria: vec!["The fixture outcome is observable.".to_string()],
            tags: tags.iter().map(|tag| (*tag).to_string()).collect(),
            plan: "Inspect and update the fixture.".to_string(),
            context_files: context_files
                .iter()
                .map(|selector| (*selector).to_string())
                .collect(),
            workspace_path: Some(".".to_string()),
            priority: TaskPriority::Medium,
            task_type: Some(TaskType::Chore),
            status: Some(status),
            ..TaskAddParams::default()
        })
        .expect("seed task")
}

fn prepared(runtime: &OrbitRuntime, repo_root: &std::path::Path, task_ids: &[String]) -> Value {
    prepare(
        runtime,
        "prepare_task_pilot",
        &json!({
            "task_ids": task_ids,
            "workspace_path": repo_root,
        }),
    )
    .expect("prepare explicit task-pilot selection")
}

fn selector_assessment(task: &Task, after: Vec<&str>) -> Value {
    json!({
        "task_id": task.id,
        "context_files_before": task.context_files,
        "context_files_after": after,
        "disposition": "selectors",
        "recommended_crew": "luna",
        "recommended_complexity": "medium",
        "blocked_by": [],
        "duplicate_of": null,
        "already_landed": null,
        "adr_conflicts": [],
        "utility_warnings": [],
        "surface_warnings": [],
    })
}

fn partition_result(partition_index: usize, task_ids: &[String], tasks: Vec<Value>) -> Value {
    json!({
        "partition_index": partition_index,
        "task_ids": task_ids,
        "tasks": tasks,
        "summary": "fixture partition",
    })
}

#[test]
fn automatic_discovery_filters_status_context_and_no_diff_tags_then_partitions() {
    let (_root, runtime, repo_root) = runtime_with_workspace_layout();
    write_workspace_file(&repo_root, "src/existing.rs");
    let mut eligible = Vec::new();
    for index in 0..7 {
        eligible.push(seed_task(
            &runtime,
            &format!("eligible-{index}"),
            if index % 2 == 0 {
                TaskStatus::Proposed
            } else {
                TaskStatus::Backlog
            },
            &[],
            &[],
        ));
    }
    let active = seed_task(&runtime, "active", TaskStatus::InProgress, &[], &[]);
    let review = seed_task(&runtime, "review", TaskStatus::Review, &[], &[]);
    let terminal = seed_task(&runtime, "terminal", TaskStatus::Done, &[], &[]);
    let no_diff = seed_task(
        &runtime,
        "no-diff",
        TaskStatus::Backlog,
        &["no-diff-needed"],
        &[],
    );
    let scoped = seed_task(
        &runtime,
        "already-scoped",
        TaskStatus::Backlog,
        &[],
        &["file:src/existing.rs"],
    );

    let output = prepare(
        &runtime,
        "prepare_task_pilot",
        &json!({ "workspace_path": repo_root }),
    )
    .expect("automatic discovery");

    assert_eq!(output["mode"], "automatic");
    assert_eq!(output["task_count"], 7);
    assert_eq!(output["partition_count"], 2);
    assert_eq!(
        output["partitions"][0]["task_ids"]
            .as_array()
            .unwrap()
            .len(),
        5
    );
    assert_eq!(
        output["partitions"][1]["task_ids"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    let selected = output["task_ids"].as_array().expect("selected task ids");
    for task in eligible {
        assert!(selected.iter().any(|value| value == &json!(task.id)));
    }
    let excluded = output["excluded"].as_array().expect("excluded entries");
    for task in [active, review, terminal] {
        assert!(excluded.iter().any(|entry| {
            entry["task_id"] == task.id && entry["reason"] == "status_not_eligible"
        }));
    }
    assert!(
        excluded
            .iter()
            .any(|entry| { entry["task_id"] == no_diff.id && entry["reason"] == "no_diff_task" })
    );
    assert!(excluded.iter().any(|entry| {
        entry["task_id"] == scoped.id && entry["reason"] == "context_files_not_empty"
    }));
}

#[test]
fn explicit_mode_selects_exact_ids_even_with_nonempty_context_or_active_status() {
    let (_root, runtime, repo_root) = runtime_with_workspace_layout();
    write_workspace_file(&repo_root, "src/existing.rs");
    let scoped = seed_task(
        &runtime,
        "scoped",
        TaskStatus::Review,
        &[],
        &["file:src/existing.rs"],
    );
    let backlog = seed_task(&runtime, "backlog", TaskStatus::Backlog, &[], &[]);

    let output = prepared(
        &runtime,
        &repo_root,
        &[scoped.id.clone(), backlog.id.clone()],
    );

    assert_eq!(output["mode"], "explicit");
    assert_eq!(output["task_ids"], json!([scoped.id, backlog.id]));
    assert_eq!(output["excluded"], json!([]));
}

#[test]
fn apply_validates_all_results_then_mutates_context_files_only() {
    let (_root, runtime, repo_root) = runtime_with_workspace_layout();
    write_workspace_file(&repo_root, "src/alpha.rs");
    let alpha = seed_task(&runtime, "alpha", TaskStatus::Backlog, &["pilot"], &[]);
    let operational = seed_task(
        &runtime,
        "operational",
        TaskStatus::Backlog,
        &["host-operation"],
        &[],
    );
    let task_ids = vec![alpha.id.clone(), operational.id.clone()];
    let prepared_snapshot = prepared(&runtime, &repo_root, &task_ids);
    let before_alpha = runtime.get_task(&alpha.id).expect("alpha before");
    let before_operational = runtime
        .get_task(&operational.id)
        .expect("operational before");
    let result = partition_result(
        0,
        &task_ids,
        vec![
            selector_assessment(&alpha, vec!["file:src/alpha.rs"]),
            json!({
                "task_id": operational.id,
                "context_files_before": [],
                "context_files_after": [],
                "disposition": "host_operational",
                "evidence": "Changes host service state only; no repository artifact is modified.",
                "recommended_crew": "luna",
                "recommended_complexity": "low",
                "blocked_by": [],
                "duplicate_of": null,
                "already_landed": null,
                "adr_conflicts": [],
                "utility_warnings": ["requires host access"],
                "surface_warnings": [],
            }),
        ],
    );

    let output = apply(
        &runtime,
        "apply_task_pilot_results",
        &json!({
            "prepared": prepared_snapshot,
            "results": [result],
            "workspace_path": repo_root,
            "crew": "luna",
        }),
    )
    .expect("apply validated pilot results");

    let after_alpha = runtime.get_task(&alpha.id).expect("alpha after");
    let after_operational = runtime
        .get_task(&operational.id)
        .expect("operational after");
    assert_eq!(after_alpha.context_files, vec!["file:src/alpha.rs"]);
    assert_eq!(after_operational.context_files, Vec::<String>::new());
    assert_eq!(after_alpha.title, before_alpha.title);
    assert_eq!(after_alpha.status, before_alpha.status);
    assert_eq!(after_alpha.tags, before_alpha.tags);
    assert_eq!(after_alpha.plan, before_alpha.plan);
    assert_eq!(after_operational.title, before_operational.title);
    assert_eq!(after_operational.status, before_operational.status);
    assert_eq!(output["status"], "success");
    assert_eq!(output["crew"], "luna");
    assert_eq!(output["tasks"][0]["context_files_before"], json!([]));
    assert_eq!(
        output["tasks"][0]["context_files_after"],
        json!(["file:src/alpha.rs"])
    );
    assert_eq!(output["tasks"][0]["applied"], true);
    assert_eq!(output["tasks"][1]["applied"], false);
    assert_eq!(
        output["tasks"][1]["utility_warnings"],
        json!(["requires host access"])
    );
}

#[test]
fn invalid_selector_in_later_assessment_prevents_every_mutation() {
    let (_root, runtime, repo_root) = runtime_with_workspace_layout();
    write_workspace_file(&repo_root, "src/alpha.rs");
    let alpha = seed_task(&runtime, "alpha", TaskStatus::Backlog, &[], &[]);
    let beta = seed_task(&runtime, "beta", TaskStatus::Backlog, &[], &[]);
    let task_ids = vec![alpha.id.clone(), beta.id.clone()];
    let prepared_snapshot = prepared(&runtime, &repo_root, &task_ids);
    let result = partition_result(
        0,
        &task_ids,
        vec![
            selector_assessment(&alpha, vec!["file:src/alpha.rs"]),
            selector_assessment(&beta, vec!["file:src/missing.rs"]),
        ],
    );

    let error = apply(
        &runtime,
        "apply_task_pilot_results",
        &json!({
            "prepared": prepared_snapshot,
            "results": [result],
            "workspace_path": repo_root,
            "crew": "luna",
        }),
    )
    .expect_err("missing selector target must fail closed");

    assert!(error.to_string().contains("does not resolve"));
    assert!(
        runtime
            .get_task(&alpha.id)
            .unwrap()
            .context_files
            .is_empty()
    );
    assert!(runtime.get_task(&beta.id).unwrap().context_files.is_empty());
}

#[test]
fn empty_context_requires_verified_no_diff_or_host_operational_evidence() {
    let (_root, runtime, repo_root) = runtime_with_workspace_layout();
    let task = seed_task(&runtime, "no-diff", TaskStatus::Backlog, &[], &[]);
    let task_ids = vec![task.id.clone()];
    let invalid_prepared = prepared(&runtime, &repo_root, &task_ids);
    let invalid = partition_result(
        0,
        &task_ids,
        vec![json!({
            "task_id": task.id,
            "context_files_before": [],
            "context_files_after": [],
            "disposition": "selectors",
        })],
    );

    let error = apply(
        &runtime,
        "apply_task_pilot_results",
        &json!({
            "prepared": invalid_prepared,
            "results": [invalid],
            "workspace_path": repo_root,
            "crew": "luna",
        }),
    )
    .expect_err("unverified empty result must fail");
    assert!(error.to_string().contains("verified_no_diff"));

    let valid_prepared = prepared(&runtime, &repo_root, &task_ids);
    let valid = partition_result(
        0,
        &task_ids,
        vec![json!({
            "task_id": task.id,
            "context_files_before": [],
            "context_files_after": [],
            "disposition": "verified_no_diff",
            "evidence": "The requested behavior already exists on the target branch.",
            "recommended_crew": "luna",
            "recommended_complexity": "low",
            "blocked_by": [],
            "duplicate_of": null,
            "already_landed": null,
            "adr_conflicts": [],
            "utility_warnings": [],
            "surface_warnings": [],
        })],
    );
    let output = apply(
        &runtime,
        "apply_task_pilot_results",
        &json!({
            "prepared": valid_prepared,
            "results": [valid],
            "workspace_path": repo_root,
            "crew": "luna",
        }),
    )
    .expect("verified no-diff result is valid");
    assert_eq!(output["tasks"][0]["applied"], false);
}

#[test]
fn out_of_workspace_selector_is_rejected_before_mutation() {
    let (root, runtime, repo_root) = runtime_with_workspace_layout();
    let outside = root.path().join("outside.rs");
    std::fs::write(&outside, "outside").expect("write outside fixture");
    let task = seed_task(&runtime, "escape", TaskStatus::Backlog, &[], &[]);
    let task_ids = vec![task.id.clone()];
    let prepared = prepared(&runtime, &repo_root, &task_ids);
    let outside_selector = format!("file:{}", outside.display());
    let result = partition_result(
        0,
        &task_ids,
        vec![json!({
            "task_id": task.id,
            "context_files_before": [],
            "context_files_after": [outside_selector],
            "disposition": "selectors",
        })],
    );

    let error = apply(
        &runtime,
        "apply_task_pilot_results",
        &json!({
            "prepared": prepared,
            "results": [result],
            "workspace_path": repo_root,
            "crew": "luna",
        }),
    )
    .expect_err("out-of-workspace selector must fail");
    assert!(error.to_string().contains("inside workspace"));
    assert!(runtime.get_task(&task.id).unwrap().context_files.is_empty());
}
