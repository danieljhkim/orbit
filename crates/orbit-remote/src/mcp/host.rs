//! MCP host implementations and audit bracketing.
//!
//! Canonical listing composes generic builtin definitions with Remote-owned
//! discovery definitions while preserving each schema's adjacent MCP policy.
//! Runtime-backed and Remote-owned execution both use the audit
//! boundary tagged with [`ToolEntryPoint::Mcp`], so every dispatch has the same
//! identity-resolution rules as the CLI path. Registry preflight failures are
//! recorded explicitly before runtime dispatch. Either rejection path produces
//! a failure-status row.

use std::collections::{BTreeMap, BTreeSet};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use orbit_common::types::{
    AuditEventStatus, McpCapability, McpToolDefinition, McpToolPlacement, McpToolPolicyError,
    McpToolScope, McpTransport, ToolSessionContext, WorkspaceCheckoutRole, WorkspaceStatus,
    audit_execution_id, validate_mcp_tool_definitions,
};
use orbit_core::command::tool::{
    ToolEntryPoint, audit_role_label_for_entry_point, trusted_mcp_audit_context,
};
use orbit_core::runtime::HubCoordinationExecutor;
use orbit_core::{
    AuditEventInsertParams, NotFoundKind, OrbitError, OrbitRuntime, WorkspaceRuntimeBinding,
    redact_sensitive_env_text,
};
use orbit_mcp::McpHost;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::runtime::RemoteRuntimeFactory;
use crate::{HostIdentityState, inspect_host_identity};

use super::owner_link::OwnerLinkPool;

/// One configured route to an owner machine, with the exact capability set
/// `mcp.toml` grants for it.
///
/// The ceiling travels with the route because it is per target: a session may
/// legitimately hold `operator` while one owner grants only `agent`. Keeping
/// the set here lets the broker say *why* a route is unusable instead of
/// reporting it as absent.
pub(super) struct OwnerRoute {
    pub(super) allowed_capabilities: BTreeSet<McpCapability>,
    pub(super) pool: Arc<OwnerLinkPool>,
}

/// Configured owner routes keyed by the owner `machine_id` they reach.
///
/// Routes are per machine, not per workspace (§5.1), so the broker looks up a
/// workspace's declared owner and then finds that machine here. A missing entry
/// is a refusal, never a fallback to local coordination state.
pub(super) type OwnerRouteTable = BTreeMap<String, OwnerRoute>;

pub fn canonical_mcp_tool_definitions() -> Result<Vec<McpToolDefinition>, McpToolPolicyError> {
    let mut definitions = orbit_tools::canonical_builtin_mcp_tool_definitions()?;
    definitions.extend(super::discovery::discovery_tool_definitions()?);
    definitions.sort_by(|left, right| left.schema.name.cmp(&right.schema.name));
    validate_mcp_tool_definitions(&definitions)?;
    Ok(definitions)
}

pub fn safe_mcp_tool_names() -> Vec<String> {
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
    ship_mode: String,
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

/// Where a workspace's coordination state lives, relative to this machine.
///
/// This is deliberately two-valued: v1 has exactly one coordination writer per
/// workspace and no third-machine relay, so a call either executes here or
/// crosses exactly one configured route to the machine named here.
#[derive(Debug, Clone, PartialEq, Eq)]
enum OwnerResolution {
    Local,
    Remote(String),
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
    owner_routes: OwnerRouteTable,
}

impl BrokerMcpHost {
    /// A broker with no configured owner routes: every workspace it serves must
    /// be owned by this machine. Production always goes through
    /// [`Self::new_with_owner_routes`], which passes the parsed `mcp.toml`.
    #[cfg(test)]
    pub(super) fn new(global_root: PathBuf) -> Self {
        Self::new_with_owner_routes(global_root, OwnerRouteTable::new())
    }

    pub(super) fn new_with_owner_routes(
        global_root: PathBuf,
        owner_routes: OwnerRouteTable,
    ) -> Self {
        Self {
            global_root,
            runtimes: Mutex::new(BTreeMap::new()),
            owner_routes,
        }
    }

    /// This machine's stable identity, when it has one.
    fn local_machine_id(&self) -> Result<Option<String>, OrbitError> {
        Ok(match inspect_host_identity(&self.global_root)? {
            HostIdentityState::Present(identity) => Some(identity.machine_id),
            HostIdentityState::Legacy { .. } | HostIdentityState::Absent => None,
        })
    }

    fn scalar_capability(
        context: &ToolSessionContext,
    ) -> Result<orbit_common::types::McpCapability, OrbitError> {
        if context.effective_capabilities.len() != 1 {
            return Err(OrbitError::InvalidInput(
                "remote owner routing requires exactly one effective capability".to_string(),
            ));
        }
        context
            .effective_capabilities
            .iter()
            .next()
            .copied()
            .ok_or_else(|| {
                OrbitError::InvalidInput("remote owner routing requires a capability".to_string())
            })
    }

    fn remote_owner_call(
        &self,
        name: &str,
        input: Value,
        mut context: ToolSessionContext,
        owner_machine_id: &str,
        workspace_id: Option<&str>,
    ) -> Result<Value, OrbitError> {
        let route = self.owner_routes.get(owner_machine_id).ok_or_else(|| {
            OrbitError::OwnerUnavailable(format!(
                "no initialized MCP route to owner machine '{owner_machine_id}'; local fallback is forbidden"
            ))
        })?;
        context = normalize_trusted_call_context(context);
        context.transport = Some(McpTransport::SshMcp);
        context.process_machine_id = None;
        context.process_host_id = None;
        context.workspace = workspace_id.map(ToOwned::to_owned);
        context.workspace_id = workspace_id.map(ToOwned::to_owned);
        let capability = Self::scalar_capability(&context)?;
        route.pool.call(capability, name, input, context)
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
        Err(OrbitError::CapabilityDenied(format!(
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
            .and_then(|selector| self.resolve_workspace(selector).ok())
        {
            context.workspace_id = Some(workspace_id);
            if let Some(binding) = binding {
                context.workspace = Some(binding.key.repo_root.to_string_lossy().into_owned());
            }
        }
        let params = mcp_preflight_failure_params(name, &context, denial);
        if let Err(error) = crate::record_global_audit_event_at(&self.global_root, &params) {
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

    /// Resolve a selector to its logical workspace ID and, when this machine
    /// holds one, its validated exact local checkout.
    ///
    /// The binding is always attached when it exists — placement preflight, not
    /// this lookup, decides whether a call may proceed without one. A workspace
    /// registered here with no local checkout resolves to `(id, None)`.
    fn resolve_workspace(
        &self,
        selector: &str,
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

        let registry_path = crate::workspace_registry::registry_path_for(&self.global_root);
        let registry = crate::workspace_registry::load_registry_from(&registry_path)?;
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
        let Some(checkout) = registry
            .checkouts
            .iter()
            .find(|checkout| checkout.workspace_id == workspace.id)
        else {
            return Ok((workspace.id.clone(), None));
        };
        // A registered checkout that no longer validates is indistinguishable
        // here from having none: a checkoutless coordination call still
        // succeeds, and a call that needs the checkout is refused by placement
        // preflight with the same message. An explicit path selector still
        // fails hard above, because the caller named that path.
        match self.resolve_exact_checkout(&checkout.repo_root) {
            Ok(binding) => Ok((binding.logical_workspace_id.clone(), Some(binding))),
            Err(_) => Ok((workspace.id.clone(), None)),
        }
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

        let registry_path = crate::workspace_registry::registry_path_for(&self.global_root);
        let registry = crate::workspace_registry::load_registry_from(&registry_path)?;
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
                ship_mode: orbit_core::resolved_ship_mode(workspace)
                    .as_input_value()
                    .to_string(),
                repo_root: repo_root.clone(),
                shared_root,
                local_root: repo_root.join(".orbit"),
            },
            role,
            owner_machine_id: workspace.owner_machine_id.clone(),
        })
    }

    /// Where a workspace's coordination state lives, per the machine-local
    /// workspace registry. `mcp.toml` never participates: it maps an owner
    /// machine to a route, it cannot declare ownership.
    fn resolve_owner(
        &self,
        workspace_id: &str,
        binding: Option<&ExactCheckoutBinding>,
    ) -> Result<OwnerResolution, OrbitError> {
        let declared = match binding {
            Some(binding) => binding.owner_machine_id.clone(),
            None => {
                let registry_path = crate::workspace_registry::registry_path_for(&self.global_root);
                crate::workspace_registry::load_registry_from(&registry_path)?
                    .workspaces
                    .iter()
                    .find(|workspace| workspace.id == workspace_id)
                    .and_then(|workspace| workspace.owner_machine_id.clone())
            }
        };
        let Some(declared) = declared else {
            // A workspace with no declared owner predates the ownership model.
            // The machine holding it is the only coordination writer it has.
            return Ok(OwnerResolution::Local);
        };
        match self.local_machine_id()? {
            // No stable identity means nothing can be owned elsewhere.
            None => Ok(OwnerResolution::Local),
            Some(local) if local == declared => Ok(OwnerResolution::Local),
            Some(_) => Ok(OwnerResolution::Remote(declared)),
        }
    }

    /// Refuse an owner-placed call for a workspace owned elsewhere, naming the
    /// owning machine and the configured route if one exists (§4.2 rule 4).
    fn remote_owner_refusal(
        &self,
        name: &str,
        workspace_id: &str,
        owner: &str,
        capability: Option<McpCapability>,
    ) -> OrbitError {
        let route = match (self.owner_routes.get(owner), capability) {
            (None, _) => {
                format!("no [[owner]] route to '{owner}' is configured in machine-global mcp.toml")
            }
            (Some(route), Some(capability))
                if !route.allowed_capabilities.contains(&capability) =>
            {
                format!(
                    "the configured route to '{owner}' does not grant capability '{capability}'"
                )
            }
            (Some(_), _) => {
                format!("a route to '{owner}' is configured, but only task tools may cross it")
            }
        };
        OrbitError::InvalidInput(format!(
            "tool '{name}' is owner-placed and workspace '{workspace_id}' is owned by machine '{owner}': {route}. Orbit never relays this call through a third machine"
        ))
    }

    /// Placement preflight (§4.2). `Owner` does not mean "find and contact any
    /// owner": it resolves the owner from machine-local state, executes here
    /// when that is this machine, dispatches over the one configured route for
    /// the task surface, and otherwise refuses by name.
    fn preflight_placement(
        &self,
        placement: McpToolPlacement,
        name: &str,
        workspace_id: &str,
        owner: &OwnerResolution,
        capability: McpCapability,
        binding: Option<&ExactCheckoutBinding>,
    ) -> Result<(), OrbitError> {
        match placement {
            McpToolPlacement::LocalDerived => binding
                .map(|_| ())
                .ok_or_else(|| self.local_checkout_unavailable(workspace_id)),
            McpToolPlacement::Owner => match owner {
                OwnerResolution::Local => {
                    // A replica binding whose workspace record still names this
                    // machine as owner is drifted registry state, not a route.
                    // Refuse rather than author a replica-local fork.
                    if binding.is_some_and(|binding| binding.role == WorkspaceCheckoutRole::Replica)
                    {
                        return Err(OrbitError::InvalidInput(format!(
                            "workspace '{workspace_id}' is a replica on this machine and may not author owner-placed mutations"
                        )));
                    }
                    // The coordination surface is checkoutless by construction
                    // (§2.3): it opens only the global task/friction stores.
                    // Every other owner-placed tool reads checkout-derived
                    // state and needs the validated owner checkout.
                    if served_by_coordination_executor(name) || binding.is_some() {
                        Ok(())
                    } else {
                        Err(self.local_checkout_unavailable(workspace_id))
                    }
                }
                OwnerResolution::Remote(owner) => {
                    let usable = crosses_owner_route(name)
                        && self
                            .owner_routes
                            .get(owner)
                            .is_some_and(|route| route.allowed_capabilities.contains(&capability));
                    if !usable {
                        return Err(self.remote_owner_refusal(
                            name,
                            workspace_id,
                            owner,
                            Some(capability),
                        ));
                    }
                    Ok(())
                }
            },
            McpToolPlacement::Composite => {
                let binding =
                    binding.ok_or_else(|| self.local_checkout_unavailable(workspace_id))?;
                // A composite call fans out to an owner branch and a local
                // branch. Both must resolve here: there is no owner proxy, and
                // a replica must never be presented as current.
                if !matches!(owner, OwnerResolution::Local)
                    || binding.role == WorkspaceCheckoutRole::Replica
                {
                    return Err(OrbitError::InvalidInput(format!(
                        "composite placement for workspace '{workspace_id}' requires a validated owner checkout on this machine; its owner branch cannot be proxied and a replica is not current"
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
        let runtime = RemoteRuntimeFactory::open_resolved_checkout(
            &self.global_root,
            &binding.key.shared_root,
            &binding.key.local_root,
            WorkspaceRuntimeBinding {
                workspace_id: binding.key.workspace_id.clone(),
                repo_root: binding.key.repo_root.clone(),
                ship_mode: orbit_core::ShipMode::parse(&binding.key.ship_mode)?,
            },
        )?;
        runtimes.insert(binding.key.clone(), runtime.clone());
        Ok(runtime)
    }

    fn global_call(&self, name: &str) -> Result<Value, OrbitError> {
        let identity = crate::load_host_identity(&self.global_root)?;
        let registry_path = crate::workspace_registry::registry_path_for(&self.global_root);
        let registry = crate::workspace_registry::load_registry_from(&registry_path)?;
        super::discovery::execute_discovery_tool(name, &registry, &identity.machine_id)
    }

    /// Project the sanitized crew-discovery response for `orbit.crew.list` from
    /// this machine's own crew configuration.
    ///
    /// Only reached when the workspace is owned here (§8.1): a workspace owned
    /// elsewhere has no crew surface on this broker, because `orbit.crew.list`
    /// does not cross the owner route.
    fn crew_discovery(&self, workspace_id: &str) -> Result<Value, OrbitError> {
        let discovery =
            crate::OwnerLocalCrews::new(self.global_root.clone()).crew_discovery(workspace_id)?;
        serde_json::to_value(discovery)
            .map_err(|error| OrbitError::Execution(format!("serialize crew discovery: {error}")))
    }

    /// Resolve the coordination task-registry partition key for a workspace.
    ///
    /// The host registry keys workspaces by their logical ID while the
    /// coordination task registry partitions by the checkout identity recorded
    /// in `.orbit/config.yaml`. `orbit workspace init` writes both from the same
    /// value, but workspaces registered before that convergence carry two
    /// distinct keys (L-0098) — and reading coordination tasks under the
    /// logical key then silently resolves an empty partition, so every hub-
    /// placement task tool reports `task not found` for tasks the checkout-local
    /// surfaces serve fine (F2026-07-099, ORB-10448).
    ///
    /// A validated exact-checkout binding already carries the identity key and
    /// is authoritative. Without one, fall back to the registered checkout's
    /// identity document, then to the logical ID for a genuinely checkoutless
    /// workspace.
    fn coordination_workspace_id(
        &self,
        workspace_id: &str,
        binding: Option<&ExactCheckoutBinding>,
    ) -> String {
        if let Some(binding) = binding {
            return binding.key.workspace_id.clone();
        }
        let registry_path = crate::workspace_registry::registry_path_for(&self.global_root);
        let Ok(registry) = crate::workspace_registry::load_registry_from(&registry_path) else {
            return workspace_id.to_string();
        };
        registry
            .checkouts
            .iter()
            .find(|checkout| checkout.workspace_id == workspace_id)
            .and_then(|checkout| {
                read_workspace_identity(&checkout.orbit_dir.join("config.yaml")).ok()
            })
            .map(|identity| identity.workspace_id)
            .unwrap_or_else(|| workspace_id.to_string())
    }

    fn legacy_friction_root(
        &self,
        workspace_id: &str,
        binding: Option<&ExactCheckoutBinding>,
    ) -> Option<PathBuf> {
        if let Some(binding) = binding {
            return Some(binding.key.shared_root.join("frictions"));
        }
        let registry_path = crate::workspace_registry::registry_path_for(&self.global_root);
        let registry = crate::workspace_registry::load_registry_from(&registry_path).ok()?;
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
        if let Err(error) = crate::record_global_audit_event_at(&self.global_root, &params) {
            tracing::warn!(tool = name, error = %error, "failed to persist global MCP coordination audit");
        }
    }

    /// Merge an unscoped `orbit.search` request over every active local
    /// workspace. A merged row carries the logical workspace selector needed
    /// to feed its otherwise workspace-local ID back into a follow-up call.
    fn merged_search(&self, input: Value) -> Result<Value, OrbitError> {
        let registry_path = crate::workspace_registry::registry_path_for(&self.global_root);
        let registry = crate::workspace_registry::load_registry_from(&registry_path)?;
        let mut results = Vec::new();
        let mut notes = Vec::new();
        let mut mode = None;
        let mut kind = None;

        for (workspace, checkout) in crate::workspace_registry::local_workspaces(&registry) {
            if workspace.status != WorkspaceStatus::Active {
                continue;
            }
            let binding = self.resolve_exact_checkout(&checkout.repo_root)?;
            let runtime = self.runtime_for(&binding)?;
            let response = runtime.run_tool("orbit.search", input.clone())?;
            let response = response.as_object().ok_or_else(|| {
                OrbitError::Execution(
                    "serialize merged search response: expected object".to_string(),
                )
            })?;
            mode.get_or_insert_with(|| response.get("mode").cloned().unwrap_or(Value::Null));
            kind.get_or_insert_with(|| response.get("kind").cloned().unwrap_or(Value::Null));

            if let Some(workspace_notes) = response.get("notes").and_then(Value::as_array) {
                notes.extend(workspace_notes.iter().cloned());
            }
            if let Some(rows) = response.get("results").and_then(Value::as_array) {
                for row in rows {
                    let mut row = row.clone();
                    let object = row.as_object_mut().ok_or_else(|| {
                        OrbitError::Execution(
                            "serialize merged search response: result row is not an object"
                                .to_string(),
                        )
                    })?;
                    object.insert("workspace".to_string(), Value::String(workspace.id.clone()));
                    results.push(row);
                }
            }
        }

        Ok(json!({
            "mode": mode.unwrap_or_else(|| Value::String("lexical".to_string())),
            "kind": kind.unwrap_or_else(|| input.get("kind").cloned().unwrap_or_else(|| Value::String("all".to_string()))),
            "results": results,
            "notes": notes,
        }))
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
        // Registry-wide discovery is `local-derived` and `global`: it reads
        // this machine's own registry and enumerates only the workspaces this
        // machine owns, so it never selects a workspace and never routes.
        if definition.policy.scope() == McpToolScope::Global {
            let result = self.global_call(name);
            self.record_coordination_outcome(name, &context, &result);
            return result;
        }

        // `all: true` without a workspace selector is the broker's explicit
        // cross-workspace search mode. Supplying a workspace (directly or in
        // session metadata) retains the established per-workspace meaning of
        // `all` and its unchanged response shape.
        if name == "orbit.search"
            && input.get("all").and_then(Value::as_bool) == Some(true)
            && Self::selector(&input, &context).is_none()
        {
            let result = self.merged_search(input);
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
        let placement = definition.policy.placement();

        // Resolve the workspace without demanding a checkout first: whether one
        // is required depends on where the workspace is owned.
        let (workspace_id, binding) = match self.resolve_workspace(selector) {
            Ok(resolved) => resolved,
            Err(error) => {
                self.record_preflight_denial(name, &input, &context, &error);
                return Err(error);
            }
        };
        let owner = match self.resolve_owner(&workspace_id, binding.as_ref()) {
            Ok(owner) => owner,
            Err(error) => {
                self.record_preflight_denial(name, &input, &context, &error);
                return Err(error);
            }
        };
        let owner_is_local = matches!(owner, OwnerResolution::Local);

        let with_context = name == "orbit.task.show"
            && input
                .get("with_context")
                .or_else(|| input.get("withContext"))
                .or_else(|| input.get("with-context"))
                .and_then(Value::as_bool)
                .unwrap_or(false);
        if with_context && !owner_is_local {
            let error = OrbitError::InvalidInput(
                "remote `orbit.task.show` cannot provide `with_context`; checkout-derived enrichment is local-only and local coordination fallback is forbidden"
                    .to_string(),
            );
            self.record_preflight_denial(name, &input, &context, &error);
            return Err(error);
        }
        // The workflow executor needs the exact selected checkout, so workflow
        // tools are owner-placed *and* checkout-bound. Multi-host execution is
        // deferred to v2 (ADR-0358), so a workspace owned elsewhere has no
        // workflow surface here at all — owner preflight refuses it below.
        let local_workflow_execution = name.starts_with("orbit.workflow.") && owner_is_local;

        let capability = match Self::scalar_capability(&context) {
            Ok(capability) => capability,
            Err(error) => {
                self.record_preflight_denial(name, &input, &context, &error);
                return Err(error);
            }
        };
        if let Err(error) = self.preflight_placement(
            placement,
            name,
            &workspace_id,
            &owner,
            capability,
            binding.as_ref(),
        ) {
            self.record_preflight_denial(name, &input, &context, &error);
            return Err(error);
        }

        // An owner-placed call for a workspace this machine owns dispatches
        // through the checkout-independent coordination executor: no SSH to
        // self, no second MCP serialization boundary (§2.3). One owned
        // elsewhere reached this point only because owner preflight admitted it
        // to the configured route.
        // `orbit.crew.list` is owner-placed but config-backed rather than
        // coordination-store-backed, so it is its own branch below.
        let owner_dispatch = placement == McpToolPlacement::Owner
            && !with_context
            && !local_workflow_execution
            && (!owner_is_local
                || served_by_coordination_executor(name)
                || name == "orbit.crew.list");
        if owner_dispatch {
            context.workspace_id = Some(workspace_id.clone());
            context.workspace = Some(workspace_id.clone());
            if let Some(object) = input.as_object_mut()
                && object.contains_key("workspace")
            {
                object.insert("workspace".to_string(), Value::String(workspace_id.clone()));
            }
            if let OwnerResolution::Remote(owner_machine_id) = &owner {
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
                return self.remote_owner_call(
                    name,
                    input,
                    context,
                    owner_machine_id,
                    Some(&workspace_id),
                );
            }
            // Crew validation runs where the workspace is owned, so it reads
            // this machine's local config directly (§8.1); it never opens the
            // coordination task registry.
            if name == "orbit.crew.list" {
                let result = self.crew_discovery(&workspace_id);
                self.record_coordination_outcome(name, &context, &result);
                return result;
            }
            let legacy_root = self.legacy_friction_root(&workspace_id, binding.as_ref());
            let task_partition_id = self.coordination_workspace_id(&workspace_id, binding.as_ref());
            let result = crate::runtime::sync_task_prefix(&self.global_root).and_then(|()| {
                HubCoordinationExecutor::new_with_task_partition(
                    &self.global_root,
                    workspace_id,
                    task_partition_id,
                    legacy_root,
                )
                .and_then(|executor| executor.execute_tool(name, input, context.clone()))
            });
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

/// The coordination-record mutators all pass the broker before either the
/// checkout-local runtime or the checkoutless coordinator can open a store.
///
/// ORB-10727: this is now the *route* predicate. A workspace owned elsewhere
/// admits exactly the task surface over its configured owner route
/// (§4.2 rule 3); every other owner-placed tool is refused by name. Keep it
/// explicit and test it against the registered MCP surface so a new task verb
/// cannot silently become unreachable — or a non-task tool silently start
/// crossing a machine boundary.
pub(super) fn crosses_owner_route(name: &str) -> bool {
    name.starts_with("orbit.task.")
}

/// Whether the checkout-independent coordination executor serves this tool.
///
/// Placement answers *which machine*; this answers *which executor on it*. The
/// two used to be conflated, because the withdrawn `hub` placement meant both
/// "the hub machine" and "the checkoutless coordination store". Collapsing
/// `hub` into `owner` (ADR-0355) separates them: `orbit.learning.*` and
/// `orbit.auto_task.*` are equally owner-placed but read checkout-derived
/// state, so they run through the owner's validated checkout runtime instead.
///
/// This must stay in step with `HubCoordinationExecutor::execute`, which
/// rejects any other action outright.
fn served_by_coordination_executor(name: &str) -> bool {
    name.starts_with("orbit.task.") || name.starts_with("orbit.friction.")
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

impl super::learning::LearningSidecarHost for BrokerMcpHost {}

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
/// invoked outside the CLI audit middleware guard.
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
        let _ = writeln!(
            std::io::stderr().lock(),
            "warning: failed to persist MCP preflight audit event: {write_err}"
        );
    }
}

pub(super) fn mcp_preflight_failure_params(
    name: &str,
    session_context: &ToolSessionContext,
    err: &OrbitError,
) -> AuditEventInsertParams {
    let start = Instant::now();
    let role = audit_role_label_for_entry_point(&Value::Null, None, None, ToolEntryPoint::Mcp);
    let audit_context = trusted_mcp_audit_context();
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
        error_message: Some(redact_sensitive_env_text(&err.to_string())),
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
        // ORB-10727 [ADR-0358]: run leases are deferred to v2; no v1 session
        // carries one. The audit column stays for the v2 producer.
        lease_id: None,
        task_id: audit_context.task_id,
        job_run_id: audit_context.job_run_id,
        activity_id: audit_context.activity_id,
        step_index: audit_context.step_index,
    }
}
