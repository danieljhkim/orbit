use orbit_common::types::TaskStatus;
use serde_json::{Value, json};

use super::test_support::{create_task, invalid_input_message, test_runtime};

#[test]
fn execute_tool_command_searches_tasks_for_agents() {
    let (_root, runtime, repo_root) = test_runtime();
    let title_match = create_task(
        &runtime,
        &repo_root,
        "Fix search surface",
        "Wire the tool through Orbit.",
        TaskStatus::Backlog,
        &[],
    );
    let description_match = create_task(
        &runtime,
        &repo_root,
        "Refactor task queries",
        "Preserve SEARCH parity for agents.",
        TaskStatus::Review,
        &[],
    );
    create_task(
        &runtime,
        &repo_root,
        "Unrelated maintenance",
        "Nothing to see here.",
        TaskStatus::Backlog,
        &[],
    );

    let output = runtime
        .execute_tool_command(
            "orbit.task.search",
            json!({ "query": "sEaRcH" }),
            Some("codex".to_string()),
            Some("gpt-5.4".to_string()),
        )
        .expect("search tool succeeds");

    let matches = output.as_array().expect("search returns an array");
    let ids = matches
        .iter()
        .filter_map(|task| task.get("id").and_then(Value::as_str))
        .collect::<Vec<_>>();

    assert_eq!(ids.len(), 2);
    assert!(ids.contains(&title_match.id.as_str()));
    assert!(ids.contains(&description_match.id.as_str()));
}

#[test]
fn task_add_tool_creates_proposed_tasks_for_agents() {
    let (_root, runtime, _repo_root) = test_runtime();

    let output = runtime
        .execute_tool_command(
            "orbit.task.add",
            json!({
                "title": "Propose task from tool",
                "description": "Exercise the agent-facing task creation path.",
                "workspace": ".",
            }),
            Some("codex".to_string()),
            Some("gpt-5.5".to_string()),
        )
        .expect("task add tool succeeds");

    assert_eq!(
        output.get("status").and_then(Value::as_str),
        Some("proposed")
    );
}

#[test]
fn task_delete_tool_rejects_unforced_protected_statuses() {
    let (_root, runtime, repo_root) = test_runtime();
    let task = create_task(
        &runtime,
        &repo_root,
        "Protected delete",
        "Backlog tasks require force before permanent deletion.",
        TaskStatus::Backlog,
        &[],
    );

    let message = invalid_input_message(runtime.execute_tool_command(
        "orbit.task.delete",
        json!({ "id": task.id.clone() }),
        Some("codex".to_string()),
        Some("gpt-5.5".to_string()),
    ));

    assert_eq!(
        message,
        format!(
            "task '{}' is in status 'backlog'; use --force to delete tasks not in proposed, friction, or rejected status",
            task.id
        )
    );
    runtime
        .get_task(&task.id)
        .expect("unforced protected task remains");
}

#[test]
fn task_delete_tool_allows_unforced_proposed_friction_and_rejected_tasks() {
    let (_root, runtime, repo_root) = test_runtime();

    for status in [
        TaskStatus::Proposed,
        TaskStatus::Friction,
        TaskStatus::Rejected,
    ] {
        let task = create_task(
            &runtime,
            &repo_root,
            &format!("Delete {status}"),
            "Unprotected statuses can be permanently deleted without force.",
            status,
            &[],
        );

        let output = runtime
            .execute_tool_command(
                "orbit.task.delete",
                json!({ "id": task.id.clone() }),
                Some("codex".to_string()),
                Some("gpt-5.5".to_string()),
            )
            .expect("unprotected delete succeeds");

        assert_eq!(output, json!({ "id": task.id, "deleted": true }));
    }
}

#[test]
fn task_delete_tool_allows_forced_protected_statuses() {
    let (_root, runtime, repo_root) = test_runtime();
    let task = create_task(
        &runtime,
        &repo_root,
        "Forced delete",
        "Protected statuses can be permanently deleted with explicit force.",
        TaskStatus::InProgress,
        &[],
    );

    let output = runtime
        .execute_tool_command(
            "orbit.task.delete",
            json!({ "id": task.id.clone(), "force": true }),
            Some("codex".to_string()),
            Some("gpt-5.5".to_string()),
        )
        .expect("forced protected delete succeeds");

    assert_eq!(output, json!({ "id": task.id.clone(), "deleted": true }));
    assert!(runtime.get_task(&task.id).is_err(), "task was deleted");
}

#[test]
fn task_add_tool_persists_dependencies() {
    let (_root, runtime, repo_root) = test_runtime();
    let dependency = create_task(
        &runtime,
        &repo_root,
        "Dependency task",
        "Existing task that must finish first.",
        TaskStatus::Backlog,
        &[],
    );

    let output = runtime
        .execute_tool_command(
            "orbit.task.add",
            json!({
                "title": "Dependent task from tool",
                "description": "Exercise dependency input on the agent-facing task creation path.",
                "workspace": ".",
                "dependencies": [dependency.id.clone()],
            }),
            Some("codex".to_string()),
            Some("gpt-5.5".to_string()),
        )
        .expect("task add tool succeeds");

    assert_eq!(
        output.get("dependencies"),
        Some(&json!([dependency.id.as_str()]))
    );
}

#[test]
fn task_add_tool_persists_external_refs() {
    let (_root, runtime, _repo_root) = test_runtime();

    let output = runtime
        .execute_tool_command(
            "orbit.task.add",
            json!({
                "title": "External ref task",
                "description": "Exercise external ref input on the agent-facing task creation path.",
                "workspace": ".",
                "external_refs": [
                    {"system": "jira", "id": "ENG-1234", "url": "https://example.com/browse/ENG-1234"},
                    {"system": "linear", "id": "LIN-567"}
                ],
            }),
            Some("codex".to_string()),
            Some("gpt-5.5".to_string()),
        )
        .expect("task add tool succeeds");

    assert_eq!(
        output.get("external_refs"),
        Some(&json!([
            {"system": "jira", "id": "ENG-1234", "url": "https://example.com/browse/ENG-1234"},
            {"system": "linear", "id": "LIN-567"}
        ]))
    );
}

#[test]
fn task_add_tool_recovers_mcp_encoded_acceptance_and_context_arrays() {
    let (_root, runtime, repo_root) = test_runtime();
    let src_dir = repo_root.join("src");
    std::fs::create_dir_all(&src_dir).expect("create src dir");
    std::fs::write(src_dir.join("lib.rs"), "pub fn ok() {}\n").expect("write source file");

    let output = runtime
        .execute_tool_command(
            "orbit.task.add",
            json!({
                "title": "Encoded list task",
                "description": "Exercise MCP single-element encoded array recovery.",
                "workspace": repo_root.to_string_lossy(),
                "acceptance_criteria": ["[\"Criterion A\", \"Criterion B\"]"],
                "context_files": ["[\"file:src/lib.rs\"]"],
            }),
            Some("codex".to_string()),
            Some("gpt-5.5".to_string()),
        )
        .expect("task add tool succeeds");

    assert_eq!(
        output.get("acceptance_criteria"),
        Some(&json!(["Criterion A", "Criterion B"]))
    );
    assert_eq!(
        output.get("context_files"),
        Some(&json!(["file:src/lib.rs"]))
    );
}

#[test]
fn task_add_tool_infers_agent_from_model_only_input() {
    let (_root, runtime, _repo_root) = test_runtime();

    let output = runtime
        .execute_tool_command(
            "orbit.task.add",
            json!({
                "title": "Propose model-only task",
                "description": "Exercise model-first provenance.",
                "workspace": ".",
                "model": "gpt-5.5",
            }),
            None,
            None,
        )
        .expect("task add tool succeeds");

    assert_eq!(output.get("agent").and_then(Value::as_str), Some("codex"));
    assert_eq!(output.get("model").and_then(Value::as_str), Some("gpt-5.5"));
    assert_eq!(
        output.get("created_by").and_then(Value::as_str),
        Some("gpt-5.5")
    );
}

#[test]
fn task_update_tool_infers_agent_from_model_only_input() {
    let (_root, runtime, repo_root) = test_runtime();
    let task = create_task(
        &runtime,
        &repo_root,
        "Update model-only task",
        "Exercise model-first update provenance.",
        TaskStatus::Backlog,
        &[],
    );

    let output = runtime
        .execute_tool_command(
            "orbit.task.update",
            json!({
                "id": task.id,
                "comment": "record model-only update",
                "model": "gemini-3.1-pro-preview",
            }),
            None,
            None,
        )
        .expect("task update tool succeeds");

    assert_eq!(output.get("agent").and_then(Value::as_str), Some("gemini"));
    assert_eq!(
        output.get("model").and_then(Value::as_str),
        Some("gemini-3.1-pro-preview")
    );
}

#[test]
fn task_update_tool_replaces_dependencies() {
    let (_root, runtime, repo_root) = test_runtime();
    let first_dependency = create_task(
        &runtime,
        &repo_root,
        "First dependency",
        "Existing task that must finish first.",
        TaskStatus::Backlog,
        &[],
    );
    let second_dependency = create_task(
        &runtime,
        &repo_root,
        "Second dependency",
        "Replacement prerequisite.",
        TaskStatus::Backlog,
        &[],
    );
    let task = create_task(
        &runtime,
        &repo_root,
        "Update dependency task",
        "Exercise dependency replacement through tool input.",
        TaskStatus::Backlog,
        &[],
    );

    let output = runtime
        .execute_tool_command(
            "orbit.task.update",
            json!({
                "id": task.id.clone(),
                "dependencies": [first_dependency.id.clone()],
            }),
            Some("codex".to_string()),
            Some("gpt-5.5".to_string()),
        )
        .expect("task update tool sets dependency");

    assert_eq!(
        output.get("dependencies"),
        Some(&json!([first_dependency.id.as_str()]))
    );

    let output = runtime
        .execute_tool_command(
            "orbit.task.update",
            json!({
                "id": task.id,
                "dependencies": [second_dependency.id.clone()],
            }),
            Some("codex".to_string()),
            Some("gpt-5.5".to_string()),
        )
        .expect("task update tool replaces dependency");

    assert_eq!(
        output.get("dependencies"),
        Some(&json!([second_dependency.id.as_str()]))
    );
}

#[test]
fn task_update_tool_recovers_mcp_encoded_acceptance_array() {
    let (_root, runtime, repo_root) = test_runtime();
    let task = create_task(
        &runtime,
        &repo_root,
        "Update encoded list",
        "Exercise replacement through MCP encoded array shape.",
        TaskStatus::Backlog,
        &[],
    );

    let output = runtime
        .execute_tool_command(
            "orbit.task.update",
            json!({
                "id": task.id,
                "acceptance_criteria": ["[\"Criterion A\", \"Criterion B\"]"],
            }),
            Some("codex".to_string()),
            Some("gpt-5.5".to_string()),
        )
        .expect("task update tool succeeds");

    assert_eq!(
        output.get("acceptance_criteria"),
        Some(&json!(["Criterion A", "Criterion B"]))
    );
}

#[test]
fn task_show_tool_recovers_mcp_encoded_fields_array() {
    let (_root, runtime, repo_root) = test_runtime();
    let task = create_task(
        &runtime,
        &repo_root,
        "Show encoded fields",
        "Exercise field projection through MCP encoded array shape.",
        TaskStatus::Backlog,
        &["file:src/lib.rs"],
    );

    let output = runtime
        .execute_tool_command(
            "orbit.task.show",
            json!({
                "id": task.id,
                "fields": ["[\"description\", \"context_files\"]"],
            }),
            Some("codex".to_string()),
            Some("gpt-5.5".to_string()),
        )
        .expect("task show tool succeeds");

    assert_eq!(
        output,
        json!({
            "description": "Exercise field projection through MCP encoded array shape.",
            "context_files": ["file:src/lib.rs"],
        })
    );
}

#[test]
fn task_update_tool_allows_explicit_attribution_updates() {
    let (_root, runtime, repo_root) = test_runtime();
    let task = create_task(
        &runtime,
        &repo_root,
        "Update explicit attribution",
        "Exercise explicit provenance correction.",
        TaskStatus::Backlog,
        &[],
    );

    let output = runtime
        .execute_tool_command(
            "orbit.task.update",
            json!({
                "id": task.id.clone(),
                "planned_by": "manual-planner",
                "implemented_by": "manual-implementer",
            }),
            Some("codex".to_string()),
            Some("gpt-5.5".to_string()),
        )
        .expect("task update tool succeeds");

    assert_eq!(
        output.get("planned_by").and_then(Value::as_str),
        Some("manual-planner")
    );
    assert_eq!(
        output.get("implemented_by").and_then(Value::as_str),
        Some("manual-implementer")
    );

    let output = runtime
        .execute_tool_command(
            "orbit.task.update",
            json!({
                "id": task.id,
                "planned_by": "",
                "implemented_by": "",
            }),
            Some("codex".to_string()),
            Some("gpt-5.5".to_string()),
        )
        .expect("task update tool clears attribution");

    assert_eq!(output.get("planned_by"), Some(&Value::Null));
    assert_eq!(output.get("implemented_by"), Some(&Value::Null));
}

#[test]
fn task_update_tool_explicit_implemented_by_overrides_review_stamp() {
    let (_root, runtime, repo_root) = test_runtime();
    let task = create_task(
        &runtime,
        &repo_root,
        "Review explicit attribution",
        "Exercise explicit provenance correction on review transition.",
        TaskStatus::InProgress,
        &[],
    );

    let output = runtime
        .execute_tool_command(
            "orbit.task.update",
            json!({
                "id": task.id,
                "status": "review",
                "execution_summary": "Implemented and validated.",
                "implemented_by": "manual-implementer",
                "model": "gemini-3.1-pro-preview",
            }),
            None,
            None,
        )
        .expect("task update tool succeeds");

    assert_eq!(output.get("status").and_then(Value::as_str), Some("review"));
    assert_eq!(
        output.get("implemented_by").and_then(Value::as_str),
        Some("manual-implementer")
    );
}

#[test]
fn task_tool_rejects_mismatched_agent_and_model() {
    let (_root, runtime, _repo_root) = test_runtime();

    let error = runtime
        .execute_tool_command(
            "orbit.task.add",
            json!({
                "title": "Reject mismatched identity",
                "description": "Exercise explicit mismatch validation.",
                "workspace": ".",
                "agent": "claude",
                "model": "gpt-5.5",
            }),
            None,
            None,
        )
        .expect_err("mismatched identity should fail");

    assert!(error.to_string().contains("does not match `model`"));
}
