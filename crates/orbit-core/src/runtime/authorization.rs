//! The runtime half of the capability chokepoint [ADR-0260, ORB-10453].
//!
//! `orbit-common::authorization` owns the registry and the decision; this
//! module owns the two things a decision needs a runtime for — reading the
//! caller's envelope off the live tool context, and persisting the denial.
//!
//! # Why the record is written here
//!
//! Every entry surface already audits its own invocation, but not all of them
//! audit the same way and one (the dashboard's direct `run_tool`) does not
//! audit at all. Emitting the authorization row from the decision itself makes
//! "a governed operation was refused" a fact of the decision rather than a
//! property of whichever caller happened to make it, so a denial is queryable
//! with one predicate (`command = 'authorization'`) across every path.
//!
//! The entry-point row still lands too, and still carries
//! [`AuditEventStatus::Denied`]; the two rows answer different questions
//! ("what was refused" versus "what call failed").

use orbit_common::OrbitError;
use orbit_common::governance::authorization::{
    CallerCapabilities, CallerEnvelope, GovernedOperation, authorize, governed_command,
    governed_tool,
};
use orbit_common::observability::audit_id::audit_execution_id;
use orbit_store::contracts::AuditEventInsertParams;
use orbit_types::telemetry::AuditEventStatus;
use orbit_types::tool::ToolSessionContext;

use crate::OrbitRuntime;
use crate::runtime::tool_exec::CapabilityEnforcement;

impl OrbitRuntime {
    /// Authorize a governed tool call, or pass an ungoverned one straight
    /// through.
    ///
    /// Called from `run_tool_with_context_and_role`, which every tool caller
    /// traverses: CLI `tool run`, the CLI's admin `run_tool` bypass, MCP
    /// `tools/call`, the dashboard, the v2 deterministic dispatcher, and agent
    /// loops. There is deliberately no second tool-side guard anywhere.
    pub(crate) fn authorize_tool_operation(
        &self,
        tool_name: &str,
        session_context: &ToolSessionContext,
        capability_enforcement: CapabilityEnforcement,
    ) -> Result<(), OrbitError> {
        let Some(operation) = governed_tool(tool_name) else {
            return Ok(());
        };
        let envelope = match capability_enforcement {
            CapabilityEnforcement::Enforce => CallerEnvelope::from_process_env(session_context),
            CapabilityEnforcement::McpSessionOnly => CallerEnvelope::mcp_session(session_context),
        };
        self.decide_with_envelope(operation, envelope)
    }

    /// Authorize a governed CLI command, or pass an ungoverned one through.
    ///
    /// Called once from the CLI's dispatch chokepoint. Commands reach it by
    /// name because the destructive ones (`workspace teardown`, `audit prune`)
    /// perform their destruction directly rather than through a tool, so the
    /// tool chokepoint never sees them.
    pub fn authorize_command_operation(
        &self,
        command: &str,
        subcommand: &str,
    ) -> Result<(), OrbitError> {
        let Some(operation) = governed_command(command, subcommand) else {
            return Ok(());
        };
        self.decide(operation, &ToolSessionContext::default())
    }

    fn decide(
        &self,
        operation: &'static GovernedOperation,
        session_context: &ToolSessionContext,
    ) -> Result<(), OrbitError> {
        self.decide_with_envelope(operation, CallerEnvelope::from_process_env(session_context))
    }

    fn decide_with_envelope(
        &self,
        operation: &'static GovernedOperation,
        envelope: CallerEnvelope,
    ) -> Result<(), OrbitError> {
        let caller = CallerCapabilities::resolve(&envelope);

        match authorize(operation, &caller) {
            Ok(()) => {
                if caller.is_override() {
                    // The escape hatch is allowed to be easy, not quiet.
                    tracing::warn!(
                        target: "orbit.authorization",
                        operation = operation.id,
                        provenance = %caller.provenance(),
                        "governed operation authorized through the operator override"
                    );
                    self.record_authorization_event(
                        operation,
                        &caller,
                        AuditEventStatus::Success,
                        Some("authorized through the operator override".to_string()),
                    );
                }
                Ok(())
            }
            Err(denial) => {
                tracing::warn!(
                    target: "orbit.authorization",
                    operation = operation.id,
                    provenance = %denial.provenance,
                    granted = %denial.granted,
                    caller_machine_id = denial
                        .remote_caller_grant
                        .as_ref()
                        .map(|grant| grant.caller_machine_id.as_str()),
                    "governed operation denied"
                );
                let message = denial.to_string();
                self.record_authorization_event(
                    operation,
                    &caller,
                    AuditEventStatus::Denied,
                    Some(message.clone()),
                );
                Err(OrbitError::CapabilityDenied(message))
            }
        }
    }

    /// Persist the authorization decision.
    ///
    /// A failed write is logged and swallowed rather than converted into the
    /// caller's error. The decision itself is what the caller asked about, and
    /// on a denial the call is already being refused — turning an unwritable
    /// audit store into a *different* failure would only obscure why the
    /// operation did not run. (This is the opposite trade from
    /// `finalize_successful_dispatch`, which fails a *successful, committed*
    /// mutation whose audit row is missing; there, silence would hide a
    /// completed change.)
    fn record_authorization_event(
        &self,
        operation: &'static GovernedOperation,
        caller: &CallerCapabilities,
        status: AuditEventStatus,
        error_message: Option<String>,
    ) {
        let params = AuditEventInsertParams {
            execution_id: audit_execution_id("authz"),
            command: "authorization".to_string(),
            subcommand: Some(caller.provenance().to_string()),
            tool_name: None,
            target_type: Some("operation".to_string()),
            target_id: Some(operation.id.to_string()),
            role: self.actor_label().to_string(),
            status,
            exit_code: i32::from(status != AuditEventStatus::Success),
            duration_ms: 1,
            working_directory: std::env::current_dir()
                .map(|path| path.to_string_lossy().into_owned())
                .unwrap_or_else(|_| ".".to_string()),
            // A destination-side grant is recorded next to the effective set,
            // not folded into it: `effective` alone cannot distinguish a
            // caller this machine capped from one that never asked for more
            // [ORB-11052]. `effective_capabilities` below stays the resolved
            // set every existing query reads.
            arguments_json: caller.remote_caller_grant().map(|grant| {
                serde_json::json!({
                    "caller_machine_id": grant.caller_machine_id,
                    "granted_capabilities": grant.granted_capabilities,
                    "effective_capabilities": caller.grants(),
                    "source": grant.source,
                    // Which tier answered. Both tiers produce a grant that
                    // looks identical once resolved, so a trail that recorded
                    // only the grant would leave a reader to assume whether
                    // the caller had to hold a key to select it [ORB-11053].
                    "caller_identity": grant.identity,
                })
                .to_string()
            }),
            stdout_truncated: None,
            stderr_truncated: None,
            error_message,
            host: std::env::var("HOSTNAME").ok(),
            pid: std::process::id(),
            session_id: None,
            workspace_id: None,
            caller_machine_id: caller
                .remote_caller_grant()
                .map(|grant| grant.caller_machine_id.clone()),
            caller_host_id: None,
            process_machine_id: None,
            process_host_id: None,
            transport: None,
            effective_capabilities: caller.grants().clone(),
            origin_session_id: None,
            mcp_call_id: None,
            lease_id: None,
            task_id: None,
            job_run_id: None,
            activity_id: None,
            step_index: None,
        };

        if let Err(error) = self.record_audit_event(&params) {
            tracing::error!(
                target: "orbit.authorization",
                operation = operation.id,
                "failed to persist authorization audit event: {error}"
            );
        }
    }
}
