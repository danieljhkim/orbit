#![deny(clippy::print_stderr, clippy::print_stdout)]
// Internal feature surfaces still need a focused documentation pass.
#![allow(missing_docs)]
// Unit tests use unwrap/expect for fixture setup; production call sites remain linted.
#![cfg_attr(test, allow(clippy::expect_used, clippy::unwrap_used))]
#![allow(
    rustdoc::broken_intra_doc_links,
    rustdoc::invalid_html_tags,
    rustdoc::private_intra_doc_links
)]
//! Remote execution, coordination, and machine/workspace registry domain for Orbit.
//!
//! This crate owns strict machine identity, the path-free logical workspace
//! catalog plus machine-local checkout roles, the atomic satellite cache, the
//! host/workspace registry service, registry persistence, and the thin remote
//! MCP proxy plus machine-local identity and discovery support.
//! Shared DTOs remain in `orbit-common`; generic MCP framing remains in
//! `orbit-mcp`; generic workspace-scoped builtin definitions remain in
//! `orbit-tools`; registry migrations, revision advancement, and transactional
//! snapshot queries are encapsulated here over `orbit-store`'s generic SQLite
//! connection and namespaced migration-ledger infrastructure.

pub mod host_identity;
pub mod host_registry;
pub mod mcp;
pub mod persistence;
pub mod registry_cache;
pub mod service;
pub mod workspace_registry;

#[cfg(test)]
mod tests;

pub use host_identity::{
    HostIdentity, HostIdentityOutcome, HostIdentityState, HostMode, NewHostIdentity,
    ensure_host_identity, inspect_host_identity, load_host_identity, os_hostname,
    rename_current_host_identity,
};
pub use host_registry::{HostRegistryService, WorkspaceLink, require_local_hub_identity};
pub use mcp::{
    McpServerIdentity, RemoteProxyArgs, canonical_mcp_tool_definitions, execute_discovery_tool,
    mcp_server_identity, safe_mcp_tool_names, serve_mcp_remote_proxy,
};
pub use persistence::RemoteStore;
pub use registry_cache::{RegistryCacheOutcome, RegistryCacheService, RegistryCacheState};
pub use service::host_registry_service;
