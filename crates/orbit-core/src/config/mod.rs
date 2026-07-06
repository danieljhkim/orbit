//! Config layering: global defaults overridden by workspace-local settings.
//!
//! Orbit config is split across two TOML files:
//! - `~/.orbit/config.toml` — global defaults (agent, env passthrough, execution policy)
//! - `.orbit/config.toml` — workspace-local overrides
//!
//! **Merge semantics are replace-not-merge**: if a workspace config specifies a
//! key, it completely replaces the global value for that key. There is no deep
//! merge of nested structures. This avoids surprising implicit inheritance while
//! still letting workspaces stay minimal by omitting keys they don't need to change.
//!
//! The `bootstrap` module seeds a default `config.toml` on first `orbit init`.
//! The `raw` module holds the serde-deserializable structs.
//! The `persistence` and `runtime` modules derive strongly-typed config views.

pub mod agent_detect;
pub mod agent_prompt;
mod bootstrap;
mod persistence;
mod raw;
mod registry;
mod runtime;
mod store;

pub(crate) use bootstrap::seed_default_config;
pub(crate) use persistence::PersistenceConfig;
pub use raw::{RawAgentRoleConfig, RawCrewEntry};
pub(crate) use raw::{RawQaCheckConfig, RawQaConfig, RawQaWorkspaceConfig};
pub use registry::{CONFIG_KEY_REGISTRY, ConfigKeyDescriptor, describe as describe_config_key};
pub(crate) use runtime::{CodexExecutionPolicy, DuelConfig, ExecutionEnvPolicy, RuntimeConfig};
pub use store::{ConfigScope, ConfigSnapshot, ConfigStore, WorkspaceInitMode};

/// Validate the effective (workspace-over-global) `config.toml` without
/// exposing the internal `RuntimeConfig` shape. `pub` for the workspace
/// doctor in `orbit-cmd` [ORB-10016].
pub fn validate_layered_config(
    global_root: &std::path::Path,
    data_root: &std::path::Path,
) -> Result<(), orbit_common::types::OrbitError> {
    RuntimeConfig::load_layered(global_root, data_root).map(|_| ())
}

/// Store-database path resolved from the layered config. `pub` for the
/// runtime-less `orbit migrate --dry-run` inspection in `orbit-cmd`
/// [ORB-10016].
pub fn resolved_audit_db_path(
    global_root: &std::path::Path,
    orbit_dir: &std::path::Path,
) -> Result<std::path::PathBuf, orbit_common::types::OrbitError> {
    Ok(RuntimeConfig::load_layered(global_root, orbit_dir)?
        .persistence
        .audit_db)
}

#[cfg(test)]
mod tests;
