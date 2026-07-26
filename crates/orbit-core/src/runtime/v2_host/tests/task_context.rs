use orbit_engine::V2RuntimeHost;
use serde_json::json;

use crate::OrbitRuntime;
use crate::command::task::TaskAddParams;

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
}
