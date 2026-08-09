use orbit_common::types::TaskStatus;
use orbit_engine::{TaskAutomationUpdate, TaskWriteHost, V2RuntimeHost, WORKFLOW_RUN_FAILED_EVENT};
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
    assert!(context.get("execution_summary").is_none());
    assert!(context.get("status_note").is_none());

    runtime
        .update_task(
            &task.id,
            TaskUpdateParams {
                execution_summary: Some("Prior attempt needs a missing capability.".to_string()),
                ..Default::default()
            },
        )
        .expect("record prior execution summary");
    runtime
        .apply_task_automation_update(
            &task.id,
            TaskAutomationUpdate {
                status: Some(TaskStatus::Blocked),
                status_event: Some(WORKFLOW_RUN_FAILED_EVENT.to_string()),
                status_note: Some("workflow run failed: missing provider capability".to_string()),
                ..TaskAutomationUpdate::default()
            },
        )
        .expect("record workflow failure");
    runtime
        .update_task(
            &task.id,
            TaskUpdateParams {
                status: Some(TaskStatus::InProgress),
                ..Default::default()
            },
        )
        .expect("redispatch task");

    let redispatched_context = runtime
        .task_context_for_agent_input(&json!({ "task_id": task.id.clone() }))
        .expect("build redispatched task context")
        .expect("task context present");

    assert_eq!(
        redispatched_context["execution_summary"],
        "Prior attempt needs a missing capability."
    );
    assert_eq!(
        redispatched_context["status_note"],
        "workflow run failed: missing provider capability"
    );
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
