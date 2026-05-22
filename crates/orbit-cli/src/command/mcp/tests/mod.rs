#![allow(missing_docs)]

// Content moved from inline #[cfg(test)] mod tests in mcp/mod.rs per ORB-00221.

use std::collections::BTreeSet;

use orbit_core::OrbitRuntime;
use orbit_mcp::McpHost;

use super::{
    DOCS_TOOL_NAMES, GRAPH_READ_TOOL_NAMES, LEARNING_TOOL_NAMES, RuntimeMcpHost, SEARCH_TOOL_NAMES,
    SEMANTIC_TOOL_NAMES, TASK_TOOL_NAMES, is_mcp_tool_exposed, safe_mcp_tool_names,
};

#[test]
fn safe_surface_matches_runtime_graph_and_task_tools() {
    let runtime = OrbitRuntime::in_memory().expect("build test runtime");
    let names: BTreeSet<String> = runtime
        .list_tools()
        .expect("list tools")
        .into_iter()
        .map(|tool| tool.name)
        .collect();
    let safe_names: BTreeSet<&str> = safe_mcp_tool_names().into_iter().collect();

    for name in TASK_TOOL_NAMES {
        assert!(names.contains(*name), "missing runtime task tool: {name}");
        assert!(is_mcp_tool_exposed(name));
    }

    for name in names.iter().filter(|name| name.starts_with("orbit.task.")) {
        assert!(
            safe_names.contains(name.as_str()),
            "runtime task tool missing from safe MCP surface: {name}"
        );
    }

    for name in GRAPH_READ_TOOL_NAMES {
        assert!(
            names.contains(*name),
            "missing runtime graph read tool: {name}"
        );
        assert!(is_mcp_tool_exposed(name));
    }

    for name in SEMANTIC_TOOL_NAMES {
        assert!(
            names.contains(*name),
            "missing runtime semantic read tool: {name}"
        );
        assert!(is_mcp_tool_exposed(name));
    }

    for name in SEARCH_TOOL_NAMES {
        assert!(names.contains(*name), "missing runtime search tool: {name}");
        assert!(is_mcp_tool_exposed(name));
    }

    for name in DOCS_TOOL_NAMES {
        assert!(names.contains(*name), "missing runtime docs tool: {name}");
        assert!(is_mcp_tool_exposed(name));
    }

    for name in LEARNING_TOOL_NAMES {
        assert!(
            names.contains(*name),
            "missing runtime learning tool: {name}"
        );
        assert!(is_mcp_tool_exposed(name));
    }

    for name in names
        .iter()
        .filter(|name| name.starts_with("orbit.learning."))
    {
        assert!(
            safe_names.contains(name.as_str()),
            "runtime learning tool missing from safe MCP surface: {name}"
        );
    }

    for name in [
        "orbit.graph.add",
        "orbit.graph.delete",
        "orbit.graph.move",
        "orbit.graph.write",
    ] {
        assert!(
            !names.contains(name),
            "runtime exposes graph write tool: {name}"
        );
        assert!(!is_mcp_tool_exposed(name));
    }

    assert!(!is_mcp_tool_exposed("orbit.state.get"));
    assert!(!is_mcp_tool_exposed("demo.hello"));
}

#[test]
fn runtime_mcp_host_lists_safe_graph_tools_for_clients() {
    let runtime = OrbitRuntime::in_memory().expect("build test runtime");
    let host = RuntimeMcpHost { runtime };
    let listed: BTreeSet<String> = host
        .list_tool_schemas()
        .into_iter()
        .map(|schema| schema.name)
        .collect();

    for name in GRAPH_READ_TOOL_NAMES {
        assert!(
            listed.contains(*name),
            "client-visible MCP tool list missing graph read tool: {name}"
        );
    }

    for name in SEMANTIC_TOOL_NAMES {
        assert!(
            listed.contains(*name),
            "client-visible MCP tool list missing semantic read tool: {name}"
        );
    }

    for name in SEARCH_TOOL_NAMES {
        assert!(
            listed.contains(*name),
            "client-visible MCP tool list missing search tool: {name}"
        );
    }

    for name in DOCS_TOOL_NAMES {
        assert!(
            listed.contains(*name),
            "client-visible MCP tool list missing docs tool: {name}"
        );
    }

    for name in LEARNING_TOOL_NAMES {
        assert!(
            listed.contains(*name),
            "client-visible MCP tool list missing learning tool: {name}"
        );
    }

    for name in [
        "orbit.graph.add",
        "orbit.graph.delete",
        "orbit.graph.move",
        "orbit.graph.write",
    ] {
        assert!(
            !listed.contains(name),
            "client-visible MCP tool list exposes graph write tool: {name}"
        );
    }

    // ORB-00195: MCP `tools/list` (via schema_to_tool) must advertise allow_fuzzy for
    // the sanitized orbit_graph_search so agents discover the fuzzy fallback.
    let search_schema = host
        .list_tool_schemas()
        .into_iter()
        .find(|s| s.name == "orbit.graph.search")
        .expect("orbit.graph.search schema must be exposed to MCP");
    let fuzzy = search_schema
        .parameters
        .iter()
        .find(|p| p.name == "allow_fuzzy")
        .expect("allow_fuzzy must be declared in ToolSchema for discoverability");
    assert_eq!(fuzzy.param_type, "boolean", "allow_fuzzy is boolean input");
    assert!(!fuzzy.required, "allow_fuzzy is optional");
    assert!(
        fuzzy.description.contains("fuzzy") || fuzzy.description.contains("fallback"),
        "description must mention fuzzy fallback: {}",
        fuzzy.description
    );
}

mod audited_mcp_call_tests {
    use orbit_common::types::AuditEventStatus;
    use orbit_core::OrbitRuntime;
    use orbit_core::TaskStatus;
    use orbit_core::command::task::TaskAddParams;
    use orbit_mcp::McpHost;
    use serde_json::json;

    use super::super::{RuntimeMcpHost, audited_mcp_call};

    fn create_task(runtime: &OrbitRuntime, status: TaskStatus) -> String {
        runtime
            .add_task(TaskAddParams {
                title: format!("Delete {status}"),
                description: "Exercise MCP task deletion guard.".to_string(),
                workspace_path: Some(".".to_string()),
                status: Some(status),
                ..Default::default()
            })
            .expect("create task")
            .id
    }

    #[test]
    fn preflight_failure_for_unknown_tool_records_failure_audit_row() {
        let runtime = OrbitRuntime::in_memory().expect("build test runtime");
        // The runtime is the source of truth for the audit store; the
        // wrapper writes to the same backing store the MCP host shares.
        let result = audited_mcp_call(&runtime, "orbit.state.get", json!({}));
        assert!(
            result.is_err(),
            "preflight rejects unknown / unexposed tool"
        );

        let events = runtime
            .list_audit_events(None, Some("orbit.state.get".to_string()), None, None, 16)
            .expect("list audit events");
        assert_eq!(events.len(), 1, "preflight failure produced one audit row");
        let row = &events[0];
        assert_eq!(row.command, "tool");
        assert_eq!(row.subcommand.as_deref(), Some("run-mcp"));
        assert_eq!(row.tool_name.as_deref(), Some("orbit.state.get"));
        assert_eq!(row.status, AuditEventStatus::Failure);
        assert_eq!(row.exit_code, 1);
        assert!(row.error_message.is_some());
        assert!(
            row.duration_ms >= 1,
            "duration_ms clamped to >= 1 (got {})",
            row.duration_ms
        );
    }

    #[test]
    fn happy_path_dispatch_records_one_audit_row_via_runtime() {
        // ORB-00202: migrated from deleted `orbit.task.search` to
        // `orbit.search`, the unified replacement.
        let runtime = OrbitRuntime::in_memory().expect("build test runtime");
        let host = RuntimeMcpHost {
            runtime: runtime.clone(),
        };
        let value = host
            .call_tool(
                "orbit.search",
                json!({ "query": "anything", "kind": "task" }),
            )
            .expect("dispatch ok");
        assert!(
            value.get("results").is_some(),
            "orbit.search returns wrapped results"
        );

        let events = runtime
            .list_audit_events(None, Some("orbit.search".to_string()), None, None, 16)
            .expect("list audit events");
        assert_eq!(events.len(), 1, "exactly one audit row for happy path");
        assert_eq!(events[0].subcommand.as_deref(), Some("run-mcp"));
        assert_eq!(events[0].status, AuditEventStatus::Success);
    }

    #[test]
    fn orbit_search_is_exposed_to_mcp_dispatch() {
        // ORB-00202: `orbit.learning.search` was deleted; the unified
        // `orbit.search` surface is exposed instead.
        let runtime = OrbitRuntime::in_memory().expect("build test runtime");
        let value = audited_mcp_call(&runtime, "orbit.search", json!({ "query": "anything" }))
            .expect("orbit.search dispatch ok");
        assert!(
            value.get("results").is_some(),
            "orbit.search returns wrapped results"
        );
    }

    #[test]
    fn mcp_graph_search_accepts_allow_fuzzy_and_returns_result_shape() {
        let runtime = OrbitRuntime::in_memory().expect("build test runtime");
        // MCP-path (preflight + audited dispatch) regression for ORB-00195.
        // Exercises allow_fuzzy passthrough for both canonical and (via adapter) sanitized names.
        // In-memory test runtime has no graph data, so execution may yield knowledge err;
        // the important check is that preflight accepts the exposed tool+param (no "not found").
        let res = audited_mcp_call(
            &runtime,
            "orbit.graph.search",
            json!({"query": "TypoForFuzzyTest", "allow_fuzzy": true, "limit": 3}),
        );
        match res {
            Ok(body) => {
                assert!(body.get("total").is_some());
                assert!(body.get("results").is_some());
            }
            Err(e) => {
                let msg = e.to_string().to_lowercase();
                assert!(
                    !msg.contains("not found") && !msg.contains("unknown") && !msg.contains("tool"),
                    "preflight must accept orbit.graph.search (MCP-exposed); execution err ok in empty fixture: {}",
                    e
                );
            }
        }
    }

    #[test]
    fn task_delete_rejects_unforced_protected_status_and_audits_failure() {
        let runtime = OrbitRuntime::in_memory().expect("build test runtime");
        let task_id = create_task(&runtime, TaskStatus::Backlog);
        let host = RuntimeMcpHost {
            runtime: runtime.clone(),
        };

        let result = host.call_tool(
            "orbit.task.delete",
            json!({ "id": task_id, "model": "gpt-5.5" }),
        );

        let error = result.expect_err("unforced protected delete fails");
        assert!(
            error.to_string().contains(
                "use --force to delete tasks not in proposed, friction, or rejected status"
            )
        );
        runtime
            .get_task(&task_id)
            .expect("unforced protected task remains");

        let events = runtime
            .list_audit_events(None, Some("orbit.task.delete".to_string()), None, None, 16)
            .expect("list audit events");
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].subcommand.as_deref(), Some("run-mcp"));
        assert_eq!(events[0].status, AuditEventStatus::Failure);
        assert_eq!(events[0].exit_code, 1);
        assert!(
            events[0]
                .error_message
                .as_deref()
                .is_some_and(|message| message.contains("use --force"))
        );
    }

    #[test]
    fn task_delete_allows_unforced_proposed_and_rejected_tasks_over_mcp() {
        let runtime = OrbitRuntime::in_memory().expect("build test runtime");
        let host = RuntimeMcpHost {
            runtime: runtime.clone(),
        };

        for status in [TaskStatus::Proposed, TaskStatus::Rejected] {
            let task_id = create_task(&runtime, status);
            let value = host
                .call_tool(
                    "orbit.task.delete",
                    json!({ "id": task_id, "model": "gpt-5.5" }),
                )
                .expect("unprotected delete succeeds");
            assert_eq!(value, json!({ "id": task_id, "deleted": true }));
        }

        let events = runtime
            .list_audit_events(None, Some("orbit.task.delete".to_string()), None, None, 16)
            .expect("list audit events");
        assert_eq!(events.len(), 2);
        assert!(events.iter().all(|event| {
            event.subcommand.as_deref() == Some("run-mcp")
                && event.status == AuditEventStatus::Success
        }));
    }

    #[test]
    fn task_delete_allows_forced_protected_status_over_mcp_and_audits_success() {
        let runtime = OrbitRuntime::in_memory().expect("build test runtime");
        let task_id = create_task(&runtime, TaskStatus::InProgress);
        let host = RuntimeMcpHost {
            runtime: runtime.clone(),
        };

        let value = host
            .call_tool(
                "orbit.task.delete",
                json!({ "id": task_id, "force": true, "model": "gpt-5.5" }),
            )
            .expect("forced protected delete succeeds");

        assert_eq!(value, json!({ "id": task_id, "deleted": true }));
        assert!(runtime.get_task(&task_id).is_err(), "task was deleted");

        let events = runtime
            .list_audit_events(None, Some("orbit.task.delete".to_string()), None, None, 16)
            .expect("list audit events");
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].subcommand.as_deref(), Some("run-mcp"));
        assert_eq!(events[0].status, AuditEventStatus::Success);
        assert_eq!(events[0].exit_code, 0);
    }
}
