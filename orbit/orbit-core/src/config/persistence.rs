use std::path::{Path, PathBuf};

use orbit_types::WorkspacePaths;
use serde_json::{Value, json};

/// Holds the resolved paths for all persistent artifact stores.
///
/// Each resource has at most a workspace path and a global path.
/// - Tasks: workspace only
/// - Activities/Jobs: workspace + global (layered reads merge both)
/// - Skills: workspace + global (workspace replaces global when present)
/// - Audit: global only (single SQLite database)
#[derive(Debug, Clone)]
pub(crate) struct PersistenceConfig {
    pub(crate) task_dir: PathBuf,
    pub(crate) activity_dir: PathBuf,
    pub(crate) job_dir: PathBuf,
    pub(crate) skill_dir: PathBuf,
    pub(crate) audit_db: PathBuf,
    pub(crate) global_activity_dir: PathBuf,
    pub(crate) global_job_dir: PathBuf,
    pub(crate) global_skill_dir: PathBuf,
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
        let has_workspace = paths.global_dir != paths.orbit_dir;

        let global_activity_dir = paths.global_dir.join("activities");
        let global_job_dir = paths.global_dir.join("jobs");
        let global_skill_dir = paths.global_dir.join("skills");

        if has_workspace {
            Self {
                task_dir: paths.tasks_dir.clone(),
                activity_dir: paths.activities_dir.clone(),
                job_dir: paths.jobs_dir.clone(),
                skill_dir: paths.skills_dir.clone(),
                audit_db: paths.global_dir.join("orbit.db"),
                global_activity_dir,
                global_job_dir,
                global_skill_dir,
            }
        } else {
            // Single-root mode: workspace paths equal global paths.
            Self {
                task_dir: paths.global_dir.join("tasks"),
                activity_dir: global_activity_dir.clone(),
                job_dir: global_job_dir.clone(),
                skill_dir: global_skill_dir.clone(),
                audit_db: paths.global_dir.join("orbit.db"),
                global_activity_dir,
                global_job_dir,
                global_skill_dir,
            }
        }
    }

    pub(crate) fn as_json_value(&self) -> Value {
        json!({
            "task": { "path": self.task_dir.to_string_lossy() },
            "activity": { "path": self.activity_dir.to_string_lossy(), "global_path": self.global_activity_dir.to_string_lossy() },
            "job": { "path": self.job_dir.to_string_lossy(), "global_path": self.global_job_dir.to_string_lossy() },
            "skill": { "path": self.skill_dir.to_string_lossy() },
            "audit": { "path": self.audit_db.to_string_lossy() },
        })
    }
}
