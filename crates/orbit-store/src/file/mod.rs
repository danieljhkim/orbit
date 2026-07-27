//! File-based store implementations using YAML for human-readable persistence.
//!
//! Each sub-module (`task_store`, `activity_store`, `skill_store`)
//! serializes domain objects to YAML files under a predictable directory layout
//! (e.g., `.orbit/tasks/<id>.yaml`). All writes use
//! [`orbit_common::utility::fs::atomic_write_text_volatile`] to prevent partial writes
//! from corrupting state.

pub(crate) mod adr_store;
pub(crate) mod executor_def_store;
pub(crate) mod friction_store;
pub(crate) mod learning_store;
pub(crate) mod path_safety;
pub(crate) mod policy_def_store;
pub(crate) mod scoreboard;
pub(crate) mod skill_store;
pub(crate) mod sort;
pub(crate) mod task_store;
pub(crate) mod yaml_doc;
