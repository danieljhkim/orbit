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
    AuditEventStatus, LearningInjectionState, McpCapability, McpToolDefinition, McpToolPolicyError,
    McpTransport, ToolSessionContext, audit_execution_id, validate_mcp_tool_definitions,
};
use orbit_core::command::tool::{
    ToolEntryPoint, audit_role_label_for_entry_point, trusted_mcp_audit_context,
};
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

fn ensure_mcp_tool_exposed(
    name: &str,
    session_context: &ToolSessionContext,
) -> Result<(), OrbitError> {
    if !is_mcp_tool_exposed(name) {
        return Err(OrbitError::not_found(NotFoundKind::Tool, name.to_string()));
    }
    let definitions = canonical_mcp_tool_definitions()
        .map_err(|error| OrbitError::InvalidInput(error.to_string()))?;
    let definition = definitions
        .iter()
        .find(|definition| definition.schema.name == name)
        .ok_or_else(|| OrbitError::not_found(NotFoundKind::Tool, name.to_string()))?;
    if definition
        .policy
        .allowed_capabilities()
        .iter()
        .any(|capability| session_context.has_capability(*capability))
    {
        Ok(())
    } else {
        Err(OrbitError::PolicyDenied(format!(
            "MCP tool '{name}' is not exposed to the effective capability set"
        )))
    }
}

fn normalize_trusted_call_context(mut context: ToolSessionContext) -> ToolSessionContext {
    if context.transport.is_none() {
        context.transport = Some(McpTransport::Local);
    }
    if context.effective_capabilities.is_empty() {
        context.effective_capabilities.insert(McpCapability::Agent);
    }
    if context.origin_session_id.is_none() {
        context.origin_session_id = Some(audit_execution_id("mcp-session"));
    }
    if context.mcp_call_id.is_none() {
        context.mcp_call_id = Some(audit_execution_id("mcall"));
    }
    context
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
        audited_mcp_call_with_session_context(
            &self.runtime,
            name,
            input,
            normalize_trusted_call_context(session_context),
        )
    }

    fn call_in_process_tool(
        &self,
        name: &str,
        input: Value,
        session_context: ToolSessionContext,
        dispatch: &mut dyn FnMut(Value, ToolSessionContext) -> Result<Value, OrbitError>,
    ) -> Result<Value, OrbitError> {
        let session_context = normalize_trusted_call_context(session_context);
        let dispatch_context = session_context.clone();
        self.runtime
            .execute_in_process_tool_dispatch(
                name,
                input,
                ToolEntryPoint::Mcp,
                session_context,
                |input| {
                    ensure_mcp_tool_exposed(name, &dispatch_context)?;
                    dispatch(input, dispatch_context)
                },
            )
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
    audited_mcp_call_with_session_context(
        runtime,
        name,
        input,
        normalize_trusted_call_context(ToolSessionContext::default()),
    )
}

pub(super) fn audited_mcp_call_with_session_context(
    runtime: &OrbitRuntime,
    name: &str,
    input: Value,
    session_context: ToolSessionContext,
) -> Result<Value, OrbitError> {
    let session_context = normalize_trusted_call_context(session_context);
    if let Err(err) = ensure_mcp_tool_exposed(name, &session_context) {
        record_mcp_preflight_failure(runtime, name, &session_context, &err);
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
    session_context: &ToolSessionContext,
    err: &OrbitError,
) {
    let start = Instant::now();
    let role = audit_role_label_for_entry_point(&Value::Null, None, None, ToolEntryPoint::Mcp);
    let (audit_context, correlation_error) = trusted_mcp_audit_context(session_context);
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
        status: AuditEventStatus::Denied,
        exit_code: 1,
        duration_ms,
        working_directory,
        arguments_json: None,
        stdout_truncated: None,
        stderr_truncated: None,
        error_message: Some(redact_sensitive_env_text(
            &correlation_error.as_ref().unwrap_or(err).to_string(),
        )),
        host: std::env::var("HOSTNAME").ok(),
        pid: std::process::id(),
        session_id: None,
        workspace_id: session_context.workspace_id.clone(),
        caller_machine_id: session_context.caller_machine_id.clone(),
        caller_host_id: session_context.caller_host_id.clone(),
        process_machine_id: session_context.process_machine_id.clone(),
        process_host_id: session_context.process_host_id.clone(),
        transport: session_context.transport,
        effective_capabilities: session_context.effective_capabilities.clone(),
        origin_session_id: session_context.origin_session_id.clone(),
        mcp_call_id: session_context.mcp_call_id.clone(),
        lease_id: session_context
            .leased_run
            .as_ref()
            .map(|leased_run| leased_run.lease_id.clone()),
        task_id: audit_context.task_id,
        job_run_id: audit_context.job_run_id,
        activity_id: audit_context.activity_id,
        step_index: audit_context.step_index,
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
