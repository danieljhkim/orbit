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
use std::net::SocketAddr;
use std::sync::Arc;

use orbit_common::types::{McpCapability, ToolSessionContext};
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

/// Serve the local broker or fixed coordination hub over MCP stdio.
///
/// This is intentionally independent of Clap so alternate front ends can
/// construct the same trusted host, route, and session boundary.
pub fn serve_mcp_stdio(
    hub: bool,
    requested_capability: Option<McpCapability>,
) -> Result<(), OrbitError> {
    let (host, trusted_context, composition) = build_mcp_session(hub, requested_capability)?;
    let tokio_runtime = build_tokio_runtime()?;
    tokio_runtime.block_on(orbit_mcp::serve_stdio_with_context_and_composition(
        host,
        trusted_context,
        composition,
    ))
}

/// Serve the local broker or fixed coordination hub over MCP-over-TCP.
///
/// The endpoint carries no authentication of its own (see
/// [`orbit_mcp::McpTcpServer`]); ADR-0350 authenticates it by requiring an SSH
/// tunnel to reach it rather than by anything Orbit adds. That posture only
/// holds if the listener itself refuses a routable bind, so the loopback
/// check runs before the socket opens — the same guard
/// `orbit-dashboard::check_bindable_host` applies, reused here rather than
/// re-derived because both guards protect an unauthenticated listener the
/// same way.
pub fn serve_mcp_tcp(
    addr: SocketAddr,
    hub: bool,
    requested_capability: Option<McpCapability>,
) -> Result<(), OrbitError> {
    check_bindable_mcp_host(addr)?;
    let (host, trusted_context, composition) = build_mcp_session(hub, requested_capability)?;
    let tokio_runtime = build_tokio_runtime()?;
    tokio_runtime.block_on(orbit_mcp::serve_tcp_with_context_and_composition(
        addr,
        host,
        trusted_context,
        composition,
    ))
}

/// Reject binding the MCP TCP listener to anything other than a loopback
/// address, before the socket is opened.
///
/// SECURITY: the listener carries no authentication of its own. A non-loopback
/// bind would turn it into unauthenticated remote control of the machine, so
/// this refuses eagerly rather than documenting the risk. For remote access,
/// bind loopback and reach it through an authenticated tunnel (e.g. `ssh -L`),
/// which is exactly the posture ADR-0350 commits to.
fn check_bindable_mcp_host(addr: SocketAddr) -> Result<(), OrbitError> {
    if addr.ip().is_loopback() {
        return Ok(());
    }
    Err(OrbitError::InvalidInput(format!(
        "refusing to bind the MCP TCP listener to non-loopback address {addr}: the listener is \
         unauthenticated and relies on an SSH tunnel for access control (ADR-0350). Bind a \
         loopback address (127.0.0.1 or ::1) and use an authenticated tunnel/reverse proxy \
         (e.g. `ssh -L {port}:localhost:{port} <host>`) for remote access.",
        port = addr.port()
    )))
}

/// Resolve the effective capability for a session that did not request one.
///
/// A deployment that says nothing must not become an operator: [`McpCapability::Agent`]
/// is the capability nothing else grants on its own (see
/// `orbit_common::authorization`'s governed-operation table, which never lists
/// `Agent` alone), so it is the floor every unspecified session gets — for
/// both stdio and TCP.
fn least_privileged_mcp_capability(requested: Option<McpCapability>) -> McpCapability {
    requested.unwrap_or(McpCapability::Agent)
}

fn build_tokio_runtime() -> Result<tokio::runtime::Runtime, OrbitError> {
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|error| OrbitError::Execution(format!("tokio runtime: {error}")))
}

/// Build the trusted host, session template, and server composition shared by
/// every MCP transport. The effective capability is resolved here so stdio
/// and TCP agree on the same least-privileged default.
fn build_mcp_session(
    hub: bool,
    requested_capability: Option<McpCapability>,
) -> Result<(Arc<dyn McpHost>, ToolSessionContext, McpServerComposition), OrbitError> {
    let global_root = resolve_global_root()?;
    // Parse the trusted file, when present, before constructing either server
    // host. Workspace/cwd config never participates in this load.
    let trusted_config = load_trusted_mcp_config(&global_root)?;
    let capability = least_privileged_mcp_capability(requested_capability);
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
            Arc::clone(&host) as Arc<dyn McpHost>,
            ToolSessionContext::trusted_local(None, machine_id, host_id),
            broker_server_composition(host),
        )
    };
    trusted_context.effective_capabilities = BTreeSet::from([capability]);

    Ok((host, trusted_context, composition))
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
