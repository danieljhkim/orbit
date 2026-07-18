//! MCP host implementations and audit bracketing.
//!
//! Registry-backed listing is sourced from [`OrbitRuntime::list_tools`], which
//! already filters disabled tools and merges external (non-builtin) entries.
//! Registry-backed and adapter-owned execution both use the runtime audit
//! boundary tagged with [`ToolEntryPoint::Mcp`], so every dispatch has the same
//! identity-resolution rules as the CLI path. Adapter preflight lives inside
//! that boundary; registry preflight failures are recorded explicitly before
//! runtime dispatch. Either rejection path produces a failure-status row.

use std::time::Instant;

use orbit_common::types::{
    AuditEventStatus, LearningInjectionState, McpToolPolicy, ToolSchema, ToolSessionContext,
    audit_execution_id, canonical_mcp_tool_policies, canonical_mcp_tool_policy,
};
use orbit_core::command::tool::{ToolEntryPoint, audit_role_label};
use orbit_core::{
    AuditEventInsertParams, LearningSearchParams, NotFoundKind, OrbitError, OrbitRuntime,
    redact_sensitive_env_text,
};
use orbit_mcp::McpHost;
use serde_json::{Value, json};

pub(crate) const ORBIT_MCP_SERVER_ID: &str = "orbit";

pub(crate) fn safe_mcp_tool_names() -> Vec<&'static str> {
    canonical_mcp_tool_policies()
        .map(|entries| entries.iter().map(|entry| entry.canonical_name).collect())
        .unwrap_or_default()
}

pub(crate) fn is_mcp_tool_exposed(name: &str) -> bool {
    canonical_mcp_tool_policy(name).is_some()
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
        let safe_names = safe_mcp_tool_names();
        assert!(
            orbit_mcp::graph_tool_names()
                .iter()
                .all(|name| safe_names.contains(name)),
            "in-process graph tool names must be a subset of the MCP safe surface"
        );
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

    fn mcp_tool_policy(&self, canonical_name: &str) -> Option<McpToolPolicy> {
        canonical_mcp_tool_policy(canonical_name)
    }

    fn call_tool(
        &self,
        name: &str,
        input: Value,
        session_context: ToolSessionContext,
    ) -> Result<Value, OrbitError> {
        audited_mcp_call_with_session_context(&self.runtime, name, input, session_context)
    }

    fn call_in_process_tool(
        &self,
        name: &str,
        input: Value,
        session_context: ToolSessionContext,
        dispatch: &mut dyn FnMut(Value, ToolSessionContext) -> Result<Value, OrbitError>,
    ) -> Result<Value, OrbitError> {
        self.runtime
            .execute_in_process_tool_dispatch(name, input, ToolEntryPoint::Mcp, |input| {
                ensure_mcp_tool_exposed(name)?;
                dispatch(input, session_context)
            })
            .map(|outcome| outcome.value)
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
