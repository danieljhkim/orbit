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

/// Path-free coordination record for a logical workspace.
#[derive(Debug, Clone)]
pub struct RegisterWorkspaceParams {
    pub workspace_id: String,
    pub slug: String,
    pub repo_fingerprint: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceBinding {
    pub workspace_id: String,
    pub slug: String,
    pub repo_fingerprint: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Machine-local checkout attached to a logical workspace, when one exists.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceCheckoutBinding {
    pub workspace_id: String,
    pub repo_root: PathBuf,
    pub workspace_path: PathBuf,
    pub orbit_dir: PathBuf,
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

/// A relation edge whose target is a valid `ORB-` task id that does not
/// resolve to any registered task bundle in the coordination registry — the
/// exact condition
/// [`validate_relation_targets_exist`](crate::sqlite::task_registry::TaskRegistryStore)
/// rejects at index-rebuild time. These are the "grandfathered" relations that
/// make an index rebuild fail its validator and fall back to a full bundle
/// scan (see ORB-10305 / ORB-10246).
///
/// `relation_type` carries the canonical snake_case name as stored in the
/// registry (`related_to`, `blocked_by`, …); non-`ORB-` targets (friction /
/// learning / ADR ids that `produces`/`resolves` edges legitimately allow to
/// dangle) are never reported here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DanglingRelationTarget {
    pub workspace_id: String,
    pub source_task_id: String,
    pub relation_type: String,
    pub target_task_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectionRebuildResult {
    pub projected: usize,
    pub repaired: usize,
    pub degraded_reason: Option<String>,
}

/// Outcome of seeding the task-id allocator via
/// [`TaskRegistryStore::seed_allocator_start`](crate::sqlite::task_registry::TaskRegistryStore::seed_allocator_start).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AllocatorSeedOutcome {
    /// Counter value before the seed.
    pub previous: u32,
    /// Counter value after the seed (the id the next allocation will hand out).
    pub next: u32,
    /// Whether the seed changed the counter (`false` when it already matched).
    pub changed: bool,
}
