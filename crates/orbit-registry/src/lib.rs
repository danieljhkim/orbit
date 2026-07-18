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
//! Machine and workspace registry domain for Orbit.
//!
//! This crate owns strict machine identity, the path-free logical workspace
//! catalog plus machine-local checkout roles, the atomic satellite cache, and
//! the store-backed host/workspace registry service. Shared DTOs remain in
//! `orbit-common`; SQL, migrations, revision advancement, and transactional
//! snapshot queries remain in `orbit-store`.

pub mod host_identity;
pub mod host_registry;
pub mod registry_cache;
pub mod workspace_registry;

pub use host_identity::{
    HOST_IDENTITY_SCHEMA_VERSION, HOST_TOML_FILE, HostIdentity, HostIdentityOutcome,
    HostIdentityState, HostMode, NewHostIdentity, ensure_host_identity, inspect_host_identity,
    load_host_identity, os_hostname, rename_current_host_identity,
};
pub use host_registry::{HostRegistryService, WorkspaceLink, require_local_hub_identity};
pub use registry_cache::{RegistryCacheOutcome, RegistryCacheService, RegistryCacheState};
