//! Remote MCP broker, owner endpoint, and owner-route composition.

mod config;
mod contract;
mod discovery;
mod host;
mod owner;
mod owner_client;
mod owner_link;
mod proxy;
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
use orbit_mcp::{McpHost, McpServerComposition, McpServerMetadata};

use crate::{HostIdentityState, inspect_host_identity};

use self::config::{TrustedMcpConfig, load_trusted_mcp_config};
use self::contract::owner_schema_digest;
use self::host::{BrokerMcpHost, OwnerRoute, OwnerRouteTable};
use self::owner::OwnerMcpHost;
use self::owner_link::OwnerLinkPool;
use self::schema::RemoteInputSchemaResolver;
use self::transport::RemoteCallContextResolver;

pub use self::host::{canonical_mcp_tool_definitions, safe_mcp_tool_names};
pub use self::proxy::{DEFAULT_REMOTE_MCP_PORT, RemoteProxyArgs, serve_mcp_remote_proxy};

/// Which server this invocation presents.
///
/// ORB-10727 [ADR-0355]: this is no longer a client-supplied `--hub` flag. The
/// same `orbit mcp serve` command starts a local broker for an interactive
/// client and the owner endpoint when a remote client reaches this machine over
/// the fixed SSH command, so the role travels with the transport, not with
/// machine-level configuration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum McpServerRole {
    /// Client-facing local broker: resolves placement and routes to owners.
    Broker,
    /// Checkoutless owner endpoint for the workspaces this machine owns.
    Owner,
}

/// Serve the local broker or the owner endpoint over MCP stdio.
///
/// This is intentionally independent of Clap so alternate front ends can
/// construct the same trusted host, route, and session boundary.
pub fn serve_mcp_stdio(
    role: McpServerRole,
    requested_capability: Option<McpCapability>,
) -> Result<(), OrbitError> {
    let (host, trusted_context, composition) = build_mcp_session(role, requested_capability)?;
    let tokio_runtime = build_tokio_runtime()?;
    tokio_runtime.block_on(orbit_mcp::serve_stdio_with_context_and_composition(
        host,
        trusted_context,
        composition,
    ))
}

/// Serve the local broker or the owner endpoint over MCP-over-TCP.
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
    role: McpServerRole,
    requested_capability: Option<McpCapability>,
) -> Result<(), OrbitError> {
    check_bindable_mcp_host(addr)?;
    let (host, trusted_context, composition) = build_mcp_session(role, requested_capability)?;
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
fn least_privileged_mcp_capability(
    requested: Option<McpCapability>,
) -> Result<McpCapability, OrbitError> {
    let capability = requested.unwrap_or(McpCapability::Agent);
    // ADR-0358 withdrew `runner` from the bridge: no tool policy allows it, so
    // such a session would advertise an empty surface. Refuse it by name rather
    // than serving nothing.
    if !capability.is_bridge_v1() {
        return Err(OrbitError::InvalidInput(format!(
            "MCP capability '{capability}' is withdrawn from the v1 bridge (ADR-0358); request agent or operator"
        )));
    }
    Ok(capability)
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
    role: McpServerRole,
    requested_capability: Option<McpCapability>,
) -> Result<(Arc<dyn McpHost>, ToolSessionContext, McpServerComposition), OrbitError> {
    let global_root = resolve_global_root()?;
    // Parse the trusted file, when present, before constructing either server
    // host. Workspace/cwd config never participates in this load.
    let trusted_config = load_trusted_mcp_config(&global_root)?;
    let capability = least_privileged_mcp_capability(requested_capability)?;
    let (host, mut trusted_context, composition): (
        Arc<dyn McpHost>,
        ToolSessionContext,
        McpServerComposition,
    ) = match role {
        McpServerRole::Owner => {
            let owner = Arc::new(OwnerMcpHost::new(global_root.clone(), capability)?);
            let identity = owner.identity();
            let context = ToolSessionContext::trusted_local(
                None,
                Some(identity.machine_id.clone()),
                Some(identity.host_id.clone()),
            );
            let composition = owner_server_composition(Arc::clone(&owner));
            (owner, context, composition)
        }
        McpServerRole::Broker => {
            let (machine_id, host_id) = match inspect_host_identity(&global_root)? {
                HostIdentityState::Present(identity) => {
                    (Some(identity.machine_id), Some(identity.host_id))
                }
                HostIdentityState::Legacy { .. } | HostIdentityState::Absent => (None, None),
            };
            let routes = owner_route_table(&trusted_config, machine_id.as_deref())?;
            let host = Arc::new(BrokerMcpHost::new_with_owner_routes(
                global_root.clone(),
                routes,
            ));
            (
                Arc::clone(&host) as Arc<dyn McpHost>,
                ToolSessionContext::trusted_local(None, machine_id, host_id),
                broker_server_composition(host),
            )
        }
    };
    trusted_context.effective_capabilities = BTreeSet::from([capability]);

    Ok((host, trusted_context, composition))
}

/// Open one bounded link pool per configured owner machine.
///
/// Routes are per machine, not per workspace: a machine holding replica
/// checkouts of workspaces owned by several others names each owner once. A
/// route pointing at this machine is a configuration error, not a loopback —
/// an owned workspace short-circuits locally and never opens a link.
fn owner_route_table(
    trusted_config: &TrustedMcpConfig,
    local_machine_id: Option<&str>,
) -> Result<OwnerRouteTable, OrbitError> {
    let definitions = canonical_mcp_tool_definitions()
        .map_err(|error| OrbitError::InvalidInput(error.to_string()))?;
    let mut table = BTreeMap::new();
    for route in trusted_config.routes() {
        if local_machine_id == Some(route.machine_id.as_str()) {
            return Err(OrbitError::InvalidInput(format!(
                "trusted MCP config names an [[owner]] route to this machine ('{}'); a machine reaches its own workspaces in process",
                route.machine_id
            )));
        }
        let mut schema_digests = BTreeMap::new();
        for allowed in &route.allowed_capabilities {
            schema_digests.insert(*allowed, owner_schema_digest(&definitions, *allowed)?);
        }
        let pool =
            OwnerLinkPool::ssh(route.host.clone(), route.machine_id.clone(), schema_digests)?;
        table.insert(
            route.machine_id.clone(),
            OwnerRoute {
                allowed_capabilities: route.allowed_capabilities.clone(),
                pool: Arc::new(pool),
            },
        );
    }
    Ok(table)
}

fn broker_server_composition(_host: Arc<BrokerMcpHost>) -> McpServerComposition {
    McpServerComposition::new().with_input_schema_resolver(Arc::new(RemoteInputSchemaResolver))
}

fn owner_server_composition(host: Arc<OwnerMcpHost>) -> McpServerComposition {
    McpServerComposition::new()
        .with_call_context_resolver(Arc::new(RemoteCallContextResolver))
        .with_input_schema_resolver(Arc::new(RemoteInputSchemaResolver))
        .with_metadata(McpServerMetadata::default().with_instructions(host.contract_instructions()))
}
