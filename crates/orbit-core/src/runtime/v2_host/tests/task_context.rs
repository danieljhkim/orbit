use orbit_common::types::TaskStatus;
use orbit_engine::V2RuntimeHost;
use serde_json::json;

use crate::OrbitRuntime;
use crate::command::task::{TaskAddParams, TaskUpdateParams};

#[test]
fn task_context_for_agent_input_embeds_canonical_task_with_input_overrides() {
    let runtime = OrbitRuntime::in_memory().expect("build runtime");
    let task = runtime
        .add_task(TaskAddParams {
            title: "Envelope task".to_string(),
            description: "Task description for agent context.".to_string(),
            acceptance_criteria: vec!["Agent can recover the task id.".to_string()],
            plan: "Read the task and implement it.".to_string(),
            workspace_path: Some(".".to_string()),
            ..Default::default()
        })
        .expect("add task");

    let context = runtime
        .task_context_for_agent_input(&json!({
            "task_id": task.id.clone(),
            "workspace_path": "/override/worktree",
            "repo_root": "/override/repo"
        }))
        .expect("build task context")
        .expect("task context present");

    assert_eq!(context["id"], task.id);
    assert_eq!(context["title"], "Envelope task");
    assert_eq!(
        context["description"],
        "Task description for agent context."
    );
    assert_eq!(
        context["acceptance_criteria"][0],
        "Agent can recover the task id."
    );
    assert_eq!(context["plan"], "Read the task and implement it.");
    assert_eq!(context["workspace_path"], "/override/worktree");
    assert_eq!(context["repo_root"], "/override/repo");
    assert_eq!(context["status"], task.status.cli_name());
    assert_eq!(context["terminal"], false);
}

/// [ORB-10499]: `agent_implement` can be dispatched against a task that has
/// already gone terminal — via the executor's single post-recovery attempt, or
/// via a promotion through the approve surface. The envelope has to name that
/// up front so the invocation can exit before doing the work rather than at its
/// final persist call.
#[test]
fn task_context_for_agent_input_marks_write_gated_statuses_terminal() {
    let runtime = OrbitRuntime::in_memory().expect("build runtime");
    let task = runtime
        .add_task(TaskAddParams {
            title: "Already finished by a prior attempt".to_string(),
            description: "Task description for agent context.".to_string(),
            workspace_path: Some(".".to_string()),
            ..Default::default()
        })
        .expect("add task");

    runtime
        .update_task(
            &task.id,
            TaskUpdateParams {
                status: Some(TaskStatus::Done),
                ..Default::default()
            },
        )
        .expect("drive task to done");

    let context = runtime
        .task_context_for_agent_input(&json!({ "task_id": task.id.clone() }))
        .expect("build task context")
        .expect("task context present");

    assert_eq!(context["status"], "done");
    assert_eq!(context["terminal"], true);
}
