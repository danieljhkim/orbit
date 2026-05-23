use std::path::PathBuf;

use chrono::{DateTime, Utc};
use orbit_common::types::{TaskPriority, TaskStatus};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceConfig {
    pub schema_version: u32,
    pub workspace_id: String,
}

#[derive(Debug, Clone)]
pub struct BindWorkspaceParams {
    pub workspace_id: Option<String>,
    pub slug: String,
    pub repo_root: PathBuf,
    pub workspace_path: PathBuf,
    pub orbit_dir: PathBuf,
    pub repo_fingerprint: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceBinding {
    pub workspace_id: String,
    pub slug: String,
    pub repo_root: PathBuf,
    pub workspace_path: PathBuf,
    pub orbit_dir: PathBuf,
    pub repo_fingerprint: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskBundleBinding {
    pub task_id: String,
    pub workspace_id: String,
    pub canonical_path: PathBuf,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskIndexFilter {
    pub status: Option<TaskStatus>,
    pub priority: Option<TaskPriority>,
    pub job_run_id: Option<String>,
    pub tags: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectionRebuildResult {
    pub projected: usize,
    pub repaired: usize,
    pub degraded_reason: Option<String>,
}
