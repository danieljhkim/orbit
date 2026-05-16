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
mod runtime;

pub(crate) use bootstrap::seed_default_config;
pub(crate) use persistence::PersistenceConfig;
pub use raw::{RawAgentRoleConfig, RawCrewEntry};
pub(crate) use runtime::{CodexExecutionPolicy, DuelConfig, ExecutionEnvPolicy, RuntimeConfig};
