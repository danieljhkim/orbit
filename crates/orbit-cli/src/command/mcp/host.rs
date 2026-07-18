//! MCP host implementations and audit bracketing.
//!
//! Registry-backed listing is sourced from
//! [`OrbitRuntime::list_mcp_tool_definitions`], which filters disabled tools
//! while preserving each builtin schema's adjacent MCP policy.
//! Registry-backed and adapter-owned execution both use the runtime audit
//! boundary tagged with [`ToolEntryPoint::Mcp`], so every dispatch has the same
//! identity-resolution rules as the CLI path. Adapter preflight lives inside
//! that boundary; registry preflight failures are recorded explicitly before
//! runtime dispatch. Either rejection path produces a failure-status row.

use std::time::Instant;

use orbit_common::types::{
    AuditEventStatus, LearningInjectionState, McpToolDefinition, McpToolPolicyError,
    ToolSessionContext, audit_execution_id, validate_mcp_tool_definitions,
};
use orbit_core::command::tool::{ToolEntryPoint, audit_role_label};
use orbit_core::{
    AuditEventInsertParams, LearningSearchParams, NotFoundKind, OrbitError, OrbitRuntime,
    redact_sensitive_env_text,
};
use orbit_mcp::McpHost;
use serde_json::{Value, json};

pub(crate) const ORBIT_MCP_SERVER_ID: &str = "orbit";

pub(crate) fn canonical_mcp_tool_definitions() -> Result<Vec<McpToolDefinition>, McpToolPolicyError>
{
    let mut definitions = orbit_core::canonical_builtin_mcp_tool_definitions()?;
    definitions.extend(orbit_mcp::graph_mcp_tool_definitions()?);
    validate_mcp_tool_definitions(&definitions)?;
    Ok(definitions)
}

pub(crate) fn safe_mcp_tool_names() -> Vec<String> {
    canonical_mcp_tool_definitions()
        .map(|definitions| {
            definitions
                .into_iter()
                .map(|definition| definition.schema.name)
                .collect()
        })
        .unwrap_or_default()
}

pub(crate) fn is_mcp_tool_exposed(name: &str) -> bool {
    canonical_mcp_tool_definitions().is_ok_and(|definitions| {
        definitions
            .iter()
            .any(|definition| definition.schema.name == name)
    })
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
    fn list_mcp_tool_definitions(&self) -> Result<Vec<McpToolDefinition>, OrbitError> {
        self.runtime.list_mcp_tool_definitions()
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
    fn list_mcp_tool_definitions(&self) -> Result<Vec<McpToolDefinition>, OrbitError> {
        Ok(Vec::new())
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
