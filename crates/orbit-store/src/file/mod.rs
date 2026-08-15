//! File-based store implementations for human-readable persistence.
//!
//! Most artifact stores use YAML under predictable directories. The session
//! log uses JSON Lines and a stable advisory-lock sidecar because appends and
//! resolutions must remain ordered across processes.

pub(crate) mod executor_def_store;
pub(crate) mod friction_store;
pub(crate) mod path_safety;
pub(crate) mod policy_def_store;
pub(crate) mod scoreboard;
pub(crate) mod session_log_store;
pub(crate) mod skill_store;
pub(crate) mod sort;
pub(crate) mod task_store;
pub(crate) mod yaml_doc;
