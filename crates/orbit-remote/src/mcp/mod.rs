//! Remote MCP broker, coordination hub, and spoke-link composition.

mod config;
mod host;
mod hub;
mod hub_link;
mod registration;

#[cfg(test)]
mod tests;

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use orbit_common::types::{McpCapability, ToolSessionContext};
use orbit_core::OrbitError;
use orbit_core::runtime::resolve_global_root;
use orbit_mcp::{McpHost, hub_schema_digest};

use crate::{HostIdentityState, HostMode, inspect_host_identity};

use self::config::load_trusted_mcp_config;
use self::host::BrokerMcpHost;
use self::hub::HubMcpHost;
use self::hub_link::HubLinkPool;

pub use self::host::{canonical_mcp_tool_definitions, safe_mcp_tool_names};
pub use self::registration::register_local_spoke;

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
    let (host, mut trusted_context): (Arc<dyn McpHost>, ToolSessionContext) = if hub {
        let hub = HubMcpHost::new(global_root.clone(), capability)?;
        let identity = hub.identity();
        let context = ToolSessionContext::trusted_local(
            None,
            Some(identity.machine_id.clone()),
            Some(identity.host_id.clone()),
        );
        (Arc::new(hub), context)
    } else {
        let (host, machine_id, host_id): (Arc<dyn McpHost>, Option<String>, Option<String>) =
            match inspect_host_identity(&global_root)? {
                HostIdentityState::Present(identity) => {
                    if identity.mode == HostMode::Spoke {
                        let (route, _) = trusted_config.spoke_route(&identity, Some(capability))?;
                        let definitions = canonical_mcp_tool_definitions()
                            .map_err(|error| OrbitError::InvalidInput(error.to_string()))?;
                        let mut schema_digests = BTreeMap::new();
                        for allowed in &route.allowed_capabilities {
                            schema_digests
                                .insert(*allowed, hub_schema_digest(&definitions, *allowed)?);
                        }
                        let pool = HubLinkPool::ssh(
                            route.host.clone(),
                            route.machine_id.clone(),
                            schema_digests,
                        )?;
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
            host,
            ToolSessionContext::trusted_local(None, machine_id, host_id),
        )
    };
    trusted_context.effective_capabilities = BTreeSet::from([capability]);

    let tokio_runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|error| OrbitError::Execution(format!("tokio runtime: {error}")))?;
    tokio_runtime.block_on(orbit_mcp::serve_stdio_with_context(host, trusted_context))
}
