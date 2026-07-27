//! Shared helpers for the `orbit config` subcommand family: `--scope`
//! resolution and the two well-known config file paths (global/workspace).
//! Domain logic (validation, file I/O) stays in `orbit_core::config`; these
//! helpers only translate CLI flags into `orbit-core` calls.

use std::path::PathBuf;

use clap::ValueEnum;
use orbit_core::config::{ConfigScope, ConfigStore};
use orbit_core::{OrbitError, OrbitRuntime};

/// `--scope` value shared by `orbit config show` and `orbit config get`.
///
/// `Effective` resolves to whichever single file the runtime would actually
/// load under replace-not-merge semantics (see the `orbit_core::config`
/// module doc comment). `Global`/`Workspace` always read that specific file
/// directly, regardless of which one is effective.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum, Default)]
pub enum ConfigScopeArg {
    #[default]
    Effective,
    Global,
    Workspace,
}

/// The global `config.toml` path, whether or not it exists on disk.
pub(super) fn global_config_path(runtime: &OrbitRuntime) -> PathBuf {
    runtime.global_root().join("config.toml")
}

/// The workspace `config.toml` path, whether or not it exists on disk.
pub(super) fn workspace_config_path(runtime: &OrbitRuntime) -> PathBuf {
    runtime.shared_root().join("config.toml")
}

/// Resolve `--scope` into the concrete [`ConfigScope`] and file path it
/// refers to for this runtime.
pub(super) fn resolve_scope(
    runtime: &OrbitRuntime,
    scope: ConfigScopeArg,
) -> (ConfigScope, PathBuf) {
    match scope {
        ConfigScopeArg::Global => (ConfigScope::Global, global_config_path(runtime)),
        ConfigScopeArg::Workspace => (ConfigScope::Workspace, workspace_config_path(runtime)),
        ConfigScopeArg::Effective => {
            let workspace_path = workspace_config_path(runtime);
            let global_path = global_config_path(runtime);
            if workspace_path.exists() && runtime.shared_root() != runtime.global_root() {
                (ConfigScope::Workspace, workspace_path)
            } else {
                (ConfigScope::Global, global_path)
            }
        }
    }
}

/// Open a [`ConfigStore`] for read-only operations (`show`, `get`) at the
/// requested `--scope`.
pub(super) fn open_store_for_scope(
    runtime: &OrbitRuntime,
    scope: ConfigScopeArg,
) -> Result<ConfigStore, OrbitError> {
    let (resolved_scope, path) = resolve_scope(runtime, scope);
    ConfigStore::open(resolved_scope, path)
}
