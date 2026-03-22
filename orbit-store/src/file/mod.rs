//! File-based store implementations using YAML for human-readable persistence.
//!
//! Each sub-module (`task_store`, `job_store`, `activity_store`, `skill_store`)
//! serializes domain objects to YAML files under a predictable directory layout
//! (e.g., `.orbit/tasks/<id>.yaml`). All writes use an atomic write strategy:
//! data is first written to a `.tmp` file via `fs_utils::write_atomic`, then
//! renamed into place to prevent partial writes from corrupting state.
//!
//! The `fs_utils` sub-module provides the `write_atomic` helper shared by all
//! file-based stores.

pub(crate) mod activity_store;
pub(crate) mod friction_bounty;
pub(crate) mod friction_log;
pub(crate) mod fs_utils;
pub(crate) mod job_store;
pub(crate) mod metrics_log;
pub(crate) mod skill_store;
pub(crate) mod task_store;
