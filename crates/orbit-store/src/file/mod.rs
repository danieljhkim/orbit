//! File-based store implementations using YAML for human-readable persistence.
//!
//! Each sub-module (`task_store`, `job_store`, `activity_store`, `skill_store`)
//! serializes domain objects to YAML files under a predictable directory layout
//! (e.g., `.orbit/tasks/<id>.yaml`). All writes use
//! [`orbit_common::utility::fs::atomic_write_text_volatile`] to prevent partial writes
//! from corrupting state.

pub(crate) mod diagnostics {
    pub(crate) mod friction_log;
    pub(crate) mod metrics_log;
}
pub(crate) mod executor_def_store;
pub(crate) mod job_store;
pub(crate) mod layout;
pub(crate) mod policy_def_store;
pub(crate) mod scoreboard {
    pub(crate) mod duel_scoreboard;
    pub(crate) mod friction_bounty;
    pub(crate) mod planning_duel_scoreboard;
    pub(crate) mod pr_scoreboard;
    pub(crate) mod scoreboard_summary;
    pub(crate) mod task_review_scoreboard;
    pub(crate) mod token_scoreboard;
}
pub(crate) mod skill_store;
pub(crate) mod sort;
pub(crate) mod task_store;
pub(crate) mod yaml_doc;
