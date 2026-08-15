#![deny(clippy::print_stderr, clippy::print_stdout)]
#![allow(missing_docs)]
#![cfg_attr(test, allow(clippy::expect_used, clippy::unwrap_used))]
//! Machine identity and workspace registry domain for Orbit.
//!
//! This crate owns host identity, the logical workspace catalog and local
//! checkout bindings, their file persistence and validation, and the durable
//! host registry/cache. It contains no command orchestration, MCP transport, or
//! Core runtime execution.

pub mod host_identity;
pub mod host_registry;
pub mod persistence;
pub mod registry_cache;
pub mod service;
pub mod workspace_registry;

#[cfg(test)]
mod tests;

pub use host_identity::{
    HOST_IDENTITY_SCHEMA_VERSION, HOST_TOML_FILE, HostIdentity, HostIdentityOutcome,
    HostIdentityState, HostMode, NewHostIdentity, ensure_host_identity, inspect_host_identity,
    load_host_identity, os_hostname, rename_current_host_identity, validate_new_task_prefix,
};
pub use host_registry::{HostRegistryService, WorkspaceLink, require_local_hub_identity};
pub use persistence::RegistryStore;
pub use registry_cache::{RegistryCacheOutcome, RegistryCacheService, RegistryCacheState};
pub use service::host_registry_service;
