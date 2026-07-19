#![deny(clippy::print_stderr, clippy::print_stdout)]
// ORB-00004: legacy registry surfaces still need a focused documentation pass.
#![allow(missing_docs)]
// ORB-00013: Unit tests use unwrap/expect for fixture setup; production call sites remain linted.
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
//! host/workspace registry service, and remote MCP broker/hub/link composition.
//! Shared DTOs remain in `orbit-common`; generic MCP framing remains in
//! `orbit-mcp`; registry persistence, feature migrations, revision advancement,
//! and transactional snapshot queries are encapsulated here over `orbit-store`'s
//! generic SQLite connection infrastructure.

pub mod host_identity;
pub mod host_registry;
pub mod mcp;
pub mod persistence;
pub mod profile;
pub mod registry_cache;
pub mod routines;
pub mod runtime;
pub mod service;
mod tools;
pub mod workspace_registry;

pub use host_identity::{
    HOST_IDENTITY_SCHEMA_VERSION, HOST_TOML_FILE, HostIdentity, HostIdentityOutcome,
    HostIdentityState, HostMode, NewHostIdentity, ensure_host_identity, inspect_host_identity,
    load_host_identity, os_hostname, rename_current_host_identity,
};
pub use host_registry::{HostRegistryService, WorkspaceLink, require_local_hub_identity};
pub use mcp::{
    canonical_mcp_tool_definitions, register_local_spoke, safe_mcp_tool_names, serve_mcp_stdio,
};
pub use persistence::RemoteStore;
pub use profile::build_execution_profile_v1;
pub use registry_cache::{RegistryCacheOutcome, RegistryCacheService, RegistryCacheState};
pub use service::{
    host_registry_service_at, record_global_audit_event_at, registry_snapshot_at, remote_store_at,
};
