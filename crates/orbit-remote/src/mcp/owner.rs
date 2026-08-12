//! Checkoutless, non-recursive MCP server host for a workspace owner
//! [ORB-10268, ORB-10727].
//!
//! ORB-10268 built this endpoint under the `--hub` spelling, where one
//! machine-level hub owned coordination for every workspace. ADR-0355 collapses
//! that into ownership: every machine is its own coordination host for the
//! workspaces it owns, so the endpoint is now started by the same plain
//! `orbit mcp serve` a local broker uses and there is no machine-level mode to
//! require.
//!
//! The endpoint re-reads local `host.toml` before listing and every call; it filters the
//! canonical registry by exactly one placement class and one scalar capability;
//! it accepts only stable logical workspace IDs, never caller paths; it invokes
//! the checkout-independent coordination executor without constructing
//! `OrbitRuntime` or opening any connector; and it never opens another MCP/SSH
//! connection.
//!
//! What is new: a workspace this machine does not own is refused by name, so a
//! client cannot reach through one owner to another (§4.2 rule 5).

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use orbit_common::types::{
    AuditEventStatus, McpCapability, McpToolDefinition, McpToolPlacement, McpToolScope,
    McpTransport, ToolSessionContext, WorkspaceStatus, mcp_advertised_tool_name,
};
use orbit_core::runtime::HubCoordinationExecutor;
use orbit_core::{NotFoundKind, OrbitError, redact_sensitive_env_text};
use orbit_mcp::McpHost;
use serde_json::Value;

use crate::{HostIdentity, load_host_identity};

use super::contract::{
    CANONICAL_MCP_REGISTRY_REVISION, MCP_CONTRACT_REVISION, OwnerServerContractV1,
    owner_schema_digest,
};
use super::host::{
    canonical_mcp_tool_definitions, mcp_preflight_failure_params, normalize_trusted_call_context,
};

/// A fixed owner-endpoint host. It owns no broker, runtime cache, connector,
/// owner resolver, or transport factory.
#[derive(Debug)]
pub(super) struct OwnerMcpHost {
    global_root: PathBuf,
    identity: HostIdentity,
    capability: McpCapability,
    contract_instructions: String,
}

impl OwnerMcpHost {
    pub(super) fn new(global_root: PathBuf, capability: McpCapability) -> Result<Self, OrbitError> {
        let identity = load_host_identity(&global_root)?;
        crate::runtime::sync_task_prefix(&global_root)?;
        let definitions = canonical_mcp_tool_definitions()
            .map_err(|error| OrbitError::InvalidInput(error.to_string()))?;
        let contract = OwnerServerContractV1 {
            contract_revision: MCP_CONTRACT_REVISION,
            canonical_registry_revision: CANONICAL_MCP_REGISTRY_REVISION,
            owner_machine_id: identity.machine_id.clone(),
            effective_capability: capability,
            owner_schema_digest: owner_schema_digest(&definitions, capability)?,
        };
        let contract_instructions = contract.instructions()?;
        let host = Self {
            global_root,
            identity,
            capability,
            contract_instructions,
        };
        // Fail before stdio is opened. Listing and every call repeat this
        // check so a long-lived server cannot outlive an authority change.
        host.verify_authority()?;
        Ok(host)
    }

    pub(super) fn identity(&self) -> &HostIdentity {
        &self.identity
    }

    pub(super) fn contract_instructions(&self) -> &str {
        &self.contract_instructions
    }

    /// Confirm `host.toml` still names the machine this server started with.
    /// Fleet registry stamps are dormant in v1 and are never consulted.
    fn verify_authority(&self) -> Result<HostIdentity, OrbitError> {
        let identity = load_host_identity(&self.global_root)?;
        if identity.machine_id != self.identity.machine_id {
            return Err(OrbitError::InvalidInput(format!(
                "owner MCP authority changed: startup machine_id '{}' no longer matches host.toml machine_id '{}'",
                self.identity.machine_id, identity.machine_id
            )));
        }
        Ok(identity)
    }

    fn admitted(&self, definition: &McpToolDefinition) -> bool {
        definition.policy.placement() == McpToolPlacement::Owner
            && definition
                .policy
                .allowed_capabilities()
                .contains(&self.capability)
    }

    fn definition(&self, inbound: &str) -> Result<McpToolDefinition, OrbitError> {
        canonical_mcp_tool_definitions()
            .map_err(|error| OrbitError::InvalidInput(error.to_string()))?
            .into_iter()
            .find(|definition| {
                definition.schema.name == inbound
                    || mcp_advertised_tool_name(&definition.schema.name) == inbound
            })
            .ok_or_else(|| OrbitError::not_found(NotFoundKind::Tool, inbound.to_string()))
    }

    fn normalize_context(
        &self,
        mut context: ToolSessionContext,
        identity: &HostIdentity,
    ) -> ToolSessionContext {
        context = normalize_trusted_call_context(context);
        context.process_machine_id = Some(identity.machine_id.clone());
        context.process_host_id = Some(identity.host_id.clone());
        if context.transport == Some(McpTransport::Local) {
            if context.caller_machine_id.is_none() {
                context.caller_machine_id = Some(identity.machine_id.clone());
            }
            if context.caller_host_id.is_none() {
                context.caller_host_id = Some(identity.host_id.clone());
            }
        }
        context.effective_capabilities = BTreeSet::from([self.capability]);
        context
    }

    /// A remote call must carry the connector-owned caller identity.
    ///
    /// ORB-10727 [ADR-0358]: this used to additionally require the caller to be
    /// an actively registered spoke in the fleet inventory. v1 has no
    /// registration step — a client opens a route and calls — so SSH is the
    /// authenticator and the identity is required only to be complete.
    fn require_authenticated_caller(context: &ToolSessionContext) -> Result<(), OrbitError> {
        if context.transport == Some(McpTransport::SshMcp)
            && (context.caller_machine_id.is_none() || context.caller_host_id.is_none())
        {
            return Err(OrbitError::InvalidInput(
                "SSH-carried owner MCP calls require authenticated caller machine_id and host_id"
                    .to_string(),
            ));
        }
        Ok(())
    }

    fn selector<'a>(input: &'a Value, context: &'a ToolSessionContext) -> Option<&'a str> {
        input
            .get("workspace")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .or_else(|| {
                context
                    .workspace
                    .as_deref()
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
            })
            .or_else(|| {
                context
                    .workspace_id
                    .as_deref()
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
            })
    }

    /// Resolve a stable logical workspace ID that this machine actually owns.
    ///
    /// Refusing a non-owned workspace by name is what keeps the topology a
    /// star per workspace: a client that reached the wrong owner is told which
    /// machine to open a route to rather than being silently relayed there.
    fn resolve_owned_workspace(&self, selector: &str) -> Result<String, OrbitError> {
        if Path::new(selector).is_absolute()
            || selector.contains('/')
            || selector == "."
            || selector == ".."
        {
            return Err(OrbitError::InvalidInput(format!(
                "owner MCP workspace selector '{selector}' must be a stable logical workspace ID, never a checkout path"
            )));
        }
        let registry_path = crate::workspace_registry::registry_path_for(&self.global_root);
        let registry = crate::workspace_registry::load_registry_from(&registry_path)?;
        let workspace = registry
            .workspaces
            .into_iter()
            .find(|workspace| workspace.id == selector)
            .ok_or_else(|| {
                OrbitError::InvalidInput(format!(
                    "unknown logical workspace ID '{selector}' on owner machine '{}'",
                    self.identity.machine_id
                ))
            })?;
        if workspace.status != WorkspaceStatus::Active {
            return Err(OrbitError::InvalidInput(format!(
                "logical workspace ID '{}' is not active",
                workspace.id
            )));
        }
        match workspace.owner_machine_id.as_deref() {
            Some(owner) if owner == self.identity.machine_id => Ok(workspace.id),
            Some(owner) => Err(OrbitError::InvalidInput(format!(
                "workspace '{}' is owned by machine '{owner}', not this endpoint's machine '{}'; open a route to the owner instead",
                workspace.id, self.identity.machine_id
            ))),
            None => Err(OrbitError::InvalidInput(format!(
                "workspace '{}' declares no owner machine; the owner endpoint serves only owned workspaces",
                workspace.id
            ))),
        }
    }

    fn record_denial(&self, name: &str, context: &ToolSessionContext, denial: &OrbitError) {
        let params = mcp_preflight_failure_params(name, context, denial);
        if let Err(error) = crate::record_global_audit_event_at(&self.global_root, &params) {
            tracing::warn!(tool = name, error = %error, "failed to persist owner MCP denial audit");
        }
    }

    fn record_outcome(
        &self,
        name: &str,
        context: &ToolSessionContext,
        result: &Result<Value, OrbitError>,
    ) {
        let placeholder = OrbitError::Execution(String::new());
        let error = result.as_ref().err().unwrap_or(&placeholder);
        let mut params = mcp_preflight_failure_params(name, context, error);
        match result {
            Ok(_) => {
                params.status = AuditEventStatus::Success;
                params.exit_code = 0;
                params.error_message = None;
            }
            Err(error) => {
                params.status = AuditEventStatus::Failure;
                params.error_message = Some(redact_sensitive_env_text(&error.to_string()));
            }
        }
        if let Err(error) = crate::record_global_audit_event_at(&self.global_root, &params) {
            tracing::warn!(tool = name, error = %error, "failed to persist owner MCP outcome audit");
        }
    }

    fn resolved_call(
        &self,
        inbound: &str,
        mut input: Value,
        context: ToolSessionContext,
    ) -> Result<Value, OrbitError> {
        let identity = match self.verify_authority() {
            Ok(authority) => authority,
            Err(error) => {
                let context = self.normalize_context(context, &self.identity);
                self.record_denial(inbound, &context, &error);
                return Err(error);
            }
        };
        let mut context = self.normalize_context(context, &identity);
        if let Err(error) = Self::require_authenticated_caller(&context) {
            self.record_denial(inbound, &context, &error);
            return Err(error);
        }
        let definition = match self.definition(inbound) {
            Ok(definition) => definition,
            Err(error) => {
                self.record_denial(inbound, &context, &error);
                return Err(error);
            }
        };
        let name = definition.schema.name.as_str();
        if !self.admitted(&definition) {
            let error = OrbitError::InvalidInput(format!(
                "owner MCP denied tool '{name}': placement is '{}' and allowed capabilities are [{}], while this fixed session is '{}'",
                definition.policy.placement(),
                definition
                    .policy
                    .allowed_capabilities()
                    .iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
                    .join(", "),
                self.capability
            ));
            self.record_denial(name, &context, &error);
            return Err(error);
        }
        if name == "orbit.task.show"
            && input
                .get("with_context")
                .or_else(|| input.get("withContext"))
                .or_else(|| input.get("with-context"))
                .and_then(Value::as_bool)
                .unwrap_or(false)
        {
            let error = OrbitError::InvalidInput(
                "owner MCP cannot add local-checkout context to orbit.task.show".to_string(),
            );
            self.record_denial(name, &context, &error);
            return Err(error);
        }
        if definition.policy.scope() == McpToolScope::Global {
            let result = Err(OrbitError::InvalidInput(format!(
                "owner endpoint does not serve local-derived global tool '{name}'"
            )));
            self.record_outcome(name, &context, &result);
            return result;
        }

        let selector = match Self::selector(&input, &context) {
            Some(selector) => selector,
            None => {
                let error = OrbitError::InvalidInput(format!(
                    "owner tool '{name}' requires a stable logical workspace ID"
                ));
                self.record_denial(name, &context, &error);
                return Err(error);
            }
        };
        let workspace_id = match self.resolve_owned_workspace(selector) {
            Ok(workspace_id) => workspace_id,
            Err(error) => {
                self.record_denial(name, &context, &error);
                return Err(error);
            }
        };
        context.workspace_id = Some(workspace_id.clone());
        context.workspace = Some(workspace_id.clone());
        if let Some(object) = input.as_object_mut()
            && object.contains_key("workspace")
        {
            object.insert("workspace".to_string(), Value::String(workspace_id.clone()));
        }
        // Crew validation runs where the workspace is owned, so it reads this
        // machine's own execution-profile state directly (§8.1). It never opens
        // the coordination task registry.
        if name == "orbit.crew.list" {
            let result = self.crew_discovery(&workspace_id);
            self.record_outcome(name, &context, &result);
            return result;
        }
        // Explicit non-empty execution and orchestration crew aliases are
        // validated before any allocation/mutation. Omitted or cleared values
        // remain accepted without a profile.
        for field in ["crew", "orchestrator"] {
            let Some(crew) = explicit_task_crew(name, &input, field) else {
                continue;
            };
            let canonical = match self.validate_task_crew(&workspace_id, &crew) {
                Ok(canonical) => canonical,
                Err(error) => {
                    self.record_denial(name, &context, &error);
                    return Err(error);
                }
            };
            if let Some(object) = input.as_object_mut() {
                object.insert(field.to_string(), Value::String(canonical));
            }
        }
        let result = HubCoordinationExecutor::new(&self.global_root, workspace_id, None)
            .and_then(|executor| executor.execute_tool(name, input, context.clone()));
        self.record_outcome(name, &context, &result);
        result
    }

    /// Project the sanitized crew-discovery response for `orbit.crew.list` from
    /// this owner machine's local execution-profile state.
    fn crew_discovery(&self, workspace_id: &str) -> Result<Value, OrbitError> {
        let discovery = crate::ExecutionProfileProjection::at(&self.global_root)?
            .crew_discovery(workspace_id)?;
        serde_json::to_value(discovery)
            .map_err(|error| OrbitError::Execution(format!("serialize crew discovery: {error}")))
    }

    /// Validate an explicit task crew against this owner machine's current
    /// execution profile. Never falls back to the registry cache, a stale
    /// replica, or a synchronous call to another machine.
    fn validate_task_crew(&self, workspace_id: &str, crew: &str) -> Result<String, OrbitError> {
        crate::ExecutionProfileProjection::at(&self.global_root)?
            .validate_task_crew(workspace_id, crew)
            .map(|validated| validated.resolved_crew.name)
    }
}

impl McpHost for OwnerMcpHost {
    fn list_mcp_tool_definitions(&self) -> Result<Vec<McpToolDefinition>, OrbitError> {
        self.verify_authority()?;
        Ok(canonical_mcp_tool_definitions()
            .map_err(|error| OrbitError::InvalidInput(error.to_string()))?
            .into_iter()
            .filter(|definition| self.admitted(definition))
            .collect())
    }

    fn preflight_tool_call(
        &self,
        _name: &str,
        session_context: &ToolSessionContext,
    ) -> Result<(), OrbitError> {
        let identity = self.verify_authority()?;
        let context = self.normalize_context(session_context.clone(), &identity);
        Self::require_authenticated_caller(&context)
    }

    fn call_tool(
        &self,
        name: &str,
        input: Value,
        session_context: ToolSessionContext,
    ) -> Result<Value, OrbitError> {
        self.resolved_call(name, input, session_context)
    }

    fn reject_tool_call(
        &self,
        name: &str,
        _input: &Value,
        session_context: &ToolSessionContext,
        denial: OrbitError,
    ) -> OrbitError {
        let identity = self
            .verify_authority()
            .unwrap_or_else(|_| self.identity.clone());
        let context = self.normalize_context(session_context.clone(), &identity);
        let denial = Self::require_authenticated_caller(&context)
            .err()
            .unwrap_or(denial);
        self.record_denial(name, &context, &denial);
        denial
    }
}

impl super::learning::LearningSidecarHost for OwnerMcpHost {}

/// The explicit, non-empty named-crew field on a task add/update, or `None`
/// for any other tool, an omitted field, or an explicit clearing value.
fn explicit_task_crew(name: &str, input: &Value, field: &str) -> Option<String> {
    if !matches!(name, "orbit.task.add" | "orbit.task.update") {
        return None;
    }
    input
        .get(field)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}
