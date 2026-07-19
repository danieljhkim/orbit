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

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use orbit_common::types::{
    AuditEventStatus, McpToolDefinition, McpToolPlacement, McpToolPolicyError, McpToolScope,
    McpTransport, ToolSessionContext, WorkspaceCheckoutRole, WorkspaceStatus, audit_execution_id,
    validate_mcp_tool_definitions,
};
use orbit_core::command::tool::{
    ToolEntryPoint, audit_role_label_for_entry_point, trusted_mcp_audit_context,
};
use orbit_core::routines::{HostIdentityState, HostMode, inspect_host_identity};
use orbit_core::runtime::HubCoordinationExecutor;
use orbit_core::{
    AuditEventInsertParams, NotFoundKind, OrbitError, OrbitRuntime, redact_sensitive_env_text,
};
use orbit_mcp::McpHost;
use serde::Deserialize;
use serde_json::{Value, json};

use super::hub_link::HubLinkPool;

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

pub(super) fn ensure_mcp_tool_exposed(name: &str) -> Result<(), OrbitError> {
    if is_mcp_tool_exposed(name) {
        Ok(())
    } else {
        Err(OrbitError::not_found(NotFoundKind::Tool, name.to_string()))
    }
}

pub(super) fn normalize_trusted_call_context(
    mut context: ToolSessionContext,
) -> ToolSessionContext {
    if context.transport.is_none() {
        context.transport = Some(McpTransport::Local);
    }
    if context.origin_session_id.is_none() {
        context.origin_session_id = Some(audit_execution_id("mcp-session"));
    }
    if context.mcp_call_id.is_none() {
        context.mcp_call_id = Some(audit_execution_id("mcall"));
    }
    context
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct RuntimeCacheKey {
    workspace_id: String,
    repo_root: PathBuf,
    shared_root: PathBuf,
    local_root: PathBuf,
}

#[derive(Debug, Clone)]
struct ExactCheckoutBinding {
    logical_workspace_id: String,
    key: RuntimeCacheKey,
    role: WorkspaceCheckoutRole,
    owner_machine_id: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct WorkspaceIdentityDocument {
    schema_version: u32,
    workspace_id: String,
}

/// Checkout-independent local MCP broker.
///
/// Listing is sourced exclusively from the canonical schema-plus-policy
/// registry. A workspace runtime is constructed only after a call has passed
/// capability, workspace, binding, role, owner, and placement preflight. Cache
/// entries are keyed by the exact selected checkout and are never authority:
/// every call re-runs resolution before looking up the cached runtime.
pub(super) struct BrokerMcpHost {
    global_root: PathBuf,
    runtimes: Mutex<BTreeMap<RuntimeCacheKey, OrbitRuntime>>,
    hub_links: Option<Arc<HubLinkPool>>,
}

impl BrokerMcpHost {
    pub(super) fn new(global_root: PathBuf) -> Self {
        Self {
            global_root,
            runtimes: Mutex::new(BTreeMap::new()),
            hub_links: None,
        }
    }

    pub(super) fn new_with_hub_link(global_root: PathBuf, hub_links: HubLinkPool) -> Self {
        Self {
            global_root,
            runtimes: Mutex::new(BTreeMap::new()),
            hub_links: Some(Arc::new(hub_links)),
        }
    }

    fn is_spoke(&self) -> Result<bool, OrbitError> {
        Ok(matches!(
            inspect_host_identity(&self.global_root)?,
            HostIdentityState::Present(identity) if identity.mode == HostMode::Spoke
        ))
    }

    fn scalar_capability(
        context: &ToolSessionContext,
    ) -> Result<orbit_common::types::McpCapability, OrbitError> {
        if context.effective_capabilities.len() != 1 {
            return Err(OrbitError::InvalidInput(
                "remote hub routing requires exactly one effective capability".to_string(),
            ));
        }
        context
            .effective_capabilities
            .iter()
            .next()
            .copied()
            .ok_or_else(|| {
                OrbitError::InvalidInput("remote hub routing requires a capability".to_string())
            })
    }

    fn remote_hub_call(
        &self,
        name: &str,
        input: Value,
        mut context: ToolSessionContext,
        workspace_id: Option<&str>,
    ) -> Result<Value, OrbitError> {
        let pool = self.hub_links.as_ref().ok_or_else(|| {
            OrbitError::HubUnavailable(
                "this spoke has no initialized MCP hub link; local fallback is forbidden"
                    .to_string(),
            )
        })?;
        context = normalize_trusted_call_context(context);
        context.transport = Some(McpTransport::SshMcp);
        context.process_machine_id = None;
        context.process_host_id = None;
        context.workspace = workspace_id.map(ToOwned::to_owned);
        context.workspace_id = workspace_id.map(ToOwned::to_owned);
        let capability = Self::scalar_capability(&context)?;
        pool.call(capability, name, input, context)
    }

    fn definition(&self, name: &str) -> Result<McpToolDefinition, OrbitError> {
        canonical_mcp_tool_definitions()
            .map_err(|error| OrbitError::InvalidInput(error.to_string()))?
            .into_iter()
            .find(|definition| definition.schema.name == name)
            .ok_or_else(|| OrbitError::not_found(NotFoundKind::Tool, name.to_string()))
    }

    fn authorize_capability(
        &self,
        definition: &McpToolDefinition,
        context: &ToolSessionContext,
    ) -> Result<(), OrbitError> {
        if definition
            .policy
            .allowed_capabilities()
            .iter()
            .any(|capability| context.effective_capabilities.contains(capability))
        {
            return Ok(());
        }
        let required = definition
            .policy
            .allowed_capabilities()
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join(", ");
        Err(OrbitError::InvalidInput(format!(
            "MCP capability denied for tool '{}': the effective session set must contain one of [{required}]",
            definition.schema.name
        )))
    }

    fn record_preflight_denial(
        &self,
        name: &str,
        input: &Value,
        session_context: &ToolSessionContext,
        denial: &OrbitError,
    ) {
        let mut context = normalize_trusted_call_context(session_context.clone());
        if let Some((workspace_id, binding)) = Self::selector(input, &context)
            .and_then(|selector| self.resolve_workspace(selector, true).ok())
        {
            context.workspace_id = Some(workspace_id);
            if let Some(binding) = binding {
                context.workspace = Some(binding.key.repo_root.to_string_lossy().into_owned());
            }
        }
        let params = mcp_preflight_failure_params(name, &context, denial);
        if let Err(error) =
            orbit_core::host_registry::record_global_audit_event_at(&self.global_root, &params)
        {
            tracing::warn!(
                tool = name,
                error = %error,
                "failed to persist global MCP preflight denial"
            );
        }
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

    fn resolve_workspace(
        &self,
        selector: &str,
        require_local: bool,
    ) -> Result<(String, Option<ExactCheckoutBinding>), OrbitError> {
        let path = Path::new(selector);
        if path.is_absolute() {
            let binding = self.resolve_exact_checkout(path)?;
            return Ok((binding.logical_workspace_id.clone(), Some(binding)));
        }
        if selector.contains('/') || selector == "." || selector == ".." {
            return Err(OrbitError::InvalidInput(format!(
                "workspace path selector '{selector}' must be absolute; the MCP broker never resolves paths from process cwd"
            )));
        }

        let registry_path = orbit_core::workspace_registry::registry_path_for(&self.global_root);
        let registry = orbit_core::workspace_registry::load_registry_from(&registry_path)?;
        let workspace = registry
            .workspaces
            .iter()
            .find(|workspace| workspace.id == selector);
        if workspace.is_none() {
            for checkout in &registry.checkouts {
                let identity_path = checkout.orbit_dir.join("config.yaml");
                let Ok(identity) = read_workspace_identity(&identity_path) else {
                    continue;
                };
                if identity.workspace_id == selector {
                    let binding = self.resolve_exact_checkout(&checkout.repo_root)?;
                    return Ok((binding.logical_workspace_id.clone(), Some(binding)));
                }
            }
        }
        let workspace = workspace.ok_or_else(|| {
                OrbitError::InvalidInput(format!(
                    "unknown logical workspace ID '{selector}'; pass a registered workspace ID or an absolute local checkout path"
                ))
            })?;
        if workspace.status != WorkspaceStatus::Active {
            return Err(OrbitError::InvalidInput(format!(
                "logical workspace ID '{}' is not active",
                workspace.id
            )));
        }
        if !require_local {
            return Ok((workspace.id.clone(), None));
        }
        let checkout = registry
            .checkouts
            .iter()
            .find(|checkout| checkout.workspace_id == workspace.id)
            .ok_or_else(|| self.local_checkout_unavailable(&workspace.id))?;
        let binding = self.resolve_exact_checkout(&checkout.repo_root)?;
        Ok((binding.logical_workspace_id.clone(), Some(binding)))
    }

    fn local_checkout_unavailable(&self, workspace_id: &str) -> OrbitError {
        OrbitError::InvalidInput(format!(
            "workspace '{workspace_id}' has no single validated exact local checkout; provide an absolute checkout path as the workspace selector"
        ))
    }

    fn resolve_exact_checkout(&self, selected: &Path) -> Result<ExactCheckoutBinding, OrbitError> {
        let selected = selected.canonicalize().map_err(|error| {
            OrbitError::InvalidInput(format!(
                "workspace path '{}' is unavailable: {error}",
                selected.display()
            ))
        })?;
        if !selected.is_dir() {
            return Err(OrbitError::InvalidInput(format!(
                "workspace path '{}' is not a directory",
                selected.display()
            )));
        }
        let repo_root = git_path(&selected, "--show-toplevel")?;
        let common_dir = git_path(&selected, "--git-common-dir")?;

        let registry_path = orbit_core::workspace_registry::registry_path_for(&self.global_root);
        let registry = orbit_core::workspace_registry::load_registry_from(&registry_path)?;
        let mut matches = Vec::new();
        for checkout in &registry.checkouts {
            let Ok(registered_common) = git_path(&checkout.repo_root, "--git-common-dir") else {
                continue;
            };
            if registered_common == common_dir {
                matches.push(checkout);
            }
        }
        if matches.len() != 1 {
            return Err(OrbitError::InvalidInput(format!(
                "workspace path '{}' does not resolve to exactly one registered local checkout (matched {}); register or repair the checkout binding before retrying",
                repo_root.display(),
                matches.len()
            )));
        }
        let checkout = matches[0];
        let workspace = registry
            .workspaces
            .iter()
            .find(|workspace| workspace.id == checkout.workspace_id)
            .ok_or_else(|| {
                OrbitError::InvalidInput(format!(
                    "checkout '{}' names unknown logical workspace ID '{}'",
                    checkout.repo_root.display(),
                    checkout.workspace_id
                ))
            })?;
        if workspace.status != WorkspaceStatus::Active {
            return Err(OrbitError::InvalidInput(format!(
                "logical workspace ID '{}' is not active",
                workspace.id
            )));
        }
        let shared_root = checkout.orbit_dir.canonicalize().map_err(|error| {
            OrbitError::InvalidInput(format!(
                "workspace '{}' shared Orbit root '{}' is unavailable: {error}",
                workspace.id,
                checkout.orbit_dir.display()
            ))
        })?;
        let identity_path = shared_root.join("config.yaml");
        let identity = read_workspace_identity(&identity_path).map_err(|error| {
            OrbitError::InvalidInput(format!(
                "workspace '{}' requires a valid '{}': {error}",
                workspace.id,
                identity_path.display()
            ))
        })?;
        if identity.schema_version != 1 {
            return Err(OrbitError::InvalidInput(format!(
                "workspace identity '{}' declares schema_version {}, expected schema_version 1",
                identity_path.display(),
                identity.schema_version,
            )));
        }
        // L-0098: legacy host-registry and task-registry IDs may differ for
        // the same validated checkout until their coordinated migration lands.
        let role = checkout.role.ok_or_else(|| {
            OrbitError::InvalidInput(format!(
                "workspace '{}' local checkout is missing its required owner/replica role",
                workspace.id
            ))
        })?;
        if role == WorkspaceCheckoutRole::Replica
            && checkout.owner_machine_id.as_deref() != workspace.owner_machine_id.as_deref()
        {
            return Err(OrbitError::InvalidInput(format!(
                "workspace '{}' replica owner identity drifted between logical and local registry records",
                workspace.id
            )));
        }
        Ok(ExactCheckoutBinding {
            logical_workspace_id: workspace.id.clone(),
            key: RuntimeCacheKey {
                workspace_id: identity.workspace_id,
                repo_root: repo_root.clone(),
                shared_root,
                local_root: repo_root.join(".orbit"),
            },
            role,
            owner_machine_id: workspace.owner_machine_id.clone(),
        })
    }

    fn preflight_placement(
        &self,
        placement: McpToolPlacement,
        workspace_id: &str,
        binding: Option<&ExactCheckoutBinding>,
    ) -> Result<(), OrbitError> {
        let identity = inspect_host_identity(&self.global_root)?;
        let (mode, machine_id) = match identity {
            HostIdentityState::Present(identity) => (identity.mode, Some(identity.machine_id)),
            HostIdentityState::Legacy { .. } | HostIdentityState::Absent => {
                (HostMode::Standalone, None)
            }
        };
        match placement {
            McpToolPlacement::Hub if mode == HostMode::Spoke && self.hub_links.is_none() => {
                Err(OrbitError::HubUnavailable(format!(
                    "hub placement for workspace '{workspace_id}' is unavailable from this spoke: no MCP hub transport is configured"
                )))
            }
            McpToolPlacement::Hub => Ok(()),
            McpToolPlacement::LocalDerived => binding
                .map(|_| ())
                .ok_or_else(|| self.local_checkout_unavailable(workspace_id)),
            McpToolPlacement::Owner => {
                let binding =
                    binding.ok_or_else(|| self.local_checkout_unavailable(workspace_id))?;
                if binding.role == WorkspaceCheckoutRole::Replica {
                    return Err(OrbitError::InvalidInput(format!(
                        "workspace '{workspace_id}' is a replica on this machine and may not author owner-placed mutations"
                    )));
                }
                if mode != HostMode::Standalone
                    && binding.owner_machine_id.as_deref() != machine_id.as_deref()
                {
                    return Err(OrbitError::InvalidInput(format!(
                        "workspace '{workspace_id}' is owned by another machine; current owner state is unavailable locally"
                    )));
                }
                Ok(())
            }
            McpToolPlacement::Composite => {
                let binding =
                    binding.ok_or_else(|| self.local_checkout_unavailable(workspace_id))?;
                if mode == HostMode::Spoke || binding.role == WorkspaceCheckoutRole::Replica {
                    return Err(OrbitError::InvalidInput(format!(
                        "composite placement for workspace '{workspace_id}' cannot collapse every declared route to one validated standalone or hub-owner checkout"
                    )));
                }
                Ok(())
            }
        }
    }

    fn runtime_for(&self, binding: &ExactCheckoutBinding) -> Result<OrbitRuntime, OrbitError> {
        let mut runtimes = self
            .runtimes
            .lock()
            .map_err(|_| OrbitError::Execution("MCP runtime cache lock poisoned".to_string()))?;
        if let Some(runtime) = runtimes.get(&binding.key) {
            return Ok(runtime.clone());
        }
        let runtime = OrbitRuntime::from_resolved_roots(
            &self.global_root,
            &binding.key.shared_root,
            &binding.key.local_root,
        )?;
        runtimes.insert(binding.key.clone(), runtime.clone());
        Ok(runtime)
    }

    fn global_call(&self, name: &str) -> Result<Value, OrbitError> {
        let snapshot = orbit_core::host_registry::registry_snapshot_at(&self.global_root)?;
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

    fn legacy_friction_root(
        &self,
        workspace_id: &str,
        binding: Option<&ExactCheckoutBinding>,
    ) -> Option<PathBuf> {
        if let Some(binding) = binding {
            return Some(binding.key.shared_root.join("frictions"));
        }
        let registry_path = orbit_core::workspace_registry::registry_path_for(&self.global_root);
        let registry = orbit_core::workspace_registry::load_registry_from(&registry_path).ok()?;
        registry
            .checkouts
            .iter()
            .find(|checkout| checkout.workspace_id == workspace_id)
            .map(|checkout| checkout.orbit_dir.join("frictions"))
    }

    fn record_coordination_outcome(
        &self,
        name: &str,
        context: &ToolSessionContext,
        result: &Result<Value, OrbitError>,
    ) {
        let success_placeholder = OrbitError::Execution(String::new());
        let error = result.as_ref().err().unwrap_or(&success_placeholder);
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
            tracing::warn!(tool = name, error = %error, "failed to persist global MCP coordination audit");
        }
    }

    fn resolved_call(
        &self,
        name: &str,
        mut input: Value,
        context: ToolSessionContext,
        in_process: Option<&mut dyn FnMut(Value, ToolSessionContext) -> Result<Value, OrbitError>>,
    ) -> Result<Value, OrbitError> {
        let mut context = normalize_trusted_call_context(context);
        let definition = self.definition(name)?;
        if let Err(error) = self.authorize_capability(&definition, &context) {
            self.record_preflight_denial(name, &input, &context, &error);
            return Err(error);
        }
        if definition.policy.scope() == McpToolScope::Global {
            if self.is_spoke()? {
                return self.remote_hub_call(name, input, context, None);
            }
            let result = self.global_call(name);
            self.record_coordination_outcome(name, &context, &result);
            return result;
        }

        let selector = match Self::selector(&input, &context) {
            Some(selector) => selector,
            None => {
                let error = OrbitError::InvalidInput(format!(
                    "tool '{name}' requires a workspace selector; pass a non-empty `workspace` argument or initialize with `_meta.orbit.workspace`"
                ));
                self.record_preflight_denial(name, &input, &context, &error);
                return Err(error);
            }
        };
        let with_context = name == "orbit.task.show"
            && input
                .get("with_context")
                .or_else(|| input.get("withContext"))
                .or_else(|| input.get("with-context"))
                .and_then(Value::as_bool)
                .unwrap_or(false);
        let require_local = definition.policy.placement() != McpToolPlacement::Hub || with_context;
        let (workspace_id, binding) = match self.resolve_workspace(selector, require_local) {
            Ok(resolved) => resolved,
            Err(error) => {
                self.record_preflight_denial(name, &input, &context, &error);
                return Err(error);
            }
        };

        if let Err(error) = self.preflight_placement(
            definition.policy.placement(),
            &workspace_id,
            binding.as_ref(),
        ) {
            self.record_preflight_denial(name, &input, &context, &error);
            return Err(error);
        }
        if definition.policy.placement() == McpToolPlacement::Hub && !with_context {
            context.workspace_id = Some(workspace_id.clone());
            context.workspace = Some(workspace_id.clone());
            if let Some(object) = input.as_object_mut()
                && object.contains_key("workspace")
            {
                object.insert("workspace".to_string(), Value::String(workspace_id.clone()));
            }
            if self.is_spoke()? {
                if name == "orbit.task.artifact.put" {
                    input = match orbit_core::prepare_remote_task_artifact_put(
                        input,
                        std::env::current_dir().ok().as_deref(),
                    ) {
                        Ok(prepared) => prepared,
                        Err(error) => {
                            self.record_preflight_denial(name, &Value::Null, &context, &error);
                            return Err(error);
                        }
                    };
                }
                return self.remote_hub_call(name, input, context, Some(&workspace_id));
            }
            let legacy_root = self.legacy_friction_root(&workspace_id, binding.as_ref());
            let result = HubCoordinationExecutor::new(&self.global_root, workspace_id, legacy_root)
                .and_then(|executor| executor.execute_tool(name, input, context.clone()));
            self.record_coordination_outcome(name, &context, &result);
            return result;
        }
        let binding = match binding {
            Some(binding) => binding,
            None => {
                let error = self.local_checkout_unavailable(&workspace_id);
                self.record_preflight_denial(name, &input, &context, &error);
                return Err(error);
            }
        };
        context.workspace_id = Some(workspace_id);
        context.workspace = Some(binding.key.repo_root.to_string_lossy().into_owned());
        if let Some(object) = input.as_object_mut()
            && object.contains_key("workspace")
        {
            object.insert(
                "workspace".to_string(),
                Value::String(binding.key.repo_root.to_string_lossy().into_owned()),
            );
        }
        let runtime = match self.runtime_for(&binding) {
            Ok(runtime) => runtime,
            Err(error) => {
                self.record_preflight_denial(name, &input, &context, &error);
                return Err(error);
            }
        };
        match in_process {
            Some(dispatch) => runtime
                .execute_in_process_tool_dispatch(
                    name,
                    input,
                    ToolEntryPoint::Mcp,
                    context.clone(),
                    |input| dispatch(input, context),
                )
                .map(|outcome| outcome.value),
            None => audited_mcp_call_with_session_context(&runtime, name, input, context),
        }
    }
}

impl McpHost for BrokerMcpHost {
    fn list_mcp_tool_definitions(&self) -> Result<Vec<McpToolDefinition>, OrbitError> {
        canonical_mcp_tool_definitions()
            .map_err(|error| OrbitError::InvalidInput(error.to_string()))
    }

    fn call_tool(
        &self,
        name: &str,
        input: Value,
        session_context: ToolSessionContext,
    ) -> Result<Value, OrbitError> {
        if let Err(error) = self.definition(name) {
            self.record_preflight_denial(name, &input, &session_context, &error);
            return Err(error);
        }
        self.resolved_call(name, input, session_context, None)
    }

    fn reject_tool_call(
        &self,
        name: &str,
        input: &Value,
        session_context: &ToolSessionContext,
        denial: OrbitError,
    ) -> OrbitError {
        self.record_preflight_denial(name, input, session_context, &denial);
        denial
    }

    fn call_in_process_tool(
        &self,
        name: &str,
        input: Value,
        session_context: ToolSessionContext,
        dispatch: &mut dyn FnMut(Value, ToolSessionContext) -> Result<Value, OrbitError>,
    ) -> Result<Value, OrbitError> {
        self.resolved_call(name, input, session_context, Some(dispatch))
    }
}

fn git_path(start: &Path, argument: &str) -> Result<PathBuf, OrbitError> {
    let output = Command::new("git")
        .arg("-C")
        .arg(start)
        .args(["rev-parse", "--path-format=absolute", argument])
        .output()
        .map_err(|error| {
            OrbitError::Execution(format!("run git for '{}': {error}", start.display()))
        })?;
    if !output.status.success() {
        return Err(OrbitError::InvalidInput(format!(
            "workspace path '{}' is not a valid Git checkout: {}",
            start.display(),
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    let raw = String::from_utf8(output.stdout).map_err(|error| {
        OrbitError::InvalidInput(format!("git returned a non-UTF-8 path: {error}"))
    })?;
    let path = PathBuf::from(raw.trim());
    path.canonicalize().map_err(|error| {
        OrbitError::InvalidInput(format!(
            "git resolved '{}' to unavailable path '{}': {error}",
            start.display(),
            path.display()
        ))
    })
}

fn read_workspace_identity(path: &Path) -> Result<WorkspaceIdentityDocument, OrbitError> {
    let raw = std::fs::read_to_string(path).map_err(|error| OrbitError::Io(error.to_string()))?;
    serde_yaml::from_str(&raw).map_err(|error| {
        OrbitError::InvalidInput(format!(
            "invalid workspace identity '{}': {error}",
            path.display()
        ))
    })
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
        normalize_trusted_call_context(ToolSessionContext::trusted_local(None, None, None)),
    )
}

pub(super) fn audited_mcp_call_with_session_context(
    runtime: &OrbitRuntime,
    name: &str,
    input: Value,
    session_context: ToolSessionContext,
) -> Result<Value, OrbitError> {
    let session_context = normalize_trusted_call_context(session_context);
    if let Err(err) = ensure_mcp_tool_exposed(name) {
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
    let params = mcp_preflight_failure_params(name, session_context, err);
    if let Err(write_err) = runtime.record_audit_event(&params) {
        eprintln!("warning: failed to persist MCP preflight audit event: {write_err}");
    }
}

pub(super) fn mcp_preflight_failure_params(
    name: &str,
    session_context: &ToolSessionContext,
    err: &OrbitError,
) -> AuditEventInsertParams {
    let start = Instant::now();
    let role = audit_role_label_for_entry_point(&Value::Null, None, None, ToolEntryPoint::Mcp);
    let (audit_context, correlation_error) = trusted_mcp_audit_context(session_context);
    let duration_ms = (start.elapsed().as_millis() as i64).max(1);
    let working_directory = std::env::current_dir()
        .map(|path| path.to_string_lossy().into_owned())
        .unwrap_or_else(|_| ".".to_string());

    AuditEventInsertParams {
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
    }
}
