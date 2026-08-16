//! Config layering: global defaults overridden by workspace-local settings.
//!
//! Orbit config is split across two TOML files:
//! - `~/.orbit/config.toml` — global defaults (agent, env passthrough, execution policy)
//! - `.orbit/config.toml` — workspace-local overrides
//!
//! Ordinary settings inherit per key: workspace values override global values, global values fill omissions, and built-in defaults fill remaining gaps.
//! Nested tables layer recursively; a scalar, array, or registered table value in
//! the workspace file replaces the matching global value. Named crew fields also
//! layer recursively, so a workspace can override one model without restating the
//! crew or registry.
//!
//! Three security-sensitive settings are replace-only when a workspace file
//! exists: `execution.codex.sandbox`, `execution.codex.approval_policy`, and
//! `execution.env.pass`. An omitted replace-only setting uses its built-in default
//! rather than inheriting a machine-specific global policy.
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
pub use raw::{RawCrewAssignment, RawCrewEntry};
pub use registry::{
    CONFIG_KEY_REGISTRY, ConfigKeyDescriptor, ConfigSnapshot, describe as describe_config_key,
};
pub(crate) use runtime::{CodexExecutionPolicy, ExecutionEnvPolicy, RuntimeConfig};
pub use runtime::{
    ConfigValueSource, ConfigValueSourceKind, EffectiveConfig, EffectiveConfigValue,
    load_effective_config,
};
pub use store::{ConfigScope, ConfigStore, WorkspaceInitMode};

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
