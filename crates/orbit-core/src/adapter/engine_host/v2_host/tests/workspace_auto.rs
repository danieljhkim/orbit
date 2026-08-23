use chrono::{SecondsFormat, Utc};
use orbit_engine::RuntimeHost;
use orbit_tools::ToolContext;
use orbit_types::task::{TaskPriority, TaskStatus, TaskType};
use serde_json::{Value, json};

use crate::OrbitRuntime;
use crate::adapter::engine_host::v2_host::test_support::{
    runtime_with_workspace_layout, seed_list_backlog_task, write_workspace_file,
};
use crate::application::task::{TaskAddParams, TaskUpdateParams};

fn classify(runtime: &OrbitRuntime) -> Value {
    runtime
        .run_deterministic(
            "classify_workspace_auto_tasks",
            &json!({}),
            &json!({}),
            ToolContext::default(),
        )
        .expect("classify workspace auto tasks")
}

fn drain_window(runtime: &OrbitRuntime, input: Value) -> Value {
    runtime
        .run_deterministic("drain_window", &json!({}), &input, ToolContext::default())
        .expect("drain window")
}

fn list_epic_descendants(runtime: &OrbitRuntime, epic_task_id: &str) -> Value {
    list_epic_descendants_with(runtime, epic_task_id, json!({}))
}

fn list_epic_descendants_with(runtime: &OrbitRuntime, epic_task_id: &str, extra: Value) -> Value {
    let mut input = extra;
    if let Some(object) = input.as_object_mut() {
        object.insert("epic_task_id".to_string(), json!(epic_task_id));
    }
    runtime
        .run_deterministic(
            "list_epic_descendants",
            &json!({}),
            &input,
            ToolContext::default(),
        )
        .expect("list epic descendants")
}

fn list_epic_descendants_err(
    runtime: &OrbitRuntime,
    epic_task_id: &str,
    extra: Value,
) -> orbit_engine::DispatchError {
    let mut input = extra;
    if let Some(object) = input.as_object_mut() {
        object.insert("epic_task_id".to_string(), json!(epic_task_id));
    }
    runtime
        .run_deterministic(
            "list_epic_descendants",
            &json!({}),
            &input,
            ToolContext::default(),
        )
        .expect_err("list epic descendants should fail")
}

#[test]
fn epic_descendants_are_dependency_then_priority_ordered_and_terminal_tasks_are_skipped() {
    let (_root, runtime, _repo_root) = runtime_with_workspace_layout();
    let epic = runtime
        .add_task(TaskAddParams {
            title: "Epic root".to_string(),
            description: "Epic fixture".to_string(),
            acceptance_criteria: vec!["Supervised".to_string()],
            tags: vec!["epic".to_string()],
            plan: "Delegate children".to_string(),
            status: Some(TaskStatus::InProgress),
            ..Default::default()
        })
        .expect("seed epic root");
    let foundation = runtime
        .add_task(TaskAddParams {
            parent_id: Some(epic.id.clone()),
            title: "Foundation".to_string(),
            description: "Foundation fixture".to_string(),
            acceptance_criteria: vec!["Done".to_string()],
            plan: "Implement".to_string(),
            priority: TaskPriority::Low,
            status: Some(TaskStatus::Backlog),
            ..Default::default()
        })
        .expect("seed foundation");
    let dependent = runtime
        .add_task(TaskAddParams {
            parent_id: Some(epic.id.clone()),
            title: "Dependent".to_string(),
            description: "Dependent fixture".to_string(),
            acceptance_criteria: vec!["Done".to_string()],
            dependencies: vec![foundation.id.clone()],
            plan: "Implement".to_string(),
            priority: TaskPriority::High,
            status: Some(TaskStatus::Backlog),
            ..Default::default()
        })
        .expect("seed dependent");
    let independent = runtime
        .add_task(TaskAddParams {
            parent_id: Some(epic.id.clone()),
            title: "Independent".to_string(),
            description: "Independent fixture".to_string(),
            acceptance_criteria: vec!["Done".to_string()],
            plan: "Implement".to_string(),
            priority: TaskPriority::Critical,
            status: Some(TaskStatus::Backlog),
            ..Default::default()
        })
        .expect("seed independent");
    let done = runtime
        .add_task(TaskAddParams {
            parent_id: Some(epic.id.clone()),
            title: "Already done".to_string(),
            description: "Done fixture".to_string(),
            acceptance_criteria: vec!["Done".to_string()],
            plan: "Implemented".to_string(),
            status: Some(TaskStatus::Done),
            ..Default::default()
        })
        .expect("seed done child");

    let output = list_epic_descendants(&runtime, &epic.id);
    assert_eq!(
        output["task_ids"],
        json!([independent.id, foundation.id, dependent.id])
    );
    assert_eq!(output["task_count"], 3);
    assert!(
        !output["task_ids"]
            .as_array()
            .expect("task ids")
            .contains(&json!(done.id))
    );
}

#[test]
fn epic_with_no_descendants_has_an_empty_drain() {
    let (_root, runtime, _repo_root) = runtime_with_workspace_layout();
    let epic = runtime
        .add_task(TaskAddParams {
            title: "Empty epic".to_string(),
            description: "Epic fixture".to_string(),
            acceptance_criteria: vec!["No children".to_string()],
            tags: vec!["epic".to_string()],
            plan: "No-op".to_string(),
            status: Some(TaskStatus::InProgress),
            ..Default::default()
        })
        .expect("seed empty epic");

    let output = list_epic_descendants(&runtime, &epic.id);
    assert_eq!(output["task_ids"], json!([]));
    assert_eq!(output["task_count"], 0);
    assert_eq!(output["empty"], true);
}

#[test]
fn leftover_descendants_fail_closed_and_name_the_ids() {
    let (_root, runtime, _repo_root) = runtime_with_workspace_layout();
    let epic = runtime
        .add_task(TaskAddParams {
            title: "Epic root".to_string(),
            description: "Epic fixture".to_string(),
            acceptance_criteria: vec!["Supervised".to_string()],
            tags: vec!["epic".to_string()],
            plan: "Delegate children".to_string(),
            status: Some(TaskStatus::InProgress),
            ..Default::default()
        })
        .expect("seed epic root");
    let leftover = runtime
        .add_task(TaskAddParams {
            parent_id: Some(epic.id.clone()),
            title: "Still open".to_string(),
            description: "Unfinished descendant".to_string(),
            acceptance_criteria: vec!["Done".to_string()],
            plan: "Implement".to_string(),
            status: Some(TaskStatus::Backlog),
            ..Default::default()
        })
        .expect("seed leftover child");
    let unrelated = seed_list_backlog_task(
        &runtime,
        "Unrelated chore",
        TaskStatus::Backlog,
        TaskPriority::Low,
        TaskType::Chore,
        None,
        vec![],
    );

    let error = list_epic_descendants_err(&runtime, &epic.id, json!({ "fail_if_nonempty": true }));
    match error {
        orbit_engine::DispatchError::DeterministicActionFailed { action, message } => {
            assert_eq!(action, "list_epic_descendants");
            assert!(message.contains(&leftover.id), "{message}");
            assert!(
                message.contains("epic descendants remain after drain"),
                "{message}"
            );
            assert!(
                !message.contains(&unrelated.id),
                "unrelated backlog must not appear in the epic fail-closed message: {message}"
            );
        }
        other => panic!("expected leftover-descendant failure, got {other:?}"),
    }
}

#[test]
fn fail_if_nonempty_ignores_unrelated_backlog_when_the_epic_is_empty() {
    let (_root, runtime, _repo_root) = runtime_with_workspace_layout();
    let epic = runtime
        .add_task(TaskAddParams {
            title: "Empty epic".to_string(),
            description: "Epic fixture".to_string(),
            acceptance_criteria: vec!["No children".to_string()],
            tags: vec!["epic".to_string()],
            plan: "No-op".to_string(),
            status: Some(TaskStatus::InProgress),
            ..Default::default()
        })
        .expect("seed empty epic");
    seed_list_backlog_task(
        &runtime,
        "Unrelated chore",
        TaskStatus::Backlog,
        TaskPriority::Low,
        TaskType::Chore,
        None,
        vec![],
    );

    let output =
        list_epic_descendants_with(&runtime, &epic.id, json!({ "fail_if_nonempty": true }));
    assert_eq!(output["empty"], true);
    assert_eq!(output["task_ids"], json!([]));
}

#[test]
fn two_loose_tasks_and_one_epic_root_are_admissible_together() {
    let (_root, runtime, _repo_root) = runtime_with_workspace_layout();
    let loose_one = seed_list_backlog_task(
        &runtime,
        "Loose high",
        TaskStatus::Backlog,
        TaskPriority::High,
        TaskType::Chore,
        None,
        vec![],
    );
    let loose_two = seed_list_backlog_task(
        &runtime,
        "Loose medium",
        TaskStatus::Backlog,
        TaskPriority::Medium,
        TaskType::Chore,
        None,
        vec![],
    );
    let epic = runtime
        .add_task(TaskAddParams {
            title: "Epic root".to_string(),
            description: "Epic fixture".to_string(),
            acceptance_criteria: vec!["Supervised".to_string()],
            tags: vec!["epic".to_string()],
            plan: "Delegate children".to_string(),
            status: Some(TaskStatus::Backlog),
            ..Default::default()
        })
        .expect("seed epic root");
    for index in 0..3 {
        seed_list_backlog_task(
            &runtime,
            &format!("Epic child {index}"),
            TaskStatus::Backlog,
            TaskPriority::Medium,
            TaskType::Chore,
            Some(epic.id.clone()),
            vec![],
        );
    }

    // Leaves and the epic are independent answers, so both are admissible in
    // the same iteration: the drain ships the leaves and starts the epic.
    let first = classify(&runtime);
    assert_eq!(first["loose_task_ids"], json!([loose_one.id, loose_two.id]));
    assert_eq!(
        first["loose_task_dispatches"].as_array().map(Vec::len),
        Some(1)
    );
    assert_eq!(
        first["loose_task_dispatches"][0]["task_ids"],
        json!([loose_one.id, loose_two.id])
    );
    assert_eq!(first["has_leaves"], true);
    assert_eq!(first["epic_task_id"], epic.id);
    assert_eq!(first["has_epic"], true);
    assert_eq!(first["empty"], false);

    for loose in [&loose_one, &loose_two] {
        runtime
            .update_task(
                &loose.id,
                TaskUpdateParams {
                    status: Some(TaskStatus::Done),
                    ..Default::default()
                },
            )
            .expect("complete loose task");
    }
    let second = classify(&runtime);
    assert_eq!(second["epic_task_id"], epic.id);
    assert_eq!(second["loose_task_ids"], json!([]));
    assert_eq!(second["has_leaves"], false);
}

#[test]
fn loose_tasks_are_partitioned_by_effective_crew_in_priority_order() {
    let root = tempfile::tempdir().expect("create tempdir");
    let global = root.path().join("home/.orbit");
    let workspace = root.path().join("repo/.orbit");
    std::fs::create_dir_all(&global).expect("global orbit dir");
    std::fs::create_dir_all(&workspace).expect("workspace orbit dir");
    std::fs::write(
        workspace.join("config.toml"),
        r#"
[workflow]
default_crew = "sol"

[crews.sol]
provider = "codex"
backend = "cli"
model = "gpt-5.6-sol"

[crews.terra]
provider = "codex"
backend = "cli"
model = "gpt-5.6-terra"
"#,
    )
    .expect("write crew fixture");
    let runtime = OrbitRuntime::from_roots(&global, &workspace).expect("build runtime");

    let sol_high = runtime
        .add_task(TaskAddParams {
            title: "Sol high".to_string(),
            description: "Crew partition fixture".to_string(),
            priority: TaskPriority::High,
            crew: Some("sol".to_string()),
            status: Some(TaskStatus::Backlog),
            ..Default::default()
        })
        .expect("seed sol task");
    let terra = runtime
        .add_task(TaskAddParams {
            title: "Terra medium".to_string(),
            description: "Crew partition fixture".to_string(),
            priority: TaskPriority::Medium,
            crew: Some("terra".to_string()),
            status: Some(TaskStatus::Backlog),
            ..Default::default()
        })
        .expect("seed terra task");
    let sol_low = runtime
        .add_task(TaskAddParams {
            title: "Sol low".to_string(),
            description: "Crew partition fixture".to_string(),
            priority: TaskPriority::Low,
            crew: Some("sol".to_string()),
            status: Some(TaskStatus::Backlog),
            ..Default::default()
        })
        .expect("seed second sol task");

    let output = classify(&runtime);
    assert_eq!(
        output["loose_task_ids"],
        json!([sol_high.id, terra.id, sol_low.id])
    );
    assert_eq!(
        output["loose_task_dispatches"],
        json!([
            { "crew": "sol", "task_ids": [sol_high.id, sol_low.id] },
            { "crew": "terra", "task_ids": [terra.id] },
        ])
    );
    for dispatch in output["loose_task_dispatches"]
        .as_array()
        .expect("dispatch partitions")
    {
        let input = json!({ "task_ids": dispatch["task_ids"] });
        let run = runtime
            .stores()
            .jobs()
            .insert_job_run(
                "task_auto_pipeline",
                1,
                Utc::now(),
                Some(input.clone()),
                None,
            )
            .expect("insert homogeneous child");
        runtime
            .record_run_crew_from_input(&run.run_id, &input)
            .expect("persist homogeneous child crew");
        assert_eq!(
            runtime
                .show_job_run(&run.run_id)
                .expect("show homogeneous child")
                .resolved_crew
                .as_deref(),
            dispatch["crew"].as_str()
        );
    }
}

/// The `hold` decision this replaces froze every conflict-free chore for as
/// long as an epic root was `in-progress`. Admission is the epic's lock
/// reservation instead: the leaf that overlaps its descendants' declared files
/// is excluded, and the one that does not still ships in the same drain.
#[test]
fn a_live_epic_excludes_only_the_leaves_that_overlap_its_reservation() {
    let (_root, runtime, repo_root) = runtime_with_workspace_layout();
    write_workspace_file(&repo_root, "crates/epic/src/lib.rs");
    write_workspace_file(&repo_root, "crates/elsewhere/src/lib.rs");
    let epic = runtime
        .add_task(TaskAddParams {
            title: "Active epic".to_string(),
            description: "Epic fixture".to_string(),
            acceptance_criteria: vec!["Supervised".to_string()],
            tags: vec!["epic".to_string()],
            plan: "Delegate children".to_string(),
            workspace_path: Some(".".to_string()),
            status: Some(TaskStatus::InProgress),
            ..Default::default()
        })
        .expect("seed active epic");
    // The epic root reserves the union of its descendants' context files.
    seed_list_backlog_task(
        &runtime,
        "Epic child",
        TaskStatus::Backlog,
        TaskPriority::Medium,
        TaskType::Chore,
        Some(epic.id.clone()),
        vec!["crates/epic/src/lib.rs"],
    );
    let overlapping = seed_list_backlog_task(
        &runtime,
        "Late loose task inside the epic's files",
        TaskStatus::Backlog,
        TaskPriority::Critical,
        TaskType::Chore,
        None,
        vec!["crates/epic/src/lib.rs"],
    );
    let conflict_free = seed_list_backlog_task(
        &runtime,
        "Late loose task elsewhere",
        TaskStatus::Backlog,
        TaskPriority::Low,
        TaskType::Chore,
        None,
        vec!["crates/elsewhere/src/lib.rs"],
    );

    let admissible = classify(&runtime);
    assert_eq!(admissible["loose_task_ids"], json!([conflict_free.id]));
    assert_eq!(admissible["has_leaves"], true);
    assert_eq!(admissible["empty"], false);
    assert!(
        !admissible["loose_task_ids"]
            .as_array()
            .expect("loose task ids")
            .contains(&json!(overlapping.id)),
        "a leaf overlapping the epic's reserved files must not ship"
    );
}

#[test]
fn an_empty_workspace_is_admissibly_empty() {
    let (_root, runtime, _repo_root) = runtime_with_workspace_layout();

    let empty = classify(&runtime);

    assert_eq!(empty["loose_task_ids"], json!([]));
    assert_eq!(empty["has_leaves"], false);
    assert_eq!(empty["epic_task_id"], Value::Null);
    assert_eq!(empty["has_epic"], false);
    assert_eq!(empty["empty"], true);
    assert_eq!(empty["active_epic_run_id"], Value::Null);
}

#[test]
fn a_backlog_epic_root_waits_while_an_epic_run_is_live() {
    let (_root, runtime, _repo_root) = runtime_with_workspace_layout();
    let waiting = runtime
        .add_task(TaskAddParams {
            title: "Second epic root".to_string(),
            description: "Epic fixture".to_string(),
            acceptance_criteria: vec!["Supervised".to_string()],
            tags: vec!["epic".to_string()],
            plan: "Delegate children".to_string(),
            status: Some(TaskStatus::Backlog),
            ..Default::default()
        })
        .expect("seed backlog epic root");

    assert_eq!(classify(&runtime)["epic_task_id"], waiting.id);

    // `epic_pipeline` admits one active run. Once one is live, offering
    // another root would queue a pending run rather than start work — and the
    // drain loop would mint a fresh one every iteration.
    let live = runtime
        .stores()
        .jobs()
        .insert_job_run(
            "epic_pipeline",
            1,
            Utc::now(),
            Some(json!({ "epic_task_id": "ORB-00001" })),
            None,
        )
        .expect("insert live epic run");

    let admissible = classify(&runtime);
    assert_eq!(admissible["epic_task_id"], Value::Null);
    assert_eq!(admissible["has_epic"], false);
    assert_eq!(admissible["empty"], true);
    assert_eq!(admissible["active_epic_run_id"], live.run_id);
    assert_eq!(admissible["active_epic_task_id"], "ORB-00001");
}

#[test]
fn an_absent_window_is_expired_on_its_first_answer() {
    let (_root, runtime, _repo_root) = runtime_with_workspace_layout();

    // `break_when` is evaluated after the loop body, so an already-expired
    // window still yields exactly one iteration — today's one-tick behavior.
    let stamped = drain_window(&runtime, json!({}));
    assert_eq!(stamped["expired"], true);
    assert_eq!(stamped["remaining_seconds"], 0.0);

    // The template over an absent `for_seconds` renders an empty string.
    let rendered_absent = drain_window(&runtime, json!({ "for_seconds": "" }));
    assert_eq!(rendered_absent["expired"], true);
}

#[test]
fn a_stamped_window_answers_expiry_against_its_own_deadline() {
    let (_root, runtime, _repo_root) = runtime_with_workspace_layout();

    let stamped = drain_window(&runtime, json!({ "for_seconds": 600 }));
    assert_eq!(stamped["expired"], false);
    let remaining = stamped["remaining_seconds"]
        .as_f64()
        .expect("remaining seconds");
    assert!(
        (595.0..=600.0).contains(&remaining),
        "expected ~600s remaining, got {remaining}"
    );

    // Re-reading the stamp is a pure function of the deadline the first call
    // returned; nothing durable is written between the two.
    let reread = drain_window(&runtime, json!({ "deadline": stamped["deadline"] }));
    assert_eq!(reread["expired"], false);
    assert_eq!(reread["deadline"], stamped["deadline"]);

    let past =
        (Utc::now() - chrono::Duration::seconds(1)).to_rfc3339_opts(SecondsFormat::Secs, true);
    assert_eq!(
        drain_window(&runtime, json!({ "deadline": past }))["expired"],
        true
    );
}

#[test]
fn a_drain_window_rejects_an_unparseable_deadline_or_an_oversize_request() {
    let (_root, runtime, _repo_root) = runtime_with_workspace_layout();

    for input in [
        json!({ "deadline": "not-a-timestamp" }),
        json!({ "for_seconds": 86_401 }),
        json!({ "for_seconds": -1 }),
    ] {
        assert!(
            runtime
                .run_deterministic("drain_window", &json!({}), &input, ToolContext::default())
                .is_err(),
            "expected {input} to be refused"
        );
    }
}
