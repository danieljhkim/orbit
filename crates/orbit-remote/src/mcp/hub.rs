//! Checkoutless, non-recursive MCP server host for the coordination hub [ORB-10268].

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use orbit_common::types::{
    AuditEventStatus, HUB_KNOWLEDGE_ALLOCATION_METHOD_V1, HostRecord, HostStatus,
    HubKnowledgeAllocationRequestV1, HubKnowledgeAllocationV1, McpCapability, McpToolDefinition,
    McpToolPlacement, McpToolScope, McpTransport, RegistrySnapshotV1, SPOKE_REGISTRATION_METHOD_V1,
    SpokeRegistrationRequestV1, SpokeRegistrationResultV1, SpokeRegistrationStageV1,
    ToolSessionContext, WorkspaceStatus, mcp_advertised_tool_name,
};
use orbit_core::runtime::HubCoordinationExecutor;
use orbit_core::{NotFoundKind, OrbitError, redact_sensitive_env_text};
use orbit_mcp::McpHost;
use serde_json::{Value, json};

use crate::{HOST_IDENTITY_SCHEMA_VERSION, HostIdentity, HostMode, load_host_identity};

use super::contract::{
    CANONICAL_MCP_REGISTRY_REVISION, HubServerContractV1, MCP_CONTRACT_REVISION, hub_schema_digest,
};
use super::host::{
    BrokerMcpHost, canonical_mcp_tool_definitions, is_current_knowledge_tool,
    mcp_preflight_failure_params, normalize_trusted_call_context, preallocated_knowledge_kind,
};

/// A fixed hub-only host. Its owner broker has no connector, so composite
/// knowledge calls can terminate only in a validated hub-owned checkout.
pub(super) struct HubMcpHost {
    global_root: PathBuf,
    identity: HostIdentity,
    capability: McpCapability,
    private_instructions: String,
    knowledge_broker: BrokerMcpHost,
}

impl std::fmt::Debug for HubMcpHost {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("HubMcpHost")
            .field("global_root", &self.global_root)
            .field("identity", &self.identity)
            .field("capability", &self.capability)
            .finish_non_exhaustive()
    }
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
        let definitions = canonical_mcp_tool_definitions()
            .map_err(|error| OrbitError::InvalidInput(error.to_string()))?;
        let contract = HubServerContractV1 {
            contract_revision: MCP_CONTRACT_REVISION,
            canonical_registry_revision: CANONICAL_MCP_REGISTRY_REVISION,
            hub_machine_id: identity.machine_id.clone(),
            effective_capability: capability,
            hub_schema_digest: hub_schema_digest(&definitions, capability)?,
        };
        let private_instructions = contract.instructions()?;
        let knowledge_broker = BrokerMcpHost::new(global_root.clone());
        let host = Self {
            global_root,
            identity,
            capability,
            private_instructions,
            knowledge_broker,
        };
        // Fail before stdio is opened. Listing and every call repeat this
        // check so a long-lived server cannot outlive an authority change.
        host.verify_authority()?;
        Ok(host)
    }

    pub(super) fn identity(&self) -> &HostIdentity {
        &self.identity
    }

    pub(super) fn private_instructions(&self) -> &str {
        &self.private_instructions
    }

    pub(super) fn private_register_spoke(
        &self,
        request: SpokeRegistrationRequestV1,
        session_context: ToolSessionContext,
    ) -> Result<SpokeRegistrationResultV1, OrbitError> {
        let context = self.normalize_context(session_context, &self.identity);
        let result = self.registration_result(request, context.clone());
        let audit_result = if result.complete {
            Ok(json!({
                "complete": true,
                "last_committed_stage": result.last_committed_stage,
            }))
        } else {
            Err(OrbitError::Execution(
                result
                    .failure
                    .as_ref()
                    .map(|failure| failure.message.clone())
                    .unwrap_or_else(|| "private spoke registration failed".to_string()),
            ))
        };
        self.record_outcome(SPOKE_REGISTRATION_METHOD_V1, &context, &audit_result);
        Ok(result)
    }

    pub(super) fn private_allocate_knowledge_id(
        &self,
        request: HubKnowledgeAllocationRequestV1,
        session_context: ToolSessionContext,
    ) -> Result<HubKnowledgeAllocationV1, OrbitError> {
        let (identity, snapshot) = match self.verify_authority() {
            Ok(authority) => authority,
            Err(error) => {
                let context = self.normalize_context(session_context, &self.identity);
                self.record_denial(HUB_KNOWLEDGE_ALLOCATION_METHOD_V1, &context, &error);
                return Err(error);
            }
        };
        let context = self.normalize_context(session_context, &identity);
        let result = (|| {
            Self::require_active_remote_caller(&context, &snapshot)?;
            if !matches!(
                self.capability,
                McpCapability::Agent | McpCapability::Operator
            ) {
                return Err(OrbitError::InvalidInput(
                    "private hub knowledge allocation requires agent or operator capability"
                        .to_string(),
                ));
            }
            request.validate()?;
            let workspace_id = self.resolve_workspace(&request.workspace_id)?;
            if context.workspace_id.as_deref() != Some(workspace_id.as_str())
                || context
                    .workspace
                    .as_deref()
                    .is_some_and(|selector| selector != workspace_id)
            {
                return Err(OrbitError::InvalidInput(
                    "private hub knowledge allocation workspace must exactly match the trusted session context"
                        .to_string(),
                ));
            }
            let allocator = crate::HubKnowledgeSequenceService::at(&self.global_root)?;
            allocator.ensure_public_cutover_active()?;
            allocator.allocate(&request, &context)
        })();
        if let Err(error) = &result {
            self.record_denial(HUB_KNOWLEDGE_ALLOCATION_METHOD_V1, &context, error);
        }
        // Successful allocation writes its canonical audit row inside the
        // same SQLite transaction as sequence, occupancy, and ledger state.
        // Deliberately do not call `record_outcome` here.
        result
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
        let snapshot = crate::registry_snapshot_at(&self.global_root)?;
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
        (definition.policy.placement() == McpToolPlacement::Hub
            || (definition.policy.placement() == McpToolPlacement::Composite
                && preallocated_knowledge_kind(&definition.schema.name).is_some())
            || (definition.policy.placement() == McpToolPlacement::Owner
                && is_current_knowledge_tool(&definition.schema.name)))
            && definition
                .policy
                .allowed_capabilities()
                .contains(&self.capability)
    }

    pub(super) fn compose_preallocated_knowledge_add(
        &self,
        name: &str,
        input: Value,
        session_context: ToolSessionContext,
    ) -> Result<Value, OrbitError> {
        let (identity, snapshot) = self.verify_authority()?;
        let context = self.normalize_context(session_context, &identity);
        Self::require_active_remote_caller(&context, &snapshot)?;
        if preallocated_knowledge_kind(name).is_none() {
            return Err(OrbitError::InvalidInput(format!(
                "tool '{name}' is not a preallocated knowledge add"
            )));
        }
        self.knowledge_broker
            .preallocated_knowledge_call(name, input, context)
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

    fn require_active_remote_caller(
        context: &ToolSessionContext,
        snapshot: &RegistrySnapshotV1,
    ) -> Result<(), OrbitError> {
        Self::require_authenticated_caller(context)?;
        if context.transport != Some(McpTransport::SshMcp) {
            return Ok(());
        }
        let (Some(machine_id), Some(host_id)) = (
            context.caller_machine_id.as_deref(),
            context.caller_host_id.as_deref(),
        ) else {
            return Err(OrbitError::InvalidInput(
                "SSH-carried hub MCP calls require authenticated caller machine_id and host_id"
                    .to_string(),
            ));
        };
        let host = snapshot
            .hosts
            .iter()
            .find(|host| host.machine_id == machine_id)
            .ok_or_else(|| {
                OrbitError::InvalidInput(format!(
                    "remote caller machine_id '{machine_id}' is not registered; only the private spoke registration request is allowed"
                ))
            })?;
        if host.status != HostStatus::Active {
            return Err(OrbitError::InvalidInput(format!(
                "remote caller machine_id '{machine_id}' is retired"
            )));
        }
        if host.host_id != host_id {
            return Err(OrbitError::InvalidInput(format!(
                "remote caller host_id '{host_id}' does not match registered host_id '{}' for machine_id '{machine_id}'",
                host.host_id
            )));
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
        let registry_path = crate::workspace_registry::registry_path_for(&self.global_root);
        let registry = crate::workspace_registry::load_registry_from(&registry_path)?;
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
        super::discovery::execute_discovery_tool(name, snapshot)
    }

    fn record_denial(&self, name: &str, context: &ToolSessionContext, denial: &OrbitError) {
        let params = mcp_preflight_failure_params(name, context, denial);
        if let Err(error) = crate::record_global_audit_event_at(&self.global_root, &params) {
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
        if let Err(error) = crate::record_global_audit_event_at(&self.global_root, &params) {
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
        if let Err(error) = Self::require_active_remote_caller(&context, &snapshot) {
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
        // Crew discovery reads the stored owner execution-profile projection at
        // the hub; it never opens the coordination task registry.
        if name == "orbit.crew.list" {
            let result = self.crew_discovery(&workspace_id);
            self.record_outcome(name, &context, &result);
            return result;
        }
        // Explicit non-empty task-crew assignment is validated against the
        // owner profile before any allocation/mutation. An omitted or cleared
        // crew is accepted without a profile, so coordination tasks can still be
        // filed while an owner is unavailable.
        if let Some(crew) = explicit_task_crew(name, &input)
            && let Err(error) = self.validate_task_crew(&workspace_id, &crew)
        {
            self.record_denial(name, &context, &error);
            return Err(error);
        }
        if preallocated_knowledge_kind(name).is_some() || is_current_knowledge_tool(name) {
            let result = if preallocated_knowledge_kind(name).is_some() {
                self.compose_preallocated_knowledge_add(name, input, context.clone())
            } else {
                self.knowledge_broker
                    .call_tool(name, input, context.clone())
            };
            if let Err(error) = &result {
                self.record_denial(name, &context, error);
            }
            return result;
        }
        let result = HubCoordinationExecutor::new(&self.global_root, workspace_id, None)
            .and_then(|executor| executor.execute_tool(name, input, context.clone()));
        self.record_outcome(name, &context, &result);
        result
    }

    /// Project the sanitized crew-discovery response for `orbit.crew.list` from
    /// C2's stored owner execution-profile projection.
    fn crew_discovery(&self, workspace_id: &str) -> Result<Value, OrbitError> {
        let discovery = crate::ExecutionProfileProjection::at(&self.global_root)?
            .crew_discovery(workspace_id)?;
        serde_json::to_value(discovery)
            .map_err(|error| OrbitError::Execution(format!("serialize crew discovery: {error}")))
    }

    /// Validate an explicit task crew against the workspace owner's current
    /// execution profile. Never falls back to hub-local crews, the registry
    /// cache, a stale replica, or a synchronous owner call.
    fn validate_task_crew(&self, workspace_id: &str, crew: &str) -> Result<(), OrbitError> {
        crate::ExecutionProfileProjection::at(&self.global_root)?
            .validate_task_crew(workspace_id, crew)
            .map(|_| ())
    }

    fn registration_result(
        &self,
        request: SpokeRegistrationRequestV1,
        context: ToolSessionContext,
    ) -> SpokeRegistrationResultV1 {
        let (hub_identity, _) = match self.verify_authority() {
            Ok(authority) => authority,
            Err(error) => return SpokeRegistrationResultV1::rejected(&error),
        };
        let context = self.normalize_context(context, &hub_identity);
        if let Err(error) = Self::require_authenticated_caller(&context) {
            return SpokeRegistrationResultV1::rejected(&error);
        }
        if context.transport != Some(McpTransport::SshMcp) {
            return SpokeRegistrationResultV1::rejected(&OrbitError::InvalidInput(
                "private spoke registration requires ssh-mcp transport".to_string(),
            ));
        }
        if context.workspace.is_some() || context.workspace_id.is_some() {
            return SpokeRegistrationResultV1::rejected(&OrbitError::InvalidInput(
                "private spoke registration is global and must not carry a workspace selector"
                    .to_string(),
            ));
        }
        if let Err(error) = request.validate() {
            return SpokeRegistrationResultV1::rejected(&error);
        }
        let caller_machine_id = context.caller_machine_id.as_deref().unwrap_or_default();
        let caller_host_id = context.caller_host_id.as_deref().unwrap_or_default();
        if request.identity.machine_id != caller_machine_id
            || request.identity.host_id != caller_host_id
        {
            return SpokeRegistrationResultV1::rejected(&OrbitError::InvalidInput(format!(
                "private registration identity '{}/{}' does not match trusted session caller '{}/{}'",
                request.identity.machine_id,
                request.identity.host_id,
                caller_machine_id,
                caller_host_id
            )));
        }

        let service = match crate::host_registry_service_at(&self.global_root) {
            Ok(service) => service,
            Err(error) => return SpokeRegistrationResultV1::rejected(&error),
        };
        let spoke_identity = HostIdentity {
            schema_version: HOST_IDENTITY_SCHEMA_VERSION,
            machine_id: request.identity.machine_id.clone(),
            host_id: request.identity.host_id.clone(),
            mode: HostMode::Spoke,
        };
        let host = match service.register_identity(&spoke_identity, request.identity.labels) {
            Ok(host) => host,
            Err(error) => return SpokeRegistrationResultV1::rejected(&error),
        };

        let registry_path = crate::workspace_registry::registry_path_for(&self.global_root);
        let registry = match crate::workspace_registry::load_registry_from(&registry_path) {
            Ok(registry) => registry,
            Err(error) => {
                return registration_partial(
                    SpokeRegistrationStageV1::Registry,
                    host,
                    Vec::new(),
                    Vec::new(),
                    error,
                );
            }
        };
        let presence = match service.publish_presence(
            &registry,
            &spoke_identity.machine_id,
            &request.presence,
        ) {
            Ok(presence) => presence,
            Err(error) => {
                return registration_partial(
                    SpokeRegistrationStageV1::Registry,
                    host,
                    Vec::new(),
                    Vec::new(),
                    error,
                );
            }
        };
        let presence_workspace_ids = presence
            .into_iter()
            .map(|presence| presence.workspace_id)
            .collect::<Vec<_>>();

        let mut profile_workspace_ids = Vec::new();
        for publication in request.profiles {
            match service.publish_execution_profile(
                &spoke_identity.machine_id,
                publication.expected_generation,
                &publication.profile,
            ) {
                Ok(_) => profile_workspace_ids.push(publication.profile.workspace_id),
                Err(error) => {
                    let stage = if profile_workspace_ids.is_empty() {
                        SpokeRegistrationStageV1::Presence
                    } else {
                        SpokeRegistrationStageV1::Profiles
                    };
                    return registration_partial(
                        stage,
                        host,
                        presence_workspace_ids,
                        profile_workspace_ids,
                        error,
                    );
                }
            }
        }

        let snapshot = match service.snapshot() {
            Ok(snapshot) => snapshot,
            Err(error) => {
                let stage = if profile_workspace_ids.is_empty() {
                    SpokeRegistrationStageV1::Presence
                } else {
                    SpokeRegistrationStageV1::Profiles
                };
                return registration_partial(
                    stage,
                    host,
                    presence_workspace_ids,
                    profile_workspace_ids,
                    error,
                );
            }
        };
        SpokeRegistrationResultV1::completed(
            host,
            presence_workspace_ids,
            profile_workspace_ids,
            snapshot,
        )
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

    fn preflight_tool_call(
        &self,
        _name: &str,
        session_context: &ToolSessionContext,
    ) -> Result<(), OrbitError> {
        let (identity, snapshot) = self.verify_authority()?;
        let context = self.normalize_context(session_context.clone(), &identity);
        Self::require_active_remote_caller(&context, &snapshot)
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
        let denial = self
            .verify_authority()
            .and_then(|(_, snapshot)| Self::require_active_remote_caller(&context, &snapshot))
            .err()
            .unwrap_or(denial);
        self.record_denial(name, &context, &denial);
        denial
    }
}

impl super::learning::LearningSidecarHost for HubMcpHost {}

fn registration_partial(
    stage: SpokeRegistrationStageV1,
    host: HostRecord,
    presence_workspace_ids: Vec<String>,
    profile_workspace_ids: Vec<String>,
    error: OrbitError,
) -> SpokeRegistrationResultV1 {
    SpokeRegistrationResultV1::failed(
        Some(stage),
        Some(host),
        presence_workspace_ids,
        profile_workspace_ids,
        "projection_failed",
        error.to_string(),
    )
}

/// The explicit, non-empty `crew` on a task add/update, or `None` for any other
/// tool, an omitted crew, or an explicit crew-clearing (empty) value.
fn explicit_task_crew(name: &str, input: &Value) -> Option<String> {
    if !matches!(name, "orbit.task.add" | "orbit.task.update") {
        return None;
    }
    input
        .get("crew")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

fn placement_label(placement: McpToolPlacement) -> &'static str {
    match placement {
        McpToolPlacement::Hub => "hub",
        McpToolPlacement::Owner => "owner",
        McpToolPlacement::LocalDerived => "local-derived",
        McpToolPlacement::Composite => "composite",
    }
}
