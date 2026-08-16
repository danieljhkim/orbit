#![deny(clippy::print_stderr, clippy::print_stdout)]
#![allow(missing_docs)]
#![cfg_attr(test, allow(clippy::expect_used, clippy::unwrap_used))]
//! Machine identity and workspace registry domain for Orbit.
//!
//! This crate owns host identity, the logical workspace catalog and local
//! checkout bindings, and their file persistence and validation. It contains
//! no command orchestration, MCP transport, Core runtime execution, or shared
//! database access.

pub mod host_identity;
pub mod workspace_registry;

#[cfg(test)]
mod tests;

pub use host_identity::{
    HOST_IDENTITY_SCHEMA_VERSION, HOST_TOML_FILE, HostIdentity, HostIdentityOutcome,
    HostIdentityState, NewHostIdentity, ensure_host_identity, inspect_host_identity,
    load_host_identity, os_hostname, rename_current_host_identity, validate_new_task_prefix,
};
