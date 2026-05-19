//! `orbit mcp` — MCP client integration and server.
//!
//! `orbit mcp init/remove` manages local client integration for Claude Code,
//! Codex, Gemini, and Grok. `orbit mcp serve` serves the Orbit tool surface over
//! MCP so external clients can discover and invoke Orbit operations with typed
//! JSON schemas.

mod setup;

use std::path::Path;
use std::sync::Arc;
use std::time::Instant;

use clap::{Args, Subcommand};
use orbit_common::types::{AuditEventStatus, ToolSchema, audit_execution_id};
use orbit_core::command::tool::{ToolEntryPoint, audit_role_label};
use orbit_core::{
    AuditEventInsertParams, NotFoundKind, OrbitError, OrbitRuntime, redact_sensitive_env_text,
};
use orbit_mcp::McpHost;
use serde_json::Value;

use crate::command::Execute;

pub(crate) use setup::init_auto_for_workspace;
pub use setup::{InitArgs, RemoveArgs};

pub(crate) const ORBIT_MCP_SERVER_ID: &str = "orbit";

pub(crate) const TASK_TOOL_NAMES: &[&str] = &[
    "orbit.task.add",
    "orbit.task.approve",
    "orbit.task.artifact.put",
    "orbit.task.delete",
    "orbit.task.lint",
    "orbit.task.list",
    "orbit.task.search",
    "orbit.task.locks",
    "orbit.task.locks.release",
    "orbit.task.locks.reserve",
    "orbit.task.reject",
    "orbit.task.review_thread.add",
    "orbit.task.review_thread.list",
    "orbit.task.review_thread.reply",
    "orbit.task.review_thread.resolve",
    "orbit.task.show",
    "orbit.task.start",
    "orbit.task.update",
];

pub(crate) const FRICTION_TOOL_NAMES: &[&str] = &[
    "orbit.friction.add",
    "orbit.friction.list",
    "orbit.friction.resolve",
    "orbit.friction.show",
    "orbit.friction.stats",
    "orbit.friction.tags",
    "orbit.friction.update",
];

pub(crate) const GRAPH_READ_TOOL_NAMES: &[&str] = &[
    "orbit.graph.callers",
    "orbit.graph.deps",
    "orbit.graph.history",
    "orbit.graph.implementors",
    "orbit.graph.overview",
    "orbit.graph.pack",
    "orbit.graph.refs",
    "orbit.graph.search",
    "orbit.graph.show",
];

pub(crate) const SEMANTIC_READ_TOOL_NAMES: &[&str] =
    &["orbit.semantic.search", "orbit.semantic.related"];

pub(crate) const DOCS_TOOL_NAMES: &[&str] = &[
    "orbit.docs.list",
    "orbit.docs.show",
    "orbit.docs.search",
    "orbit.docs.add",
    "orbit.docs.reindex",
    "orbit.docs.migrate",
];

pub(crate) const LEARNING_TOOL_NAMES: &[&str] = &[
    "orbit.learning.add",
    "orbit.learning.comment.add",
    "orbit.learning.comment.delete",
    "orbit.learning.comment.list",
    "orbit.learning.list",
    "orbit.learning.search",
    "orbit.learning.show",
    "orbit.learning.update",
    "orbit.learning.supersede",
    "orbit.learning.upvote",
    "orbit.learning.prune",
    "orbit.learning.reindex",
];

pub(crate) fn safe_mcp_tool_names() -> Vec<&'static str> {
    let mut names = Vec::with_capacity(
        TASK_TOOL_NAMES.len()
            + FRICTION_TOOL_NAMES.len()
            + GRAPH_READ_TOOL_NAMES.len()
            + SEMANTIC_READ_TOOL_NAMES.len()
            + DOCS_TOOL_NAMES.len()
            + LEARNING_TOOL_NAMES.len(),
    );
    names.extend_from_slice(TASK_TOOL_NAMES);
    names.extend_from_slice(FRICTION_TOOL_NAMES);
    names.extend_from_slice(GRAPH_READ_TOOL_NAMES);
    names.extend_from_slice(SEMANTIC_READ_TOOL_NAMES);
    names.extend_from_slice(DOCS_TOOL_NAMES);
    names.extend_from_slice(LEARNING_TOOL_NAMES);
    names
}

pub(crate) fn is_mcp_tool_exposed(name: &str) -> bool {
    TASK_TOOL_NAMES.contains(&name)
        || FRICTION_TOOL_NAMES.contains(&name)
        || GRAPH_READ_TOOL_NAMES.contains(&name)
        || SEMANTIC_READ_TOOL_NAMES.contains(&name)
        || DOCS_TOOL_NAMES.contains(&name)
        || LEARNING_TOOL_NAMES.contains(&name)
}

fn ensure_mcp_tool_exposed(name: &str) -> Result<(), OrbitError> {
    if is_mcp_tool_exposed(name) {
        Ok(())
    } else {
        Err(OrbitError::not_found(NotFoundKind::Tool, name.to_string()))
    }
}

#[derive(Args)]
#[command(
    about = "Register MCP client integrations and run the MCP server",
    arg_required_else_help = true,
    subcommand_required = true
)]
pub struct McpCommand {
    #[command(subcommand)]
    pub command: McpSubcommand,
}

impl Execute for McpCommand {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        self.command.execute(runtime)
    }
}

#[derive(Subcommand)]
pub enum McpSubcommand {
    /// Initialize MCP client integration for the current workspace
    Init(InitArgs),
    /// Remove MCP client integration for the current workspace
    Remove(RemoveArgs),
    /// Serve the Orbit tool registry over Model Context Protocol
    Serve(ServeArgs),
}

impl Execute for McpSubcommand {
    fn execute(self, _runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        match self {
            // All MCP subcommands are dispatched runtime-free via main.rs's
            // pattern match before runtime initialization. They reach this
            // path only if invoked indirectly (currently never), so use the
            // same runtime-less call chain for safety.
            Self::Init(args) => args.execute_without_runtime(None),
            Self::Remove(args) => args.execute_without_runtime(None),
            Self::Serve(args) => args.execute_without_runtime(None),
        }
    }
}

#[derive(Args)]
#[command(about = "Serve the Orbit tool registry over Model Context Protocol")]
pub struct ServeArgs {}

impl ServeArgs {
    pub fn execute_without_runtime(self, root_override: Option<&Path>) -> Result<(), OrbitError> {
        let host: Arc<dyn McpHost> = match OrbitRuntime::try_initialize_existing(root_override)? {
            Some(runtime) => Arc::new(RuntimeMcpHost { runtime }),
            None => {
                let cwd = std::env::current_dir()
                    .map(|p| p.display().to_string())
                    .unwrap_or_else(|_| "<unknown>".to_string());
                eprintln!(
                    "orbit mcp serve: no initialized Orbit workspace discovered from {cwd}; serving empty tool surface"
                );
                Arc::new(EmptyMcpHost)
            }
        };

        let tokio_runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .map_err(|e| OrbitError::Execution(format!("tokio runtime: {e}")))?;

        tokio_runtime.block_on(orbit_mcp::serve_stdio(host))
    }
}

/// [`McpHost`] impl that forwards every MCP operation through the full
/// [`OrbitRuntime`] pipeline.
///
/// Listing is sourced from [`OrbitRuntime::list_tools`], which already filters
/// disabled tools and merges external (non-builtin) entries. Execution is
/// routed through [`OrbitRuntime::execute_tool_command_dispatch`] tagged with
/// [`ToolEntryPoint::Mcp`], so the runtime persists an audit row for every
/// dispatch with the same identity-resolution rules as the CLI path. The
/// `tools/call` preflight (see [`audited_mcp_call`]) wraps the dispatch so
/// rejected names also produce a failure-status audit row.
struct RuntimeMcpHost {
    runtime: OrbitRuntime,
}

impl McpHost for RuntimeMcpHost {
    fn list_tool_schemas(&self) -> Vec<ToolSchema> {
        let tools = self.runtime.list_tools().unwrap_or_default();
        tools
            .into_iter()
            .filter(|tool| tool.enabled && is_mcp_tool_exposed(&tool.name))
            .map(|tool| ToolSchema {
                name: tool.name,
                description: tool.description,
                parameters: tool.parameters,
                builtin: tool.builtin,
            })
            .collect()
    }

    fn call_tool(&self, name: &str, input: Value) -> Result<Value, OrbitError> {
        audited_mcp_call(&self.runtime, name, input)
    }
}

/// Bracket the MCP `tools/call` preflight + dispatch with a single audit
/// boundary so that **both** rejected unknown / unexposed tool names and
/// dispatch failures land in the SQLite audit trail.
///
/// Preflight failures never reach
/// [`OrbitRuntime::execute_tool_command_dispatch`], so the runtime's own audit
/// write is bypassed. This wrapper records that failure path explicitly and
/// then short-circuits. On the success path it delegates to the runtime,
/// which owns the audit row (no dedup needed because `orbit mcp serve` is
/// invoked outside any CLI [`crate::audit_middleware::AuditGuard`]).
fn audited_mcp_call(runtime: &OrbitRuntime, name: &str, input: Value) -> Result<Value, OrbitError> {
    if let Err(err) = ensure_mcp_tool_exposed(name) {
        record_mcp_preflight_failure(runtime, name, &input, &err);
        return Err(err);
    }

    runtime
        .execute_tool_command_dispatch(name, input, None, None, ToolEntryPoint::Mcp)
        .map(|outcome| outcome.value)
}

fn record_mcp_preflight_failure(
    runtime: &OrbitRuntime,
    name: &str,
    input: &Value,
    err: &OrbitError,
) {
    let start = Instant::now();
    let role = audit_role_label(input, None, None);
    let duration_ms = (start.elapsed().as_millis() as i64).max(1);
    let working_directory = std::env::current_dir()
        .map(|path| path.to_string_lossy().into_owned())
        .unwrap_or_else(|_| ".".to_string());

    let params = AuditEventInsertParams {
        execution_id: audit_execution_id("exec"),
        command: "tool".to_string(),
        subcommand: Some(ToolEntryPoint::Mcp.audit_subcommand().to_string()),
        tool_name: Some(name.to_string()),
        target_type: Some("tool".to_string()),
        target_id: Some(name.to_string()),
        role,
        status: AuditEventStatus::Failure,
        exit_code: 1,
        duration_ms,
        working_directory,
        arguments_json: None,
        stdout_truncated: None,
        stderr_truncated: None,
        error_message: Some(redact_sensitive_env_text(&err.to_string())),
        host: std::env::var("HOSTNAME").ok(),
        pid: std::process::id(),
        session_id: None,
        task_id: input
            .get("task_id")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned)
            .or_else(|| std::env::var("ORBIT_TASK_ID").ok())
            .filter(|s| !s.is_empty()),
        job_run_id: input
            .get("job_run_id")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned)
            .or_else(|| std::env::var("ORBIT_RUN_ID").ok())
            .filter(|s| !s.is_empty()),
        activity_id: input
            .get("activity_id")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned)
            .or_else(|| std::env::var("ORBIT_ACTIVITY_ID").ok())
            .filter(|s| !s.is_empty()),
        step_index: input.get("step_index").and_then(Value::as_i64).or_else(|| {
            std::env::var("ORBIT_STEP_INDEX")
                .ok()
                .and_then(|s| s.parse().ok())
        }),
    };

    if let Err(write_err) = runtime.record_audit_event(&params) {
        eprintln!("warning: failed to persist MCP preflight audit event: {write_err}");
    }
}

/// MCP host returned when no initialized Orbit workspace is discoverable.
/// Keeps the stdio transport alive so clients see an empty `tools/list`
/// instead of a connection error.
struct EmptyMcpHost;

impl McpHost for EmptyMcpHost {
    fn list_tool_schemas(&self) -> Vec<ToolSchema> {
        Vec::new()
    }

    fn call_tool(&self, name: &str, _input: Value) -> Result<Value, OrbitError> {
        Err(OrbitError::not_found(NotFoundKind::Tool, name.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use orbit_core::OrbitRuntime;
    use orbit_mcp::McpHost;

    use super::{
        DOCS_TOOL_NAMES, GRAPH_READ_TOOL_NAMES, LEARNING_TOOL_NAMES, RuntimeMcpHost,
        SEMANTIC_READ_TOOL_NAMES, TASK_TOOL_NAMES, is_mcp_tool_exposed, safe_mcp_tool_names,
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

        for name in SEMANTIC_READ_TOOL_NAMES {
            assert!(
                names.contains(*name),
                "missing runtime semantic read tool: {name}"
            );
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

        for name in SEMANTIC_READ_TOOL_NAMES {
            assert!(
                listed.contains(*name),
                "client-visible MCP tool list missing semantic read tool: {name}"
            );
        }

        for name in DOCS_TOOL_NAMES {
            assert!(names.contains(*name), "missing runtime docs tool: {name}");
            assert!(is_mcp_tool_exposed(name));
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
            let runtime = OrbitRuntime::in_memory().expect("build test runtime");
            let host = RuntimeMcpHost {
                runtime: runtime.clone(),
            };
            let value = host
                .call_tool("orbit.task.search", json!({ "query": "anything" }))
                .expect("dispatch ok");
            assert!(value.is_array(), "task search returns an array");

            let events = runtime
                .list_audit_events(None, Some("orbit.task.search".to_string()), None, None, 16)
                .expect("list audit events");
            assert_eq!(events.len(), 1, "exactly one audit row for happy path");
            assert_eq!(events[0].subcommand.as_deref(), Some("run-mcp"));
            assert_eq!(events[0].status, AuditEventStatus::Success);
        }

        #[test]
        fn learning_search_is_exposed_to_mcp_dispatch() {
            let runtime = OrbitRuntime::in_memory().expect("build test runtime");
            let value = audited_mcp_call(&runtime, "orbit.learning.search", json!({}))
                .expect("learning search dispatch ok");
            assert!(value.is_array(), "learning search returns an array");
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
            assert!(error.to_string().contains(
                "use --force to delete tasks not in proposed, friction, or rejected status"
            ));
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
}
