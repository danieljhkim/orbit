use std::fs;

use orbit_common::types::TaskStatus;
use serde_json::{Value, json};

use super::test_support::{create_task, invalid_input_message, test_runtime};

fn assert_task_titles(output: &Value, expected: &[&str]) {
    let mut titles = output
        .as_array()
        .expect("tool returns task array")
        .iter()
        .map(|task| {
            task.get("title")
                .and_then(Value::as_str)
                .expect("task title")
                .to_string()
        })
        .collect::<Vec<_>>();
    titles.sort();

    let mut expected = expected
        .iter()
        .map(|title| (*title).to_string())
        .collect::<Vec<_>>();
    expected.sort();

    assert_eq!(titles, expected);
}

#[test]
fn execute_tool_command_searches_tasks_for_agents_via_orbit_search() {
    // ORB-00202: `orbit.task.search` was deleted in phase 2; the substring
    // case migrates to `orbit.search --kind task`.
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
            "orbit.search",
            json!({ "query": "sEaRcH", "kind": "task" }),
            Some("codex".to_string()),
            Some("gpt-5.4".to_string()),
        )
        .expect("search tool succeeds");

    let matches = output["results"].as_array().expect("results array");
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
fn duel_plan_add_persists_gemini_planner_artifact() {
    let (_root, runtime, repo_root) = test_runtime();
    let task = create_task(
        &runtime,
        &repo_root,
        "Gemini planning duel artifact",
        "Exercise the planner artifact write path used by direct-agent duels.",
        TaskStatus::InProgress,
        &[],
    );
    let content = "## Plan\nPersist through Orbit tools.";

    runtime
        .execute_tool_command(
            "orbit.duel.plan.add",
            json!({
                "id": task.id.clone(),
                "planning_duel_slot": "planner_a",
                "content": content,
            }),
            Some("gemini".to_string()),
            Some("gemini-3.1-pro".to_string()),
        )
        .expect("gemini duel plan add succeeds");

    let artifacts = runtime
        .get_task_artifacts(&task.id)
        .expect("read task artifacts");
    let artifact = artifacts
        .iter()
        .find(|artifact| artifact.path == "planning-duel/planner_a.md")
        .expect("gemini planner artifact");
    assert_eq!(
        artifact.text_content(),
        Some("*authored by: gemini / planner_a*\n## Plan\nPersist through Orbit tools.")
    );
}

#[test]
fn duel_plan_winner_persists_gemini_arbiter_artifact() {
    // ORB-00037: extends ORB-00027 coverage from planner `plan.add` to arbiter
    // `plan.winner`. Both delegate to the same TaskUpdate artifact contract;
    // this test guards against the arbiter path silently regressing when the
    // planner path is refactored.
    let (_root, runtime, repo_root) = test_runtime();
    let task = create_task(
        &runtime,
        &repo_root,
        "Gemini arbiter winner",
        "Exercise the arbiter winner.json write path used by direct-agent duels.",
        TaskStatus::InProgress,
        &[],
    );

    runtime
        .execute_tool_command(
            "orbit.duel.plan.winner",
            json!({
                "id": task.id.clone(),
                "winner_slot": "planner_a",
                "arbiter_rationale": "Tighter scope and clearer staged plan.",
            }),
            Some("gemini".to_string()),
            Some("gemini-3.1-pro".to_string()),
        )
        .expect("gemini duel plan winner succeeds");

    let artifacts = runtime
        .get_task_artifacts(&task.id)
        .expect("read task artifacts");
    let artifact = artifacts
        .iter()
        .find(|artifact| artifact.path == "planning-duel/winner.json")
        .expect("arbiter winner.json artifact");
    let raw = artifact
        .text_content()
        .expect("winner.json must be text content");
    let payload: Value = serde_json::from_str(raw).expect("winner.json is valid JSON");
    assert_eq!(payload["winner_slot"], "planner_a");
    assert_eq!(payload["arbiter_family"], "gemini");
    assert_eq!(payload["artifact_path"], "planning-duel/planner_a.md");
    assert_eq!(
        payload["arbiter_rationale"],
        "Tighter scope and clearer staged plan."
    );
}

#[test]
fn task_add_tool_rejects_dropped_task_types_and_friction_status() {
    let (_root, runtime, _repo_root) = test_runtime();

    for dropped_type in ["task", "epic", "issue", "friction"] {
        let message = invalid_input_message(runtime.execute_tool_command(
            "orbit.task.add",
            json!({
            "title": "Legacy friction type",
            "description": "Should use the new friction record surface.",
            "workspace": ".",
                "type": dropped_type,
            }),
            Some("codex".to_string()),
            Some("gpt-5.5".to_string()),
        ));
        assert!(message.contains(dropped_type), "{message}");
        assert!(
            message.contains("feature, bug, refactor, chore"),
            "{message}"
        );
    }

    let message = invalid_input_message(runtime.execute_tool_command(
        "orbit.task.add",
        json!({
            "title": "Legacy friction status",
            "description": "Should use the new friction record surface.",
            "workspace": ".",
            "status": "friction",
        }),
        Some("codex".to_string()),
        Some("gpt-5.5".to_string()),
    ));
    assert!(message.contains("orbit.friction.add"), "{message}");
}

#[test]
fn friction_add_writes_markdown_record_and_validates_tags() {
    let (_root, runtime, _repo_root) = test_runtime();

    let output = runtime
        .execute_tool_command(
            "orbit.friction.add",
            json!({
                "body": "The tool guidance pointed at the old task path.",
                "tags": ["tooling", "skill-guidance"],
            }),
            Some("codex".to_string()),
            Some("gpt-5.5".to_string()),
        )
        .expect("friction add succeeds");

    let path = output["path"].as_str().expect("record path");
    let raw = std::fs::read_to_string(path).expect("read friction markdown");
    assert!(raw.starts_with("---\n"), "{raw}");
    assert!(raw.contains("id: F"), "{raw}");
    assert!(raw.contains("model: codex"), "{raw}");
    assert!(raw.contains("tooling"), "{raw}");
    assert!(raw.contains("skill-guidance"), "{raw}");

    let message = invalid_input_message(runtime.execute_tool_command(
        "orbit.friction.add",
        json!({
            "body": "Unknown tag should be rejected.",
            "tags": ["not-a-real-tag"],
        }),
        Some("codex".to_string()),
        Some("gpt-5.5".to_string()),
    ));
    assert!(message.contains("valid tags"), "{message}");
}

#[test]
fn friction_stats_does_not_write_state_scoreboard_file() {
    let (_root, runtime, _repo_root) = test_runtime();
    runtime
        .execute_tool_command(
            "orbit.friction.add",
            json!({ "body": "A friction report.", "tags": ["other"] }),
            Some("codex".to_string()),
            Some("gpt-zero".to_string()),
        )
        .expect("add friction");

    let stats = runtime
        .execute_tool_command(
            "orbit.friction.stats",
            json!({}),
            Some("codex".to_string()),
            Some("gpt-5.5".to_string()),
        )
        .expect("stats succeeds");
    assert_eq!(
        stats["by_family"]["codex"]["frictions_per_10_tasks"],
        json!("n/a")
    );
    assert!(
        !runtime
            .data_root()
            .join("state")
            .join("friction_stats.json")
            .exists()
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
fn task_add_and_show_tools_roundtrip_tags() {
    let (_root, runtime, _repo_root) = test_runtime();

    let added = runtime
        .execute_tool_command(
            "orbit.task.add",
            json!({
                "title": "Tagged task",
                "description": "Exercise tag input on the agent-facing task creation path.",
                "workspace": ".",
                "tags": ["perf", "bench"],
            }),
            Some("codex".to_string()),
            Some("gpt-5.5".to_string()),
        )
        .expect("task add tool succeeds");
    let task_id = added["id"].as_str().expect("task id");

    let shown = runtime
        .execute_tool_command(
            "orbit.task.show",
            json!({ "id": task_id }),
            Some("codex".to_string()),
            Some("gpt-5.5".to_string()),
        )
        .expect("task show tool succeeds");

    assert_eq!(shown.get("tags"), Some(&json!(["perf", "bench"])));
}

#[test]
fn task_show_tool_includes_empty_tags_array() {
    let (_root, runtime, repo_root) = test_runtime();
    let task = create_task(
        &runtime,
        &repo_root,
        "No tags",
        "Exercise empty tag shape.",
        TaskStatus::Backlog,
        &[],
    );

    let shown = runtime
        .execute_tool_command(
            "orbit.task.show",
            json!({ "id": task.id }),
            Some("codex".to_string()),
            Some("gpt-5.5".to_string()),
        )
        .expect("task show tool succeeds");

    assert_eq!(shown.get("tags"), Some(&json!([])));
}

#[test]
fn task_show_tool_with_context_includes_related_docs() {
    let (_root, runtime, repo_root) = test_runtime();
    fs::create_dir_all(repo_root.join("docs")).expect("docs dir");
    fs::write(
        repo_root.join("docs/cli.md"),
        "---\ntype: design\nsummary: CLI command design\npaths: [\"crates/orbit-cli/**\"]\n---\n# CLI Commands\n",
    )
    .expect("write doc");
    let task = create_task(
        &runtime,
        &repo_root,
        "Show related docs",
        "Exercise MCP context injection.",
        TaskStatus::Backlog,
        &["file:crates/orbit-cli/src/command/docs.rs"],
    );

    let shown = runtime
        .execute_tool_command(
            "orbit.task.show",
            json!({ "id": task.id, "with_context": true, "max_docs": 1 }),
            Some("codex".to_string()),
            Some("gpt-5.5".to_string()),
        )
        .expect("task show tool succeeds");

    assert_eq!(
        shown.get("related_docs"),
        Some(&json!([
            {
                "path": "docs/cli.md",
                "type": "design",
                "summary": "CLI command design",
                "excerpt": "CLI Commands",
                "matched_by": ["path:crates/orbit-cli/**"]
            }
        ]))
    );
}

#[test]
fn task_add_tool_normalizes_tags_at_write_time() {
    let (_root, runtime, _repo_root) = test_runtime();

    let output = runtime
        .execute_tool_command(
            "orbit.task.add",
            json!({
                "title": "Normalized tags",
                "description": "Exercise tag normalization.",
                "workspace": ".",
                "tags": ["  Perf ", "BENCH"],
            }),
            Some("codex".to_string()),
            Some("gpt-5.5".to_string()),
        )
        .expect("task add tool succeeds");

    assert_eq!(output.get("tags"), Some(&json!(["perf", "bench"])));
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

    assert!(output.get("agent").is_none());
    // `model` is internal execution routing; v2 does not persist it, so it
    // round-trips as null (the tool layer emits the key unconditionally).
    assert!(output.get("model").is_none_or(serde_json::Value::is_null));
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

    assert!(output.get("agent").is_none());
    // `model` is internal execution routing; v2 does not persist it.
    assert!(output.get("model").is_none_or(serde_json::Value::is_null));
}

#[test]
fn task_update_tool_rejects_dropped_task_types() {
    let (_root, runtime, repo_root) = test_runtime();
    let task = create_task(
        &runtime,
        &repo_root,
        "Retype fixture",
        "Task type update fixture.",
        TaskStatus::Backlog,
        &[],
    );

    for dropped_type in ["task", "epic", "issue", "friction"] {
        let message = invalid_input_message(runtime.execute_tool_command(
            "orbit.task.update",
            json!({
                "id": task.id.clone(),
                "type": dropped_type,
            }),
            Some("codex".to_string()),
            Some("gpt-5.5".to_string()),
        ));
        assert!(message.contains(dropped_type), "{message}");
        assert!(
            message.contains("feature, bug, refactor, chore"),
            "{message}"
        );
    }
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
fn task_update_tool_persists_source_task_id_and_history() {
    let (_root, runtime, repo_root) = test_runtime();
    let source = create_task(
        &runtime,
        &repo_root,
        "Regression source",
        "Existing task that introduced the defect.",
        TaskStatus::Done,
        &[],
    );
    let added = runtime
        .execute_tool_command(
            "orbit.task.add",
            json!({
                "title": "Bug without source",
                "description": "A bug whose source is discovered later.",
                "workspace": ".",
                "type": "bug",
            }),
            Some("codex".to_string()),
            Some("gpt-5.5".to_string()),
        )
        .expect("task add tool succeeds");
    let task_id = added["id"].as_str().expect("task id").to_string();
    let created_updated_at = added["updated_at"]
        .as_str()
        .expect("created updated_at")
        .to_string();
    assert_eq!(added.get("type").and_then(Value::as_str), Some("bug"));
    assert_eq!(added.get("source_task_id"), Some(&Value::Null));

    let output = runtime
        .execute_tool_command(
            "orbit.task.update",
            json!({
                "id": task_id,
                "model": "claude",
                "source_task_id": source.id.clone(),
            }),
            None,
            None,
        )
        .expect("task update tool succeeds");

    assert_eq!(
        output.get("source_task_id").and_then(Value::as_str),
        Some(source.id.as_str())
    );
    assert_ne!(
        output.get("updated_at").and_then(Value::as_str),
        Some(created_updated_at.as_str())
    );
    assert!(
        output["history"]
            .as_array()
            .expect("history")
            .iter()
            .any(|event| {
                event.get("event").and_then(Value::as_str) == Some("updated")
                    && event
                        .get("note")
                        .and_then(Value::as_str)
                        .is_some_and(|note| note.contains("source_task_id"))
            })
    );

    let shown = runtime
        .execute_tool_command(
            "orbit.task.show",
            json!({ "id": output["id"].as_str().expect("task id") }),
            Some("codex".to_string()),
            Some("gpt-5.5".to_string()),
        )
        .expect("task show tool succeeds");
    assert_eq!(
        shown.get("source_task_id").and_then(Value::as_str),
        Some(source.id.as_str())
    );
}

#[test]
fn task_update_tool_clears_source_task_id_with_empty_string() {
    let (_root, runtime, repo_root) = test_runtime();
    let source = create_task(
        &runtime,
        &repo_root,
        "Regression source",
        "Existing task that introduced the defect.",
        TaskStatus::Done,
        &[],
    );
    let added = runtime
        .execute_tool_command(
            "orbit.task.add",
            json!({
                "title": "Bug with source",
                "description": "A bug whose source should be cleared.",
                "workspace": ".",
                "type": "bug",
                "source_task_id": source.id,
            }),
            Some("codex".to_string()),
            Some("gpt-5.5".to_string()),
        )
        .expect("task add tool succeeds");

    let output = runtime
        .execute_tool_command(
            "orbit.task.update",
            json!({
                "id": added["id"].as_str().expect("task id"),
                "source_task_id": "",
            }),
            Some("codex".to_string()),
            Some("gpt-5.5".to_string()),
        )
        .expect("task update tool succeeds");

    assert_eq!(output.get("source_task_id"), Some(&Value::Null));
    assert!(
        output["history"]
            .as_array()
            .expect("history")
            .iter()
            .any(|event| {
                event.get("event").and_then(Value::as_str) == Some("updated")
                    && event
                        .get("note")
                        .and_then(Value::as_str)
                        .is_some_and(|note| note.contains("source_task_id"))
            })
    );
}

#[test]
fn task_update_tool_stores_unresolved_source_task_id_matching_add() {
    let (_root, runtime, _repo_root) = test_runtime();
    let unresolved_from_update = "ORB-99999";
    let unresolved_from_add = "ORB-99998";

    let added = runtime
        .execute_tool_command(
            "orbit.task.add",
            json!({
                "title": "Bug without resolved source",
                "description": "A bug whose source ID is known before its task exists.",
                "workspace": ".",
                "type": "bug",
                "source_task_id": unresolved_from_add,
            }),
            Some("codex".to_string()),
            Some("gpt-5.5".to_string()),
        )
        .expect("add stores a loose source reference");
    assert_eq!(
        added.get("source_task_id").and_then(Value::as_str),
        Some(unresolved_from_add)
    );

    let update_target = runtime
        .execute_tool_command(
            "orbit.task.add",
            json!({
                "title": "Bug with later loose source",
                "description": "Exercise update-side loose reference parity.",
                "workspace": ".",
                "type": "bug",
            }),
            Some("codex".to_string()),
            Some("gpt-5.5".to_string()),
        )
        .expect("task add tool succeeds");
    let output = runtime
        .execute_tool_command(
            "orbit.task.update",
            json!({
                "id": update_target["id"].as_str().expect("task id"),
                "source_task_id": unresolved_from_update,
            }),
            Some("codex".to_string()),
            Some("gpt-5.5".to_string()),
        )
        .expect("update stores a loose source reference just like add");

    assert_eq!(
        output.get("source_task_id").and_then(Value::as_str),
        Some(unresolved_from_update)
    );
}

#[test]
fn task_update_tool_replaces_tags() {
    let (_root, runtime, _repo_root) = test_runtime();

    let added = runtime
        .execute_tool_command(
            "orbit.task.add",
            json!({
                "title": "Replace tags",
                "description": "Exercise tag replacement through tool input.",
                "workspace": ".",
                "tags": ["perf", "bench"],
            }),
            Some("codex".to_string()),
            Some("gpt-5.5".to_string()),
        )
        .expect("task add tool succeeds");
    let task_id = added["id"].as_str().expect("task id").to_string();

    let output = runtime
        .execute_tool_command(
            "orbit.task.update",
            json!({
                "id": task_id,
                "tags": ["docs"],
            }),
            Some("codex".to_string()),
            Some("gpt-5.5".to_string()),
        )
        .expect("task update tool replaces tags");

    assert_eq!(output.get("tags"), Some(&json!(["docs"])));
}

#[test]
fn task_update_tool_replaces_context_files_and_keeps_future_paths() {
    let (_root, runtime, repo_root) = test_runtime();
    let src_dir = repo_root.join("src");
    fs::create_dir_all(&src_dir).expect("create src dir");
    fs::write(src_dir.join("lib.rs"), "pub fn before() {}\n").expect("write source file");
    fs::write(src_dir.join("main.rs"), "fn main() {}\n").expect("write source file");

    let added = runtime
        .execute_tool_command(
            "orbit.task.add",
            json!({
                "title": "Context update",
                "description": "Exercise context_files replacement through tool input.",
                "workspace": repo_root.to_string_lossy(),
                "context_files": ["file:src/lib.rs"],
            }),
            Some("codex".to_string()),
            Some("gpt-5.5".to_string()),
        )
        .expect("task add tool succeeds");
    let task_id = added["id"].as_str().expect("task id").to_string();
    let created_updated_at = added["updated_at"]
        .as_str()
        .expect("created updated_at")
        .to_string();

    let output = runtime
        .execute_tool_command(
            "orbit.task.update",
            json!({
                "id": task_id,
                "context_files": ["[\"file:src/main.rs\", \"file:src/future.rs\"]"],
            }),
            Some("codex".to_string()),
            Some("gpt-5.5".to_string()),
        )
        .expect("task update tool replaces context_files");

    assert_eq!(
        output.get("context_files"),
        Some(&json!(["file:src/main.rs", "file:src/future.rs"]))
    );
    assert_ne!(
        output.get("updated_at").and_then(Value::as_str),
        Some(created_updated_at.as_str())
    );

    let shown = runtime
        .execute_tool_command(
            "orbit.task.show",
            json!({ "id": output["id"].as_str().expect("task id") }),
            Some("codex".to_string()),
            Some("gpt-5.5".to_string()),
        )
        .expect("task show tool succeeds");
    assert_eq!(
        shown.get("context_files"),
        Some(&json!(["file:src/main.rs", "file:src/future.rs"]))
    );

    let events = runtime
        .list_session_events(10)
        .expect("session events")
        .into_iter()
        .filter(|event| event.event_type == "TaskUpdated")
        .collect::<Vec<_>>();
    assert!(
        events
            .iter()
            .any(|event| event.payload["data"]["id"] == shown["id"]),
        "{events:#?}"
    );
}

#[test]
fn task_list_and_search_tools_filter_by_tags_with_and_semantics() {
    let (_root, runtime, _repo_root) = test_runtime();
    for (title, tags) in [
        ("Perf task", json!(["perf"])),
        ("Bench task", json!(["bench"])),
        ("Perf bench task", json!(["perf", "bench"])),
    ] {
        runtime
            .execute_tool_command(
                "orbit.task.add",
                json!({
                    "title": title,
                    "description": "Shared tag-search marker.",
                    "workspace": ".",
                    "tags": tags,
                }),
                Some("codex".to_string()),
                Some("gpt-5.5".to_string()),
            )
            .expect("create tagged task");
    }

    let perf_list = runtime
        .execute_tool_command(
            "orbit.task.list",
            json!({ "tag": ["perf"] }),
            Some("codex".to_string()),
            Some("gpt-5.5".to_string()),
        )
        .expect("list by tag");
    assert_task_titles(&perf_list, &["Perf task", "Perf bench task"]);

    let both_list = runtime
        .execute_tool_command(
            "orbit.task.list",
            json!({ "tag": ["perf", "bench"] }),
            Some("codex".to_string()),
            Some("gpt-5.5".to_string()),
        )
        .expect("list by both tags");
    assert_task_titles(&both_list, &["Perf bench task"]);

    // ORB-00202: `orbit.task.search` was deleted; the search+tag case
    // routes through `orbit.search --kind task --tag <...>`. Results land
    // under `output["results"]` rather than the top-level array.
    let bench_search = runtime
        .execute_tool_command(
            "orbit.search",
            json!({ "query": "tag-search", "kind": "task", "tag": ["bench"] }),
            Some("codex".to_string()),
            Some("gpt-5.5".to_string()),
        )
        .expect("search by tag");
    assert_task_titles(&bench_search["results"], &["Bench task", "Perf bench task"]);

    let both_search = runtime
        .execute_tool_command(
            "orbit.search",
            json!({ "query": "tag-search", "kind": "task", "tag": ["perf", "bench"] }),
            Some("codex".to_string()),
            Some("gpt-5.5".to_string()),
        )
        .expect("search by both tags");
    assert_task_titles(&both_search["results"], &["Perf bench task"]);
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
        .expect_err("agent input should fail");

    assert!(error.to_string().contains("use `model`"));
}
