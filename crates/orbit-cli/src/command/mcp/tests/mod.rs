#![allow(missing_docs)]

// Content moved from inline #[cfg(test)] mod tests in mcp/mod.rs per ORB-00221.

use std::collections::BTreeSet;

use orbit_core::OrbitRuntime;
use orbit_mcp::McpHost;

use super::host::{
    ADR_TOOL_NAMES, DOCS_TOOL_NAMES, FRICTION_TOOL_NAMES, GRAPH_READ_TOOL_NAMES,
    LEARNING_TOOL_NAMES, RuntimeMcpHost, SEARCH_TOOL_NAMES, SEMANTIC_TOOL_NAMES, TASK_TOOL_NAMES,
    is_mcp_tool_exposed, safe_mcp_tool_names,
};

const EXPECTED_INACTIVE_TOOL_NAMES: &[&str] = &[
    "orbit.docs.index",
    "orbit.docs.migrate",
    "orbit.docs.add",
    "orbit.docs.list",
    "orbit.docs.show",
    "orbit.task.locks",
    "orbit.task.locks.release",
    "orbit.task.locks.reserve",
    "orbit.semantic.index",
    "orbit.semantic.install",
    "orbit.semantic.stats",
    "orbit.learning.sync",
    "orbit.learning.list",
    "orbit.friction.stats",
    // ORB-00289: trimmed admin/destructive tools — CLI path retains them.
    "orbit.adr.list",
    "orbit.semantic.uninstall",
    "orbit.task.delete",
    "orbit.task.lint",
    "orbit.learning.comment.delete",
    "orbit.learning.prune",
    // Agent-surface narrowing: triage / human-decision tools — CLI-only.
    "orbit.task.reject",
    "orbit.friction.list",
    "orbit.friction.resolve",
    "orbit.friction.show",
    "orbit.learning.comment.list",
    "orbit.learning.upvote",
];

// ORB-00289 + agent-surface narrowing: admin/destructive and triage tools
// (`orbit.adr.list`, `orbit.semantic.uninstall`, `orbit.task.delete`,
// `orbit.task.lint`, `orbit.task.reject`, `orbit.learning.comment.delete`,
// `orbit.learning.prune`, `orbit.learning.comment.list`,
// `orbit.learning.upvote`, `orbit.friction.list/show/resolve`) deliberately
// omitted — retained on the CLI / `runtime.run_tool` path only.
const REQUIRED_AGENT_FACING_TOOL_NAMES: &[&str] = &[
    "orbit.search",
    "orbit.task.add",
    "orbit.task.approve",
    "orbit.task.artifact.put",
    "orbit.task.show",
    "orbit.task.update",
    "orbit.task.list",
    "orbit.task.review_thread.add",
    "orbit.task.review_thread.list",
    "orbit.task.review_thread.reply",
    "orbit.task.review_thread.resolve",
    "orbit.task.start",
    // ORB-00391: the v1 orbit.graph.* builtins were decommissioned. The agent
    // graph surface is now served by the in-process orbit-graph (v2) adapter in
    // orbit-mcp (see crates/orbit-mcp/src/adapter/graph.rs and its tests), not by
    // the orbit-tools runtime registry, so no orbit.graph.* tool appears here.
    "orbit.adr.add",
    "orbit.adr.show",
    "orbit.adr.supersede",
    "orbit.adr.update",
    "orbit.learning.add",
    "orbit.learning.show",
    "orbit.learning.update",
    "orbit.learning.comment.add",
    "orbit.friction.add",
    "orbit.friction.tags",
    "orbit.friction.update",
];

fn is_runtime_mcp_category_tool(name: &str) -> bool {
    name == "orbit.search"
        || name.starts_with("orbit.task.")
        || name.starts_with("orbit.friction.")
        || name.starts_with("orbit.graph.")
        || name.starts_with("orbit.adr.")
        || name.starts_with("orbit.semantic.")
        || name.starts_with("orbit.docs.")
        || name.starts_with("orbit.learning.")
}

#[test]
fn inactive_tools_are_not_in_the_mcp_safe_surface() {
    let safe_names: BTreeSet<&str> = safe_mcp_tool_names().into_iter().collect();
    assert_eq!(EXPECTED_INACTIVE_TOOL_NAMES.len(), 26);

    for name in EXPECTED_INACTIVE_TOOL_NAMES {
        assert!(
            !safe_names.contains(name),
            "inactive tool leaked into safe MCP names: {name}"
        );
        assert!(
            !is_mcp_tool_exposed(name),
            "inactive tool exposed by MCP preflight: {name}"
        );
    }
}

#[test]
fn safe_surface_matches_runtime_graph_and_task_tools() {
    let runtime = OrbitRuntime::in_memory().expect("build test runtime");
    let names: BTreeSet<String> = runtime
        .list_tools()
        .expect("list tools")
        .into_iter()
        .map(|tool| tool.name)
        .collect();
    let all_names: BTreeSet<String> = runtime
        .list_all_tools()
        .expect("list all tools")
        .into_iter()
        .map(|tool| tool.name)
        .collect();
    let safe_names: BTreeSet<&str> = safe_mcp_tool_names().into_iter().collect();
    let inactive_names: BTreeSet<&str> = EXPECTED_INACTIVE_TOOL_NAMES.iter().copied().collect();

    for name in TASK_TOOL_NAMES
        .iter()
        .chain(FRICTION_TOOL_NAMES)
        .chain(GRAPH_READ_TOOL_NAMES)
        .chain(SEARCH_TOOL_NAMES)
        .chain(SEMANTIC_TOOL_NAMES)
        .chain(ADR_TOOL_NAMES)
        .chain(DOCS_TOOL_NAMES)
        .chain(LEARNING_TOOL_NAMES)
    {
        assert!(
            names.contains(*name),
            "MCP-candidate tool missing from runtime registry: {name}"
        );
    }

    for name in EXPECTED_INACTIVE_TOOL_NAMES {
        assert!(
            !names.contains(*name),
            "inactive tool leaked into default runtime list: {name}"
        );
        assert!(
            all_names.contains(*name),
            "inactive tool should remain registered for inspection: {name}"
        );
        assert!(
            !is_mcp_tool_exposed(name),
            "inactive tool exposed by MCP preflight: {name}"
        );
    }

    for name in REQUIRED_AGENT_FACING_TOOL_NAMES {
        assert!(
            names.contains(*name),
            "required agent-facing tool missing from runtime registry: {name}"
        );
        assert!(
            safe_names.contains(*name),
            "required agent-facing tool missing from safe MCP names: {name}"
        );
        assert!(
            is_mcp_tool_exposed(name),
            "required agent-facing tool rejected by MCP preflight: {name}"
        );
    }

    for name in names
        .iter()
        .filter(|name| is_runtime_mcp_category_tool(name))
    {
        let should_expose = !inactive_names.contains(name.as_str());
        assert!(
            safe_names.contains(name.as_str()) == should_expose,
            "runtime tool MCP exposure mismatch for {name}"
        );
        assert!(
            is_mcp_tool_exposed(name) == should_expose,
            "runtime tool MCP preflight mismatch for {name}"
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
fn runtime_mcp_host_lists_safe_tools_and_no_graph_surface_after_v2_cutover() {
    let runtime = OrbitRuntime::in_memory().expect("build test runtime");
    let host = RuntimeMcpHost { runtime };
    let listed: BTreeSet<String> = host
        .list_tool_schemas()
        .into_iter()
        .map(|schema| schema.name)
        .collect();

    for name in safe_mcp_tool_names() {
        assert!(
            listed.contains(name),
            "client-visible MCP tool list missing safe tool: {name}"
        );
    }

    for name in REQUIRED_AGENT_FACING_TOOL_NAMES {
        assert!(
            listed.contains(*name),
            "client-visible MCP tool list missing required agent-facing tool: {name}"
        );
    }

    for name in EXPECTED_INACTIVE_TOOL_NAMES {
        assert!(
            !listed.contains(*name),
            "client-visible MCP tool list exposes inactive ops tool: {name}"
        );
    }

    // ORB-00391: the orbit-tools runtime host must expose NO `orbit.graph.*`
    // tool. The orbit-mcp adapter gates its in-process orbit-graph (v2) tools on
    // exactly this condition (`host_exposes_graph_tools` → false), so any graph
    // tool leaking back into the host surface would silently disable v2.
    assert!(
        !listed.iter().any(|name| name.starts_with("orbit.graph.")),
        "host must expose no orbit.graph.* tool after the v2 cutover, found: {:?}",
        listed
            .iter()
            .filter(|name| name.starts_with("orbit.graph."))
            .collect::<Vec<_>>()
    );
}

mod audited_mcp_call_tests {
    use std::sync::{Mutex, MutexGuard, OnceLock};
    use std::time::Instant;

    use orbit_common::types::{
        AuditEventStatus, LearningInjectionCaps, LearningInjectionState, LearningReminder,
        LearningScope,
    };
    use orbit_core::LearningEvidence;
    use orbit_core::command::learning_hook::{
        HookOutputFormat, ORBIT_LEARNING_PER_CALL_CAP_ENV, ORBIT_LEARNING_SESSION_CAP_ENV,
        ORBIT_SESSION_ID_ENV, run_pretooluse_input,
    };
    use orbit_core::{LearningCreateParams, OrbitError, OrbitRuntime};
    use orbit_mcp::McpHost;
    use serde_json::json;

    use super::super::host::{RuntimeMcpHost, audited_mcp_call};

    // ORB-00289: the previous `create_task` helper + the three
    // `task_delete_*_over_mcp` tests asserted that `orbit.task.delete` was
    // dispatchable via MCP. That contract was removed when the tool moved to
    // CLI-only (inactive on the agent surface); the generic
    // `inactive_tool_is_rejected_over_mcp_dispatch` test below now covers
    // the rejection-on-inactive contract for every inactive tool, and the
    // delete business logic (force flag, protected statuses, audit row
    // shape) is exercised through `runtime.run_tool` in
    // `orbit-core/.../orbit_tool_host/{task_tools_tests, tests/task_tools}`.

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
                Default::default(),
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
    fn runtime_mcp_host_and_cli_hook_share_session_learning_state() {
        let runtime = OrbitRuntime::in_memory().expect("build test runtime");
        let learning = runtime
            .create_learning(LearningCreateParams {
                summary: "Use the shared state table.".to_string(),
                scope: LearningScope {
                    paths: vec!["crates/orbit-core/src/lib.rs".to_string()],
                    ..Default::default()
                },
                body: String::new(),
                evidence: Vec::<LearningEvidence>::new(),
                created_by: Some("codex".to_string()),
                priority: Some(7),
            })
            .expect("create learning");
        let host = RuntimeMcpHost {
            runtime: runtime.clone(),
        };
        let candidates = host
            .learning_candidates_for_path("crates/orbit-core/src/lib.rs", Default::default())
            .expect("mcp learning candidates");
        let candidates = candidates
            .as_array()
            .expect("candidate array")
            .iter()
            .map(|item| LearningReminder {
                id: item
                    .get("id")
                    .and_then(serde_json::Value::as_str)
                    .expect("candidate id")
                    .to_string(),
                summary: item
                    .get("summary")
                    .and_then(serde_json::Value::as_str)
                    .expect("candidate summary")
                    .to_string(),
                comments: Vec::new(),
            })
            .collect::<Vec<_>>();
        assert_eq!(
            candidates
                .iter()
                .map(|item| item.id.as_str())
                .collect::<Vec<_>>(),
            [learning.id.as_str()]
        );

        let caps = LearningInjectionCaps {
            per_call: 5,
            per_session_hard: 20,
        };
        let mut mcp_state = LearningInjectionState::default();
        let admitted = mcp_state.admit_reminders(&candidates, caps);
        assert_eq!(
            admitted
                .iter()
                .map(|item| item.id.as_str())
                .collect::<Vec<_>>(),
            [learning.id.as_str()]
        );
        host.upsert_session_learning_state("session-shared", &mcp_state)
            .expect("mcp writes shared session state");

        let _guard = EnvGuard::set(&[
            (ORBIT_SESSION_ID_ENV, Some("session-shared")),
            (ORBIT_LEARNING_PER_CALL_CAP_ENV, Some("5")),
            (ORBIT_LEARNING_SESSION_CAP_ENV, Some("20")),
            ("ORBIT_ACTIVE_TASK_ID", None),
            ("ORBIT_TASK_ID", None),
        ]);
        let stdin = json!({
            "tool_name": "mcp__orbit__fs_read",
            "tool_input": {
                "path": "crates/orbit-core/src/lib.rs"
            }
        })
        .to_string();
        let output =
            run_pretooluse_input(&runtime, &stdin, HookOutputFormat::Codex, Instant::now())
                .expect("cli hook succeeds");
        assert_eq!(output, None);

        let persisted = runtime
            .get_session_learning_state("session-shared")
            .expect("read shared session state")
            .expect("session state exists");
        assert_eq!(persisted.count, 1);
        assert!(persisted.emitted_ids.contains(&learning.id));
    }

    #[test]
    fn inactive_tool_is_rejected_over_mcp_dispatch() {
        let runtime = OrbitRuntime::in_memory().expect("build test runtime");
        let error = audited_mcp_call(&runtime, "orbit.learning.list", json!({ "model": "codex" }))
            .expect_err("inactive tool is not callable over MCP");
        assert!(error.to_string().contains("tool"));

        let events = runtime
            .list_audit_events(
                None,
                Some("orbit.learning.list".to_string()),
                None,
                None,
                16,
            )
            .expect("list audit events");
        assert_eq!(events.len(), 1, "preflight failure produced one audit row");
        assert_eq!(events[0].subcommand.as_deref(), Some("run-mcp"));
        assert_eq!(events[0].status, AuditEventStatus::Failure);
    }

    #[test]
    fn friction_list_is_not_exposed_to_mcp_dispatch() {
        let runtime = OrbitRuntime::in_memory().expect("build test runtime");
        let err = audited_mcp_call(&runtime, "orbit.friction.list", json!({ "limit": 1 }))
            .expect_err("orbit.friction.list must be hidden from the agent MCP surface");
        assert!(
            matches!(err, OrbitError::NotFound { .. }),
            "expected NotFound for triage-only orbit.friction.list, got: {err}"
        );
    }

    // ORB-00391: the former `mcp_graph_search_accepts_allow_fuzzy_and_returns_result_shape`
    // test exercised the v1 orbit-knowledge `orbit.graph.search` builtin over the
    // host dispatch path. That builtin was decommissioned; the v2 graph search is
    // served by the in-process orbit-graph adapter in orbit-mcp and is covered by
    // `orbit-mcp/src/adapter/tests/graph.rs` (`graph_tools_invoke_in_process_fixture`).

    struct EnvGuard {
        _lock: MutexGuard<'static, ()>,
        saved: Vec<(&'static str, Option<String>)>,
    }

    impl EnvGuard {
        fn set(values: &[(&'static str, Option<&str>)]) -> Self {
            static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
            let lock = LOCK
                .get_or_init(|| Mutex::new(()))
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            let saved = values
                .iter()
                .map(|(name, _)| (*name, std::env::var(name).ok()))
                .collect::<Vec<_>>();
            for (name, value) in values {
                // SAFETY: EnvGuard serializes process-wide mutations and restores them on drop.
                unsafe {
                    match value {
                        Some(value) => std::env::set_var(name, value),
                        None => std::env::remove_var(name),
                    }
                }
            }
            Self { _lock: lock, saved }
        }
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            for (name, value) in &self.saved {
                // SAFETY: EnvGuard holds the serialization lock until saved values are restored.
                unsafe {
                    match value {
                        Some(value) => std::env::set_var(name, value),
                        None => std::env::remove_var(name),
                    }
                }
            }
        }
    }
}
