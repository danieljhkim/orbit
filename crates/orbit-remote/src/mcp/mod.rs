//! Remote MCP broker, coordination hub, and spoke-link composition.

mod config;
mod contract;
mod discovery;
mod host;
mod hub;
mod hub_client;
mod hub_link;
mod learning;
mod registration;
mod schema;
mod transport;

#[cfg(test)]
mod tests;

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use orbit_common::types::{
    HUB_KNOWLEDGE_ALLOCATION_SCHEMA_VERSION, HubKnowledgeAllocationRequestV1,
    HubKnowledgeAllocationV1, KnowledgeIdKind, McpCapability, ToolSessionContext, WorkspaceStatus,
    audit_execution_id,
};
use orbit_core::OrbitError;
use orbit_core::runtime::resolve_global_root;
use orbit_mcp::{McpHost, McpResultDecorator, McpServerComposition, McpServerMetadata};

use crate::{HostIdentityState, HostMode, inspect_host_identity};

use self::config::load_trusted_mcp_config;
use self::contract::hub_schema_digest;
use self::host::BrokerMcpHost;
use self::hub::HubMcpHost;
use self::hub_link::HubLinkPool;
use self::learning::{LearningSidecarDecorator, LearningSidecarHost};
use self::schema::RemoteInputSchemaResolver;
use self::transport::{PrivateHubRequestHandler, RemoteCallContextResolver};

pub use self::host::{canonical_mcp_tool_definitions, safe_mcp_tool_names};
pub use self::registration::register_local_spoke;

/// Human-only CLI allocation path. This is deliberately not registered as an
/// MCP or generic builtin tool.
pub fn allocate_knowledge_id_for_human(
    workspace_selector: &str,
    kind: KnowledgeIdKind,
    model: Option<String>,
) -> Result<HubKnowledgeAllocationV1, OrbitError> {
    let global_root = resolve_global_root()?;
    let identity = match inspect_host_identity(&global_root)? {
        HostIdentityState::Present(identity) => identity,
        HostIdentityState::Legacy { .. } | HostIdentityState::Absent => {
            return Err(OrbitError::InvalidInput(
                "orbit knowledge allocate requires explicit hub/spoke host identity; standalone workspaces retain their local allocator"
                    .to_string(),
            ));
        }
    };
    let workspace_id = resolve_stable_workspace_id(&global_root, workspace_selector)?;
    let mut context = ToolSessionContext::trusted_local(
        Some(workspace_id.clone()),
        Some(identity.machine_id.clone()),
        Some(identity.host_id.clone()),
    );
    context.workspace = Some(workspace_id.clone());
    context.effective_capabilities = BTreeSet::from([McpCapability::Operator]);
    context.origin_session_id = Some(audit_execution_id("knowledge-allocate-session"));
    context.mcp_call_id = Some(audit_execution_id("mcall-knowledge-allocate"));
    let request = HubKnowledgeAllocationRequestV1 {
        schema_version: HUB_KNOWLEDGE_ALLOCATION_SCHEMA_VERSION,
        workspace_id,
        kind,
        model,
    };

    match identity.mode {
        HostMode::Hub => {
            let allocator = crate::HubKnowledgeSequenceService::at(&global_root)?;
            allocator.ensure_public_cutover_active()?;
            allocator.allocate(&request, &context)
        }
        HostMode::Spoke => {
            let trusted = load_trusted_mcp_config(&global_root)?;
            let pool = spoke_hub_link(
                &trusted,
                &identity,
                McpCapability::Operator,
            )?;
            context.transport = Some(orbit_common::types::McpTransport::SshMcp);
            context.process_machine_id = None;
            context.process_host_id = None;
            pool.allocate_knowledge_id(McpCapability::Operator, request, context)
        }
        HostMode::Standalone => Err(OrbitError::InvalidInput(
            "orbit knowledge allocate is unavailable in standalone mode; use the supported workspace-local allocator"
                .to_string(),
        )),
    }
}

/// Execute an agent-facing knowledge lifecycle tool through the same live
/// owner broker used by MCP. Direct CLI wrappers and `orbit tool run` use this
/// in managed hub/spoke mode so they cannot bypass placement enforcement.
pub fn execute_managed_knowledge_tool(
    global_root: &std::path::Path,
    workspace_selector: &str,
    name: &str,
    mut input: serde_json::Value,
    model: Option<String>,
) -> Result<serde_json::Value, OrbitError> {
    if preallocated_name_or_current(name).is_none() {
        return Err(OrbitError::InvalidInput(format!(
            "tool '{name}' is not a managed knowledge lifecycle surface"
        )));
    }
    let identity = match inspect_host_identity(global_root)? {
        HostIdentityState::Present(identity) if identity.mode != HostMode::Standalone => identity,
        _ => {
            return Err(OrbitError::InvalidInput(
                "managed knowledge routing requires explicit hub/spoke mode".to_string(),
            ));
        }
    };
    let host = if identity.mode == HostMode::Spoke {
        let trusted = load_trusted_mcp_config(global_root)?;
        BrokerMcpHost::new_with_hub_link(
            global_root.to_path_buf(),
            spoke_hub_link(&trusted, &identity, McpCapability::Agent)?,
        )
    } else {
        BrokerMcpHost::new(global_root.to_path_buf())
    };
    if let Some(object) = input.as_object_mut() {
        object.insert(
            "workspace".to_string(),
            serde_json::Value::String(workspace_selector.to_string()),
        );
        if let Some(model) = model {
            object.insert("model".to_string(), serde_json::Value::String(model));
        }
    }
    let mut context =
        ToolSessionContext::trusted_local(None, Some(identity.machine_id), Some(identity.host_id));
    context.effective_capabilities = BTreeSet::from([McpCapability::Agent]);
    host.call_tool(name, input, context)
}

fn preallocated_name_or_current(name: &str) -> Option<()> {
    (host::preallocated_knowledge_kind(name).is_some() || host::is_current_knowledge_tool(name))
        .then_some(())
}

fn resolve_stable_workspace_id(
    global_root: &std::path::Path,
    selector: &str,
) -> Result<String, OrbitError> {
    let selector = selector.trim();
    if selector.is_empty() {
        return Err(OrbitError::InvalidInput(
            "knowledge workspace selector must not be empty".to_string(),
        ));
    }
    let registry = crate::workspace_registry::load_registry_from(
        &crate::workspace_registry::registry_path_for(global_root),
    )?;
    let workspace = if std::path::Path::new(selector).is_absolute() {
        let selected = std::path::Path::new(selector).canonicalize().map_err(|error| {
            OrbitError::InvalidInput(format!(
                "knowledge workspace path '{selector}' is unavailable: {error}"
            ))
        })?;
        let checkout = crate::workspace_registry::find_checkout_by_path(&registry, &selected)
            .ok_or_else(|| {
                OrbitError::InvalidInput(format!(
                    "knowledge workspace path '{}' is not inside one registered checkout",
                    selected.display()
                ))
            })?;
        registry
            .workspaces
            .iter()
            .find(|workspace| workspace.id == checkout.workspace_id)
    } else {
        if selector.contains('/') || selector == "." || selector == ".." {
            return Err(OrbitError::InvalidInput(format!(
                "knowledge workspace selector '{selector}' must be a stable ID or absolute local path"
            )));
        }
        registry
            .workspaces
            .iter()
            .find(|workspace| workspace.id == selector)
    }
    .ok_or_else(|| {
        OrbitError::InvalidInput(format!("unknown knowledge workspace '{selector}'"))
    })?;
    if workspace.status != WorkspaceStatus::Active {
        return Err(OrbitError::InvalidInput(format!(
            "knowledge workspace '{}' is not active",
            workspace.id
        )));
    }
    Ok(workspace.id.clone())
}

fn spoke_hub_link(
    trusted_config: &config::TrustedMcpConfig,
    identity: &crate::HostIdentity,
    capability: McpCapability,
) -> Result<HubLinkPool, OrbitError> {
    let (route, _) = trusted_config.spoke_route(identity, Some(capability))?;
    let definitions = canonical_mcp_tool_definitions()
        .map_err(|error| OrbitError::InvalidInput(error.to_string()))?;
    let mut schema_digests = BTreeMap::new();
    for allowed in &route.allowed_capabilities {
        schema_digests.insert(*allowed, hub_schema_digest(&definitions, *allowed)?);
    }
    HubLinkPool::ssh(route.host.clone(), route.machine_id.clone(), schema_digests)
}

/// Serve the local broker or fixed coordination hub over MCP stdio.
///
/// This is intentionally independent of Clap so alternate front ends can
/// construct the same trusted host, route, and session boundary.
pub fn serve_mcp_stdio(
    hub: bool,
    requested_capability: Option<McpCapability>,
) -> Result<(), OrbitError> {
    let global_root = resolve_global_root()?;
    // Parse the trusted file, when present, before constructing either server
    // host. Workspace/cwd config never participates in this load.
    let trusted_config = load_trusted_mcp_config(&global_root)?;
    if !hub && requested_capability.is_some() {
        return Err(OrbitError::InvalidInput(
            "--capabilities requires --hub".to_string(),
        ));
    }
    let capability = requested_capability.unwrap_or(McpCapability::Agent);
    let (host, mut trusted_context, composition): (
        Arc<dyn McpHost>,
        ToolSessionContext,
        McpServerComposition,
    ) = if hub {
        let hub = Arc::new(HubMcpHost::new(global_root.clone(), capability)?);
        let identity = hub.identity();
        let context = ToolSessionContext::trusted_local(
            None,
            Some(identity.machine_id.clone()),
            Some(identity.host_id.clone()),
        );
        let composition = hub_server_composition(Arc::clone(&hub));
        (hub, context, composition)
    } else {
        let (host, machine_id, host_id): (Arc<BrokerMcpHost>, Option<String>, Option<String>) =
            match inspect_host_identity(&global_root)? {
                HostIdentityState::Present(identity) => {
                    if identity.mode == HostMode::Spoke {
                        let pool = spoke_hub_link(&trusted_config, &identity, capability)?;
                        (
                            Arc::new(BrokerMcpHost::new_with_hub_link(global_root.clone(), pool)),
                            Some(identity.machine_id),
                            Some(identity.host_id),
                        )
                    } else {
                        (
                            Arc::new(BrokerMcpHost::new(global_root.clone())),
                            Some(identity.machine_id),
                            Some(identity.host_id),
                        )
                    }
                }
                HostIdentityState::Legacy { .. } | HostIdentityState::Absent => (
                    Arc::new(BrokerMcpHost::new(global_root.clone())),
                    None,
                    None,
                ),
            };
        (
            Arc::clone(&host) as Arc<dyn McpHost>,
            ToolSessionContext::trusted_local(None, machine_id, host_id),
            broker_server_composition(host),
        )
    };
    trusted_context.effective_capabilities = BTreeSet::from([capability]);

    let tokio_runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|error| OrbitError::Execution(format!("tokio runtime: {error}")))?;
    tokio_runtime.block_on(orbit_mcp::serve_stdio_with_context_and_composition(
        host,
        trusted_context,
        composition,
    ))
}

fn broker_server_composition(host: Arc<BrokerMcpHost>) -> McpServerComposition {
    let learning_host: Arc<dyn LearningSidecarHost> = host;
    let learning: Arc<dyn McpResultDecorator> =
        Arc::new(LearningSidecarDecorator::from_env(learning_host));
    McpServerComposition::new()
        .with_result_decorator(learning)
        .with_input_schema_resolver(Arc::new(RemoteInputSchemaResolver))
}

fn hub_server_composition(host: Arc<HubMcpHost>) -> McpServerComposition {
    let learning_host: Arc<dyn LearningSidecarHost> = host.clone();
    let learning: Arc<dyn McpResultDecorator> =
        Arc::new(LearningSidecarDecorator::from_env(learning_host));
    McpServerComposition::new()
        .with_result_decorator(learning)
        .with_call_context_resolver(Arc::new(RemoteCallContextResolver))
        .with_input_schema_resolver(Arc::new(RemoteInputSchemaResolver))
        .with_custom_request_handler(Arc::new(PrivateHubRequestHandler::new(Arc::clone(&host))))
        .with_metadata(McpServerMetadata::default().with_instructions(host.private_instructions()))
}
