//! The federated MCP namespace: a mux over operator-configured SSH destinations.
//!
//! This module is the one place in Orbit where an MCP server also acts as an
//! MCP *client*. It exists so a caller can see every configured destination's
//! workspaces as one session-unbound list of live descriptors, each carrying
//! the host-qualified selector a later routed call will address. v1's
//! byte-transparent SSH proxy ([`crate::serve_mcp_remote_proxy`]) is unchanged
//! and remains the path for a caller that has already chosen one host.
//!
//! The mux is deliberately not a fleet registry: membership comes only from
//! the operator's [`DESTINATIONS_FILE`], never from this machine's workspace
//! registry, and no destination's answer is cached between calls.

mod capability;
mod config;
mod descriptor;
mod host;
mod probe;

#[cfg(test)]
mod tests;

pub use self::capability::{
    CapabilityClasses, McpToolClass, ensure_tool_class_held, mcp_tool_class,
};
pub use self::config::{
    DESTINATIONS_FILE, Destination, DestinationsFile, HostQualifiedSelector, destinations_path,
    load_destinations,
};
pub use self::descriptor::{Capability, CheckoutHealth, Reachability, WorkspaceDescriptor};
pub use self::host::{FEDERATED_WORKSPACE_LIST_TOOL, FederatedMcpHost};
pub use self::probe::{
    DEFAULT_PROBE_TIMEOUT, DEFAULT_ROUTED_DELIVERY_TIMEOUT, DestinationProbe, DestinationSnapshot,
    RoutedSession, SshDestinationProbe,
};
