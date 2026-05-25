//! MCP host implementations and audit bracketing.
//!
//! Listing is sourced from [`OrbitRuntime::list_tools`], which already filters
//! disabled tools and merges external (non-builtin) entries. Execution is
//! routed through [`OrbitRuntime::execute_tool_command_dispatch`] tagged with
//! [`ToolEntryPoint::Mcp`], so the runtime persists an audit row for every
//! dispatch with the same identity-resolution rules as the CLI path. The
//! `tools/call` preflight wraps the dispatch so rejected names also produce a
//! failure-status audit row.

use std::time::Instant;

use orbit_common::types::{
    AuditEventStatus, LearningInjectionState, ToolSchema, ToolSessionContext, audit_execution_id,
};
use orbit_core::command::tool::{ToolEntryPoint, audit_role_label, graph_backend_for_audit};
use orbit_core::{
    AuditEventInsertParams, LearningSearchParams, NotFoundKind, OrbitError, OrbitRuntime,
    redact_sensitive_env_text,
};
use orbit_mcp::{McpHost, McpToolAudit, McpToolAuditStatus};
use serde_json::{Value, json};

pub(crate) const ORBIT_MCP_SERVER_ID: &str = "orbit";

pub(crate) const TASK_TOOL_NAMES: &[&str] = &[
    "orbit.task.add",
    "orbit.task.approve",
    "orbit.task.artifact.put",
    // ORB-00289: `orbit.task.delete` and `orbit.task.lint` are admin-only
    // and remain reachable through the CLI / `runtime.run_tool` path,
    // but are not exposed on the agent MCP surface.
    "orbit.task.list",
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
    "orbit.friction.tags",
    "orbit.friction.update",
];

pub(crate) const GRAPH_READ_TOOL_NAMES: &[&str] = &[
    "orbit.graph.callers",
    "orbit.graph.deps",
    "orbit.graph.implementors",
    "orbit.graph.overview",
    "orbit.graph.pack",
    "orbit.graph.refs",
    "orbit.graph.search",
    "orbit.graph.show",
];

pub(crate) const SEARCH_TOOL_NAMES: &[&str] = &["orbit.search"];

// ORB-00289: `orbit.semantic.uninstall` is admin-only (destructive teardown
// of the local semantic index) and is no longer exposed on the agent MCP
// surface; the CLI / `runtime.run_tool` path retains it. The constant is
// kept (empty) so the aggregation in `safe_mcp_tool_names` and the test
// chain in `mcp/tests/mod.rs` stay structurally symmetric.
pub(crate) const SEMANTIC_TOOL_NAMES: &[&str] = &[];

pub(crate) const ADR_TOOL_NAMES: &[&str] = &[
    "orbit.adr.add",
    // ORB-00289: agents query ADRs via `orbit.search --kind adr`;
    // `orbit.adr.list` remains on the CLI / dashboard `runtime.run_tool`
    // path for admin workflows.
    "orbit.adr.show",
    "orbit.adr.supersede",
    "orbit.adr.update",
];

pub(crate) const DOCS_TOOL_NAMES: &[&str] = &[];

pub(crate) const LEARNING_TOOL_NAMES: &[&str] = &[
    "orbit.learning.add",
    "orbit.learning.comment.add",
    // ORB-00289: `orbit.learning.comment.delete` and `orbit.learning.prune`
    // are destructive admin-only operations and are not exposed on the
    // agent MCP surface; the CLI / `runtime.run_tool` path retains them.
    "orbit.learning.comment.list",
    "orbit.learning.show",
    "orbit.learning.update",
    "orbit.learning.supersede",
    "orbit.learning.upvote",
];

pub(crate) fn safe_mcp_tool_names() -> Vec<&'static str> {
    let mut names = Vec::with_capacity(
        TASK_TOOL_NAMES.len()
            + FRICTION_TOOL_NAMES.len()
            + GRAPH_READ_TOOL_NAMES.len()
            + SEARCH_TOOL_NAMES.len()
            + SEMANTIC_TOOL_NAMES.len()
            + ADR_TOOL_NAMES.len()
            + DOCS_TOOL_NAMES.len()
            + LEARNING_TOOL_NAMES.len(),
    );
    names.extend_from_slice(TASK_TOOL_NAMES);
    names.extend_from_slice(FRICTION_TOOL_NAMES);
    names.extend_from_slice(GRAPH_READ_TOOL_NAMES);
    names.extend_from_slice(SEARCH_TOOL_NAMES);
    names.extend_from_slice(SEMANTIC_TOOL_NAMES);
    names.extend_from_slice(ADR_TOOL_NAMES);
    names.extend_from_slice(DOCS_TOOL_NAMES);
    names.extend_from_slice(LEARNING_TOOL_NAMES);
    names
}

pub(crate) fn is_mcp_tool_exposed(name: &str) -> bool {
    TASK_TOOL_NAMES.contains(&name)
        || FRICTION_TOOL_NAMES.contains(&name)
        || GRAPH_READ_TOOL_NAMES.contains(&name)
        || SEARCH_TOOL_NAMES.contains(&name)
        || SEMANTIC_TOOL_NAMES.contains(&name)
        || ADR_TOOL_NAMES.contains(&name)
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

/// [`McpHost`] impl that forwards every MCP operation through the full
/// [`OrbitRuntime`] pipeline.
pub(super) struct RuntimeMcpHost {
    pub(super) runtime: OrbitRuntime,
}

impl RuntimeMcpHost {
    pub(super) fn new(runtime: OrbitRuntime) -> Self {
        Self { runtime }
    }
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

    fn call_tool(
        &self,
        name: &str,
        input: Value,
        session_context: ToolSessionContext,
    ) -> Result<Value, OrbitError> {
        audited_mcp_call_with_session_context(&self.runtime, name, input, session_context)
    }

    fn call_shadow_tool(
        &self,
        name: &str,
        input: Value,
        _session_context: ToolSessionContext,
    ) -> Result<Value, OrbitError> {
        ensure_mcp_tool_exposed(name)?;
        self.runtime.run_tool(name, input)
    }

    fn record_tool_audit(&self, audit: McpToolAudit) -> Result<(), OrbitError> {
        record_mcp_tool_audit(&self.runtime, audit)
    }

    fn learning_candidates_for_path(
        &self,
        path: &str,
        _session_context: ToolSessionContext,
    ) -> Result<Value, OrbitError> {
        // L-0043: this is adapter-internal lookup, not a client MCP tool call.
        let rows = self.runtime.search_learnings(LearningSearchParams {
            path: Some(path.to_string()),
            tag: None,
            query: None,
            limit: None,
        })?;
        Ok(Value::Array(
            rows.into_iter()
                .map(|row| {
                    json!({
                        "id": row.learning.id,
                        "summary": row.learning.summary,
                        "priority": row.learning.priority,
                        "updated_at": row.learning.updated_at.to_rfc3339(),
                    })
                })
                .collect(),
        ))
    }

    fn get_session_learning_state(
        &self,
        session_id: &str,
    ) -> Result<Option<LearningInjectionState>, OrbitError> {
        self.runtime.get_session_learning_state(session_id)
    }

    fn upsert_session_learning_state(
        &self,
        session_id: &str,
        state: &LearningInjectionState,
    ) -> Result<(), OrbitError> {
        self.runtime
            .upsert_session_learning_state(session_id, state)
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
#[cfg(test)]
pub(super) fn audited_mcp_call(
    runtime: &OrbitRuntime,
    name: &str,
    input: Value,
) -> Result<Value, OrbitError> {
    audited_mcp_call_with_session_context(runtime, name, input, ToolSessionContext::default())
}

pub(super) fn audited_mcp_call_with_session_context(
    runtime: &OrbitRuntime,
    name: &str,
    input: Value,
    session_context: ToolSessionContext,
) -> Result<Value, OrbitError> {
    if let Err(err) = ensure_mcp_tool_exposed(name) {
        record_mcp_preflight_failure(runtime, name, &input, &err);
        return Err(err);
    }

    runtime
        .execute_tool_command_dispatch_with_session_context(
            name,
            input,
            None,
            None,
            ToolEntryPoint::Mcp,
            session_context,
        )
        .map(|outcome| outcome.value)
}

fn record_mcp_preflight_failure(
    runtime: &OrbitRuntime,
    name: &str,
    input: &Value,
    err: &OrbitError,
) {
    let audit = McpToolAudit {
        name: name.to_string(),
        input: input.clone(),
        status: McpToolAuditStatus::Failure,
        duration_ms: 1,
        error_message: Some(err.to_string()),
        backend: graph_backend_for_audit(name, input),
    };
    if let Err(write_err) = record_mcp_tool_audit(runtime, audit) {
        eprintln!("warning: failed to persist MCP preflight audit event: {write_err}");
    }
}

fn record_mcp_tool_audit(runtime: &OrbitRuntime, audit: McpToolAudit) -> Result<(), OrbitError> {
    let start = Instant::now();
    let role = audit_role_label(&audit.input, None, None);
    let duration_ms = audit
        .duration_ms
        .max((start.elapsed().as_millis() as i64).max(1));
    let working_directory = std::env::current_dir()
        .map(|path| path.to_string_lossy().into_owned())
        .unwrap_or_else(|_| ".".to_string());
    let (status, exit_code, error_message) = match audit.status {
        McpToolAuditStatus::Success => (AuditEventStatus::Success, 0, None),
        McpToolAuditStatus::Failure => (
            AuditEventStatus::Failure,
            1,
            audit
                .error_message
                .as_deref()
                .map(redact_sensitive_env_text),
        ),
    };

    let params = AuditEventInsertParams {
        execution_id: audit_execution_id("exec"),
        command: "tool".to_string(),
        subcommand: Some(ToolEntryPoint::Mcp.audit_subcommand().to_string()),
        tool_name: Some(audit.name.clone()),
        target_type: Some("tool".to_string()),
        target_id: Some(audit.name.clone()),
        role,
        status,
        exit_code,
        duration_ms,
        working_directory,
        arguments_json: None,
        stdout_truncated: None,
        stderr_truncated: None,
        error_message,
        host: std::env::var("HOSTNAME").ok(),
        pid: std::process::id(),
        session_id: None,
        task_id: audit
            .input
            .get("task_id")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned)
            .or_else(|| std::env::var("ORBIT_TASK_ID").ok())
            .filter(|s| !s.is_empty()),
        job_run_id: audit
            .input
            .get("job_run_id")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned)
            .or_else(|| std::env::var("ORBIT_RUN_ID").ok())
            .filter(|s| !s.is_empty()),
        activity_id: audit
            .input
            .get("activity_id")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned)
            .or_else(|| std::env::var("ORBIT_ACTIVITY_ID").ok())
            .filter(|s| !s.is_empty()),
        step_index: audit
            .input
            .get("step_index")
            .and_then(Value::as_i64)
            .or_else(|| {
                std::env::var("ORBIT_STEP_INDEX")
                    .ok()
                    .and_then(|s| s.parse().ok())
            }),
        backend: audit
            .backend
            .or_else(|| graph_backend_for_audit(&audit.name, &audit.input)),
    };

    runtime.record_audit_event(&params)
}

/// MCP host returned when no initialized Orbit workspace is discoverable.
/// Keeps the stdio transport alive so clients see an empty `tools/list`
/// instead of a connection error.
pub(super) struct EmptyMcpHost;

impl McpHost for EmptyMcpHost {
    fn list_tool_schemas(&self) -> Vec<ToolSchema> {
        Vec::new()
    }

    fn call_tool(
        &self,
        name: &str,
        _input: Value,
        _session_context: ToolSessionContext,
    ) -> Result<Value, OrbitError> {
        Err(OrbitError::not_found(NotFoundKind::Tool, name.to_string()))
    }
}
