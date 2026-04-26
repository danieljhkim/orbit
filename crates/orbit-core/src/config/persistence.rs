use std::path::{Path, PathBuf};

use orbit_common::types::WorkspacePaths;
use serde_json::{Value, json};

/// Holds the resolved paths for all persistent artifact stores.
///
/// - Tasks: workspace only
/// - Skills: workspace override directory layered over global defaults
/// - Activities/Jobs/Executors/Policies: global only
/// - Audit: global only (single SQLite database)
#[derive(Debug, Clone)]
pub(crate) struct PersistenceConfig {
    pub(crate) task_dir: PathBuf,
    pub(crate) activity_dir: PathBuf,
    pub(crate) job_dir: PathBuf,
    pub(crate) skill_dir: PathBuf,
    pub(crate) executor_dir: PathBuf,
    pub(crate) audit_db: PathBuf,
    pub(crate) policy_dir: PathBuf,
}

impl PersistenceConfig {
    pub(crate) fn default_for_data_root(data_root: &Path) -> Self {
        Self::default_for_roots(data_root, data_root)
    }

    /// Two-root defaults (raw paths). Delegates to [`Self::from_workspace_paths`].
    pub(crate) fn default_for_roots(global_root: &Path, workspace_root: &Path) -> Self {
        let repo_root = workspace_root
            .parent()
            .unwrap_or(workspace_root)
            .to_path_buf();
        let paths = WorkspacePaths::new(
            repo_root,
            workspace_root.to_path_buf(),
            global_root.to_path_buf(),
        );
        Self::from_workspace_paths(&paths)
    }

    /// Build persistence config from [`WorkspacePaths`]. This is the **single
    /// source of truth** for artifact path resolution.
    pub(crate) fn from_workspace_paths(paths: &WorkspacePaths) -> Self {
        let global_resources_dir = paths.global_dir.join("resources");

        Self {
            task_dir: paths.tasks_dir.clone(),
            activity_dir: global_resources_dir.join("activities"),
            job_dir: global_resources_dir.join("jobs"),
            skill_dir: paths.skills_dir.clone(),
            executor_dir: global_resources_dir.join("executors"),
            policy_dir: global_resources_dir.join("policies"),
            audit_db: paths.global_dir.join("orbit.db"),
        }
    }

    pub(crate) fn as_json_value(&self) -> Value {
        json!({
            "task": { "path": self.task_dir.to_string_lossy() },
            "activity": { "path": self.activity_dir.to_string_lossy() },
            "job": { "path": self.job_dir.to_string_lossy() },
            "skill": { "path": self.skill_dir.to_string_lossy() },
            "executor": { "path": self.executor_dir.to_string_lossy() },
            "policy": { "path": self.policy_dir.to_string_lossy() },
            "audit": { "path": self.audit_db.to_string_lossy() },
        })
    }
}
