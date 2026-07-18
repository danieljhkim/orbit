//! Checkoutless, non-recursive MCP server host for the coordination hub [ORB-10268].

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use orbit_common::types::{
    AuditEventStatus, McpCapability, McpToolDefinition, McpToolPlacement, McpToolScope,
    McpTransport, RegistrySnapshotV1, ToolSessionContext, WorkspaceStatus,
    mcp_advertised_tool_name,
};
use orbit_core::routines::{HostIdentity, HostMode, load_host_identity};
use orbit_core::runtime::HubCoordinationExecutor;
use orbit_core::{NotFoundKind, OrbitError, redact_sensitive_env_text};
use orbit_mcp::McpHost;
use serde_json::{Value, json};

use super::host::{
    canonical_mcp_tool_definitions, mcp_preflight_failure_params, normalize_trusted_call_context,
};

/// A fixed hub-only host. It owns no broker, runtime cache, graph adapter,
/// connector, owner resolver, or transport factory.
#[derive(Debug)]
pub(super) struct HubMcpHost {
    global_root: PathBuf,
    identity: HostIdentity,
    capability: McpCapability,
}

impl HubMcpHost {
    pub(super) fn new(global_root: PathBuf, capability: McpCapability) -> Result<Self, OrbitError> {
        let identity = load_host_identity(&global_root)?;
        if identity.mode != HostMode::Hub {
            return Err(OrbitError::InvalidInput(format!(
                "orbit mcp serve --hub requires host.toml mode 'hub'; machine '{}' ({}) is '{}'",
                identity.host_id, identity.machine_id, identity.mode
            )));
        }
        let host = Self {
            global_root,
            identity,
            capability,
        };
        // Fail before stdio is opened. Listing and every call repeat this
        // check so a long-lived server cannot outlive an authority change.
        host.verify_authority()?;
        Ok(host)
    }

    pub(super) fn identity(&self) -> &HostIdentity {
        &self.identity
    }

    fn verify_authority(&self) -> Result<(HostIdentity, RegistrySnapshotV1), OrbitError> {
        let identity = load_host_identity(&self.global_root)?;
        if identity.mode != HostMode::Hub {
            return Err(OrbitError::InvalidInput(format!(
                "hub MCP authority changed: machine '{}' ({}) is now mode '{}'",
                identity.host_id, identity.machine_id, identity.mode
            )));
        }
        if identity.machine_id != self.identity.machine_id {
            return Err(OrbitError::InvalidInput(format!(
                "hub MCP authority changed: startup machine_id '{}' no longer matches host.toml machine_id '{}'",
                self.identity.machine_id, identity.machine_id
            )));
        }
        let snapshot = orbit_core::host_registry::registry_snapshot_at(&self.global_root)?;
        match snapshot.hub_machine_id.as_deref() {
            Some(configured) if configured == identity.machine_id => Ok((identity, snapshot)),
            Some(configured) => Err(OrbitError::InvalidInput(format!(
                "refusing hub MCP through a shadow coordination store: local hub machine_id '{}' does not match configured hub_machine_id '{configured}'",
                identity.machine_id
            ))),
            None => Err(OrbitError::InvalidInput(
                "the global coordination store has no configured hub_machine_id; register this hub before starting `orbit mcp serve --hub`"
                    .to_string(),
            )),
        }
    }

    fn admitted(&self, definition: &McpToolDefinition) -> bool {
        definition.policy.placement() == McpToolPlacement::Hub
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

    fn require_authenticated_caller(context: &ToolSessionContext) -> Result<(), OrbitError> {
        if context.transport == Some(McpTransport::SshMcp)
            && (context.caller_machine_id.is_none() || context.caller_host_id.is_none())
        {
            return Err(OrbitError::InvalidInput(
                "SSH-carried hub MCP calls require authenticated caller machine_id and host_id"
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

    fn resolve_workspace(&self, selector: &str) -> Result<String, OrbitError> {
        if Path::new(selector).is_absolute()
            || selector.contains('/')
            || selector == "."
            || selector == ".."
        {
            return Err(OrbitError::InvalidInput(format!(
                "hub MCP workspace selector '{selector}' must be a stable logical workspace ID, never a checkout path"
            )));
        }
        let registry_path = orbit_core::workspace_registry::registry_path_for(&self.global_root);
        let registry = orbit_core::workspace_registry::load_registry_from(&registry_path)?;
        let workspace = registry
            .workspaces
            .into_iter()
            .find(|workspace| workspace.id == selector)
            .ok_or_else(|| {
                OrbitError::InvalidInput(format!(
                    "unknown logical workspace ID '{selector}' on this hub"
                ))
            })?;
        if workspace.status != WorkspaceStatus::Active {
            return Err(OrbitError::InvalidInput(format!(
                "logical workspace ID '{}' is not active",
                workspace.id
            )));
        }
        Ok(workspace.id)
    }

    fn global_call(name: &str, snapshot: RegistrySnapshotV1) -> Result<Value, OrbitError> {
        match name {
            "orbit.host.list" => Ok(json!({
                "hub_machine_id": snapshot.hub_machine_id,
                "registry_revision": snapshot.registry_revision,
                "hosts": snapshot.hosts,
            })),
            "orbit.workspace.list" => Ok(json!({
                "hub_machine_id": snapshot.hub_machine_id,
                "registry_revision": snapshot.registry_revision,
                "workspaces": snapshot.workspaces,
            })),
            _ => Err(OrbitError::not_found(NotFoundKind::Tool, name.to_string())),
        }
    }

    fn record_denial(&self, name: &str, context: &ToolSessionContext, denial: &OrbitError) {
        let params = mcp_preflight_failure_params(name, context, denial);
        if let Err(error) =
            orbit_core::host_registry::record_global_audit_event_at(&self.global_root, &params)
        {
            tracing::warn!(tool = name, error = %error, "failed to persist hub MCP denial audit");
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
        if let Err(error) =
            orbit_core::host_registry::record_global_audit_event_at(&self.global_root, &params)
        {
            tracing::warn!(tool = name, error = %error, "failed to persist hub MCP outcome audit");
        }
    }

    fn resolved_call(
        &self,
        inbound: &str,
        mut input: Value,
        context: ToolSessionContext,
    ) -> Result<Value, OrbitError> {
        let (identity, snapshot) = match self.verify_authority() {
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
                "hub MCP denied tool '{name}': placement is '{}' and allowed capabilities are [{}], while this fixed session is '{}'",
                placement_label(definition.policy.placement()),
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
                "hub MCP cannot add local-checkout context to orbit.task.show".to_string(),
            );
            self.record_denial(name, &context, &error);
            return Err(error);
        }
        if definition.policy.scope() == McpToolScope::Global {
            let result = Self::global_call(name, snapshot);
            self.record_outcome(name, &context, &result);
            return result;
        }

        let selector = match Self::selector(&input, &context) {
            Some(selector) => selector,
            None => {
                let error = OrbitError::InvalidInput(format!(
                    "hub tool '{name}' requires a stable logical workspace ID"
                ));
                self.record_denial(name, &context, &error);
                return Err(error);
            }
        };
        let workspace_id = match self.resolve_workspace(selector) {
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
        let result = HubCoordinationExecutor::new(&self.global_root, workspace_id, None)
            .and_then(|executor| executor.execute_tool(name, input, context.clone()));
        self.record_outcome(name, &context, &result);
        result
    }
}

impl McpHost for HubMcpHost {
    fn list_mcp_tool_definitions(&self) -> Result<Vec<McpToolDefinition>, OrbitError> {
        self.verify_authority()?;
        Ok(canonical_mcp_tool_definitions()
            .map_err(|error| OrbitError::InvalidInput(error.to_string()))?
            .into_iter()
            .filter(|definition| self.admitted(definition))
            .collect())
    }

    fn in_process_graph_tools_enabled(&self) -> bool {
        false
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
            .map(|(identity, _)| identity)
            .unwrap_or_else(|_| self.identity.clone());
        let context = self.normalize_context(session_context.clone(), &identity);
        let denial = Self::require_authenticated_caller(&context)
            .err()
            .unwrap_or(denial);
        self.record_denial(name, &context, &denial);
        denial
    }

    fn call_in_process_tool(
        &self,
        name: &str,
        input: Value,
        session_context: ToolSessionContext,
        _dispatch: &mut dyn FnMut(Value, ToolSessionContext) -> Result<Value, OrbitError>,
    ) -> Result<Value, OrbitError> {
        // Known graph names remain recognizable to the adapter even though
        // hub mode deliberately omits their implementations and schemas. Send
        // them through the same canonical placement denial + audit boundary;
        // never invoke the adapter's local-checkout closure.
        self.resolved_call(name, input, session_context)
    }
}

fn placement_label(placement: McpToolPlacement) -> &'static str {
    match placement {
        McpToolPlacement::Hub => "hub",
        McpToolPlacement::Owner => "owner",
        McpToolPlacement::LocalDerived => "local-derived",
        McpToolPlacement::Composite => "composite",
    }
}
