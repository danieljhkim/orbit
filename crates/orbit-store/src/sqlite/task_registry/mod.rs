//! SQLite task-registry storage split into focused schema, query, config, and store modules.
//!
//! `types` contains the public task-registry data structs.
//! `workspace_config` owns YAML workspace config paths, reads, writes, and validation.
//! `workspace_id` derives, validates, and allocates workspace identifiers.
//! `schema` owns SQLite schema setup, migrations, and registry user-version guards.
//! `queries` contains internal SQL helpers and row-to-type mapping.
//! `projection` contains task projection symlink creation and degraded-filesystem handling.
//! `util` contains shared path, time, relation, and WAL helpers used by the registry.
//! `store` contains the `TaskRegistryStore` implementation and transaction orchestration.
//! `tests` contains the registry unit tests; split it further if it grows past the file-size budget.

mod projection;
mod queries;
mod schema;
mod store;
mod types;
mod util;
mod workspace_config;
mod workspace_id;

const CONFIG_SCHEMA_VERSION: u32 = 1;
const REGISTRY_SCHEMA_VERSION: u32 = 3;

pub use store::TaskRegistryStore;
pub use types::{
    BindWorkspaceParams, ProjectionRebuildResult, TaskBundleBinding, TaskIndexFilter,
    WorkspaceBinding, WorkspaceConfig,
};
pub use workspace_config::{
    assign_workspace_id, home_task_workspace_dir, read_workspace_config,
    read_workspace_config_optional, task_registry_path, workspace_config_path,
    workspace_id_for_orbit_dir, write_workspace_config,
};

#[cfg(test)]
mod tests;
