use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, Mutex};

use chrono::{DateTime, Utc};
use orbit_common::types::{
    ORB_TASK_ID_MAX, OrbitError, TaskEnvelopeV2, TaskPriority, TaskRelationType, TaskStatus,
    format_orb_task_id, normalize_task_tags, validate_orb_task_id,
};
use orbit_common::utility::fs::{atomic_write_text, create_dir_symlink};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params, params_from_iter};
use serde::{Deserialize, Serialize};

const CONFIG_SCHEMA_VERSION: u32 = 1;
const REGISTRY_SCHEMA_VERSION: u32 = 2;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceConfig {
    pub schema_version: u32,
    pub workspace_id: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct WorkspaceConfigDoc {
    schema_version: u32,
    workspace_id: String,
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
    pub tags: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectionRebuildResult {
    pub projected: usize,
    pub repaired: usize,
    pub degraded_reason: Option<String>,
}

#[derive(Clone)]
pub struct TaskRegistryStore {
    conn: Arc<Mutex<Connection>>,
    workspaces_dir: PathBuf,
}

impl TaskRegistryStore {
    pub fn open(path: &Path) -> Result<Self, OrbitError> {
        let registry_dir = path
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| PathBuf::from("."));
        let workspaces_dir = normalize_path(&registry_dir.join("workspaces"));
        fs::create_dir_all(&registry_dir).map_err(|e| OrbitError::Store(e.to_string()))?;
        fs::create_dir_all(&workspaces_dir).map_err(|e| OrbitError::Store(e.to_string()))?;

        let conn = Connection::open(path).map_err(|e| OrbitError::Store(e.to_string()))?;
        enable_best_effort_wal_mode(&conn);
        conn.pragma_update(None, "busy_timeout", "5000")
            .map_err(|e| OrbitError::Store(format!("failed to set busy_timeout: {e}")))?;
        conn.pragma_update(None, "foreign_keys", "ON")
            .map_err(|e| OrbitError::Store(format!("failed to enable foreign keys: {e}")))?;
        reject_unsupported_registry_schema(&conn)?;
        apply_schema(&conn)?;
        assert_registry_user_version(&conn)?;

        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
            workspaces_dir,
        })
    }

    pub fn bind_workspace(
        &self,
        params: BindWorkspaceParams,
    ) -> Result<WorkspaceBinding, OrbitError> {
        let repo_root = normalize_path(&params.repo_root);
        let workspace_path = normalize_path(&params.workspace_path);
        let orbit_dir = normalize_path(&params.orbit_dir);
        let slug = sanitize_slug(&params.slug);
        let requested_workspace_id = params
            .workspace_id
            .as_deref()
            .map(validate_workspace_id)
            .transpose()?;

        let mut conn = self
            .conn
            .lock()
            .map_err(|e| OrbitError::Store(format!("mutex poisoned: {e}")))?;
        let tx = conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|e| OrbitError::Store(e.to_string()))?;

        if let Some(existing) = workspace_by_orbit_dir(&tx, &orbit_dir)? {
            if let Some(requested) = &requested_workspace_id {
                if requested != &existing.workspace_id {
                    return Err(OrbitError::InvalidInput(format!(
                        "orbit dir '{}' is already bound to workspace '{}', not '{}'",
                        orbit_dir.display(),
                        existing.workspace_id,
                        requested
                    )));
                }
            }
            tx.commit().map_err(|e| OrbitError::Store(e.to_string()))?;
            return Ok(existing);
        }

        let workspace_id = match requested_workspace_id {
            Some(id) => id,
            None => next_workspace_id_candidate(&tx, &slug, &workspace_path)?,
        };
        if workspace_by_id(&tx, &workspace_id)?.is_some() {
            return Err(OrbitError::Store(format!(
                "workspace id '{workspace_id}' is already bound to a different orbit dir"
            )));
        }

        let now = now_string();
        tx.execute(
            "INSERT INTO workspace_bindings (
                workspace_id, slug, repo_root, workspace_path, orbit_dir, repo_fingerprint,
                created_at, updated_at
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?7)",
            params![
                workspace_id,
                slug,
                path_to_string(&repo_root),
                path_to_string(&workspace_path),
                path_to_string(&orbit_dir),
                params.repo_fingerprint,
                now,
            ],
        )
        .map_err(|e| OrbitError::Store(e.to_string()))?;

        let binding = workspace_by_id(&tx, &workspace_id)?
            .ok_or_else(|| OrbitError::Store("failed to read inserted workspace binding".into()))?;
        tx.commit().map_err(|e| OrbitError::Store(e.to_string()))?;
        Ok(binding)
    }

    /// Allocate a monotonic local task ID.
    ///
    /// Allocation commits independently from bundle registration. A crash between
    /// allocation and registration can leave numeric holes; those holes are expected
    /// and are not reused.
    pub fn allocate_task_id(&self, workspace_id: &str) -> Result<String, OrbitError> {
        let workspace_id = validate_workspace_id(workspace_id)?;
        let mut conn = self
            .conn
            .lock()
            .map_err(|e| OrbitError::Store(format!("mutex poisoned: {e}")))?;
        let tx = conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|e| OrbitError::Store(e.to_string()))?;

        if workspace_by_id(&tx, &workspace_id)?.is_none() {
            return Err(OrbitError::WorkspaceNotFound(workspace_id));
        }

        let next: i64 = tx
            .query_row(
                "SELECT next_number FROM allocator_state WHERE authority = 'local'",
                [],
                |row| row.get(0),
            )
            .map_err(|e| OrbitError::Store(e.to_string()))?;
        if next > i64::from(ORB_TASK_ID_MAX) {
            return Err(OrbitError::Store("ORB task id allocator exhausted".into()));
        }
        tx.execute(
            "UPDATE allocator_state SET next_number = ?1, updated_at = ?2 WHERE authority = 'local'",
            params![next + 1, now_string()],
        )
        .map_err(|e| OrbitError::Store(e.to_string()))?;
        tx.commit().map_err(|e| OrbitError::Store(e.to_string()))?;

        let next = u32::try_from(next).map_err(|e| OrbitError::Store(e.to_string()))?;
        format_orb_task_id(next)
    }

    pub fn canonical_task_bundle_path(
        &self,
        workspace_id: &str,
        task_id: &str,
    ) -> Result<PathBuf, OrbitError> {
        let workspace_id = validate_workspace_id(workspace_id)?;
        validate_orb_task_id(task_id)?;
        Ok(self.workspaces_dir.join(workspace_id).join(task_id))
    }

    pub fn register_task_bundle(
        &self,
        task_id: &str,
        workspace_id: &str,
        canonical_path: &Path,
    ) -> Result<TaskBundleBinding, OrbitError> {
        validate_orb_task_id(task_id)?;
        let workspace_id = validate_workspace_id(workspace_id)?;
        let canonical_path = normalize_path(canonical_path);
        let expected_path =
            normalize_path(&self.canonical_task_bundle_path(&workspace_id, task_id)?);
        if canonical_path != expected_path {
            return Err(OrbitError::InvalidInput(format!(
                "canonical path for task '{task_id}' in workspace '{workspace_id}' must be '{}', got '{}'",
                expected_path.display(),
                canonical_path.display()
            )));
        }

        let mut conn = self
            .conn
            .lock()
            .map_err(|e| OrbitError::Store(format!("mutex poisoned: {e}")))?;
        let tx = conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|e| OrbitError::Store(e.to_string()))?;

        if workspace_by_id(&tx, &workspace_id)?.is_none() {
            return Err(OrbitError::WorkspaceNotFound(workspace_id));
        }

        let now = now_string();
        tx.execute(
            "INSERT INTO task_bundle_bindings (
                task_id, workspace_id, canonical_path, created_at, updated_at
            ) VALUES (?1, ?2, ?3, ?4, ?4)
            ON CONFLICT(task_id) DO UPDATE SET
                workspace_id = excluded.workspace_id,
                canonical_path = excluded.canonical_path,
                updated_at = excluded.updated_at",
            params![task_id, workspace_id, path_to_string(&canonical_path), now],
        )
        .map_err(|e| OrbitError::Store(e.to_string()))?;

        let binding = task_bundle_by_id(&tx, task_id)?.ok_or_else(|| {
            OrbitError::Store("failed to read inserted task bundle binding".into())
        })?;
        tx.commit().map_err(|e| OrbitError::Store(e.to_string()))?;
        Ok(binding)
    }

    pub fn unregister_task_bundle(
        &self,
        task_id: &str,
        workspace_id: &str,
    ) -> Result<bool, OrbitError> {
        validate_orb_task_id(task_id)?;
        let workspace_id = validate_workspace_id(workspace_id)?;
        let mut conn = self
            .conn
            .lock()
            .map_err(|e| OrbitError::Store(format!("mutex poisoned: {e}")))?;
        let tx = conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|e| OrbitError::Store(e.to_string()))?;

        tx.execute(
            "DELETE FROM task_bundle_relations
             WHERE source_task_id = ?1 OR target_task_id = ?1",
            [task_id],
        )
        .map_err(|e| OrbitError::Store(e.to_string()))?;
        tx.execute("DELETE FROM task_bundle_tags WHERE task_id = ?1", [task_id])
            .map_err(|e| OrbitError::Store(e.to_string()))?;
        tx.execute(
            "DELETE FROM task_bundle_index WHERE task_id = ?1",
            [task_id],
        )
        .map_err(|e| OrbitError::Store(e.to_string()))?;
        let deleted = tx
            .execute(
                "DELETE FROM task_bundle_bindings
                 WHERE task_id = ?1 AND workspace_id = ?2",
                params![task_id, workspace_id],
            )
            .map_err(|e| OrbitError::Store(e.to_string()))?;
        tx.commit().map_err(|e| OrbitError::Store(e.to_string()))?;
        Ok(deleted > 0)
    }

    pub fn tasks_for_workspace(
        &self,
        workspace_id: &str,
    ) -> Result<Vec<TaskBundleBinding>, OrbitError> {
        let workspace_id = validate_workspace_id(workspace_id)?;
        let conn = self
            .conn
            .lock()
            .map_err(|e| OrbitError::Store(format!("mutex poisoned: {e}")))?;
        let mut stmt = conn
            .prepare(
                "SELECT task_id, workspace_id, canonical_path, created_at, updated_at
                 FROM task_bundle_bindings
                 WHERE workspace_id = ?1
                 ORDER BY task_id ASC",
            )
            .map_err(|e| OrbitError::Store(e.to_string()))?;
        let rows = stmt
            .query_map([workspace_id], decode_task_bundle_binding)
            .map_err(|e| OrbitError::Store(e.to_string()))?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|e| OrbitError::Store(e.to_string()))
    }

    pub fn replace_task_index(
        &self,
        workspace_id: &str,
        envelope: &TaskEnvelopeV2,
    ) -> Result<(), OrbitError> {
        let workspace_id = validate_workspace_id(workspace_id)?;
        envelope.validate()?;

        let mut conn = self
            .conn
            .lock()
            .map_err(|e| OrbitError::Store(format!("mutex poisoned: {e}")))?;
        let tx = conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|e| OrbitError::Store(e.to_string()))?;

        let binding = task_bundle_by_id(&tx, &envelope.id)?
            .ok_or_else(|| OrbitError::TaskNotFound(envelope.id.clone()))?;
        if binding.workspace_id != workspace_id {
            return Err(OrbitError::InvalidInput(format!(
                "task '{}' is registered to workspace '{}', not '{}'",
                envelope.id, binding.workspace_id, workspace_id
            )));
        }

        tx.execute(
            "DELETE FROM task_bundle_tags WHERE task_id = ?1",
            [&envelope.id],
        )
        .map_err(|e| OrbitError::Store(e.to_string()))?;
        tx.execute(
            "DELETE FROM task_bundle_relations WHERE source_task_id = ?1",
            [&envelope.id],
        )
        .map_err(|e| OrbitError::Store(e.to_string()))?;

        write_task_index_rows(&tx, &workspace_id, envelope)?;
        tx.commit().map_err(|e| OrbitError::Store(e.to_string()))
    }

    pub fn replace_workspace_task_indexes(
        &self,
        workspace_id: &str,
        envelopes: &[TaskEnvelopeV2],
    ) -> Result<(), OrbitError> {
        let workspace_id = validate_workspace_id(workspace_id)?;
        for envelope in envelopes {
            envelope.validate()?;
        }

        let mut conn = self
            .conn
            .lock()
            .map_err(|e| OrbitError::Store(format!("mutex poisoned: {e}")))?;
        let tx = conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|e| OrbitError::Store(e.to_string()))?;

        let registered = task_ids_for_workspace(&tx, &workspace_id)?;
        let requested = envelopes
            .iter()
            .map(|envelope| envelope.id.clone())
            .collect::<BTreeSet<_>>();
        if registered != requested {
            return Err(OrbitError::Store(format!(
                "task index rebuild for workspace '{}' expected registered ids {:?}, got {:?}",
                workspace_id, registered, requested
            )));
        }

        tx.execute(
            "DELETE FROM task_bundle_tags WHERE workspace_id = ?1",
            [&workspace_id],
        )
        .map_err(|e| OrbitError::Store(e.to_string()))?;
        tx.execute(
            "DELETE FROM task_bundle_relations WHERE workspace_id = ?1",
            [&workspace_id],
        )
        .map_err(|e| OrbitError::Store(e.to_string()))?;
        tx.execute(
            "DELETE FROM task_bundle_index WHERE workspace_id = ?1",
            [&workspace_id],
        )
        .map_err(|e| OrbitError::Store(e.to_string()))?;

        for envelope in envelopes {
            write_task_index_rows(&tx, &workspace_id, envelope)?;
        }
        tx.commit().map_err(|e| OrbitError::Store(e.to_string()))
    }

    pub fn indexed_task_versions_for_workspace(
        &self,
        workspace_id: &str,
    ) -> Result<BTreeMap<String, String>, OrbitError> {
        let workspace_id = validate_workspace_id(workspace_id)?;
        let conn = self
            .conn
            .lock()
            .map_err(|e| OrbitError::Store(format!("mutex poisoned: {e}")))?;
        let mut stmt = conn
            .prepare(
                "SELECT task_id, updated_at FROM task_bundle_index
                 WHERE workspace_id = ?1
                 ORDER BY task_id ASC",
            )
            .map_err(|e| OrbitError::Store(e.to_string()))?;
        let rows = stmt
            .query_map([workspace_id], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })
            .map_err(|e| OrbitError::Store(e.to_string()))?;
        rows.collect::<Result<BTreeMap<_, _>, _>>()
            .map_err(|e| OrbitError::Store(e.to_string()))
    }

    pub fn indexed_task_count_for_workspace(
        &self,
        workspace_id: &str,
    ) -> Result<usize, OrbitError> {
        let workspace_id = validate_workspace_id(workspace_id)?;
        let conn = self
            .conn
            .lock()
            .map_err(|e| OrbitError::Store(format!("mutex poisoned: {e}")))?;
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM task_bundle_index WHERE workspace_id = ?1",
                [workspace_id],
                |row| row.get(0),
            )
            .map_err(|e| OrbitError::Store(e.to_string()))?;
        usize::try_from(count).map_err(|e| OrbitError::Store(e.to_string()))
    }

    pub fn indexed_task_ids_filtered(
        &self,
        workspace_id: &str,
        filter: &TaskIndexFilter,
    ) -> Result<Vec<String>, OrbitError> {
        let workspace_id = validate_workspace_id(workspace_id)?;
        let required_tags = normalize_task_tags(filter.tags.clone());
        let mut sql = String::from("SELECT task_id FROM task_bundle_index WHERE workspace_id = ?");
        let mut values = vec![workspace_id.clone()];
        if let Some(status) = filter.status {
            sql.push_str(" AND status = ?");
            values.push(status.to_string());
        }
        if let Some(priority) = filter.priority {
            sql.push_str(" AND priority = ?");
            values.push(priority.to_string());
        }
        sql.push_str(" ORDER BY created_at DESC, task_id ASC");

        let conn = self
            .conn
            .lock()
            .map_err(|e| OrbitError::Store(format!("mutex poisoned: {e}")))?;
        let mut stmt = conn
            .prepare(&sql)
            .map_err(|e| OrbitError::Store(e.to_string()))?;
        let rows = stmt
            .query_map(params_from_iter(values.iter()), |row| {
                row.get::<_, String>(0)
            })
            .map_err(|e| OrbitError::Store(e.to_string()))?;
        let mut ids = rows
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| OrbitError::Store(e.to_string()))?;

        if required_tags.is_empty() {
            return Ok(ids);
        }

        let mut tag_sets = Vec::new();
        let mut tag_stmt = conn
            .prepare(
                "SELECT task_id FROM task_bundle_tags
                 WHERE workspace_id = ?1 AND tag = ?2
                 ORDER BY task_id ASC",
            )
            .map_err(|e| OrbitError::Store(e.to_string()))?;
        for tag in required_tags {
            let rows = tag_stmt
                .query_map(params![&workspace_id, &tag], |row| row.get::<_, String>(0))
                .map_err(|e| OrbitError::Store(e.to_string()))?;
            let set = rows
                .collect::<Result<BTreeSet<_>, _>>()
                .map_err(|e| OrbitError::Store(e.to_string()))?;
            tag_sets.push(set);
        }

        ids.retain(|id| tag_sets.iter().all(|set| set.contains(id)));
        Ok(ids)
    }

    pub fn indexed_relation_targets(
        &self,
        workspace_id: &str,
        source_task_id: &str,
        relation_type: TaskRelationType,
    ) -> Result<Vec<String>, OrbitError> {
        let workspace_id = validate_workspace_id(workspace_id)?;
        validate_orb_task_id(source_task_id)?;
        let conn = self
            .conn
            .lock()
            .map_err(|e| OrbitError::Store(format!("mutex poisoned: {e}")))?;
        let mut stmt = conn
            .prepare(
                "SELECT target_task_id FROM task_bundle_relations
                 WHERE workspace_id = ?1 AND source_task_id = ?2 AND relation_type = ?3
                 ORDER BY target_task_id ASC",
            )
            .map_err(|e| OrbitError::Store(e.to_string()))?;
        let rows = stmt
            .query_map(
                params![
                    workspace_id,
                    source_task_id,
                    relation_type_name(relation_type)
                ],
                |row| row.get::<_, String>(0),
            )
            .map_err(|e| OrbitError::Store(e.to_string()))?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|e| OrbitError::Store(e.to_string()))
    }

    pub fn indexed_relation_sources(
        &self,
        workspace_id: &str,
        target_task_id: &str,
        relation_type: TaskRelationType,
    ) -> Result<Vec<String>, OrbitError> {
        let workspace_id = validate_workspace_id(workspace_id)?;
        validate_orb_task_id(target_task_id)?;
        let conn = self
            .conn
            .lock()
            .map_err(|e| OrbitError::Store(format!("mutex poisoned: {e}")))?;
        let mut stmt = conn
            .prepare(
                "SELECT source_task_id FROM task_bundle_relations
                 WHERE workspace_id = ?1 AND target_task_id = ?2 AND relation_type = ?3
                 ORDER BY source_task_id ASC",
            )
            .map_err(|e| OrbitError::Store(e.to_string()))?;
        let rows = stmt
            .query_map(
                params![
                    workspace_id,
                    target_task_id,
                    relation_type_name(relation_type)
                ],
                |row| row.get(0),
            )
            .map_err(|e| OrbitError::Store(e.to_string()))?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|e| OrbitError::Store(e.to_string()))
    }

    pub fn find_rebind_candidates(
        &self,
        repo_root: &Path,
        workspace_path: &Path,
        orbit_dir: &Path,
    ) -> Result<Vec<WorkspaceBinding>, OrbitError> {
        let repo_root = normalize_path(repo_root);
        let workspace_path = normalize_path(workspace_path);
        let orbit_dir = normalize_path(orbit_dir);
        let conn = self
            .conn
            .lock()
            .map_err(|e| OrbitError::Store(format!("mutex poisoned: {e}")))?;
        let mut stmt = conn
            .prepare(
                "SELECT workspace_id, slug, repo_root, workspace_path, orbit_dir,
                    repo_fingerprint, created_at, updated_at
                 FROM workspace_bindings
                 WHERE repo_root = ?1 OR workspace_path = ?2 OR orbit_dir = ?3
                 ORDER BY updated_at DESC, workspace_id ASC",
            )
            .map_err(|e| OrbitError::Store(e.to_string()))?;
        let rows = stmt
            .query_map(
                params![
                    path_to_string(&repo_root),
                    path_to_string(&workspace_path),
                    path_to_string(&orbit_dir),
                ],
                decode_workspace_binding,
            )
            .map_err(|e| OrbitError::Store(e.to_string()))?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|e| OrbitError::Store(e.to_string()))
    }

    pub fn rebuild_projection(
        &self,
        workspace_orbit_dir: &Path,
        workspace_id: &str,
    ) -> Result<ProjectionRebuildResult, OrbitError> {
        let workspace_id = validate_workspace_id(workspace_id)?;
        let projection_dir = workspace_orbit_dir.join("tasks");
        fs::create_dir_all(&projection_dir).map_err(|e| OrbitError::Io(e.to_string()))?;

        let tasks = self.tasks_for_workspace(&workspace_id)?;
        let mut result = ProjectionRebuildResult {
            projected: 0,
            repaired: 0,
            degraded_reason: None,
        };

        for task in tasks {
            let link_path = projection_dir.join(&task.task_id);
            let target = task.canonical_path;
            match fs::symlink_metadata(&link_path) {
                Ok(metadata) if metadata.file_type().is_symlink() => {
                    let current =
                        fs::read_link(&link_path).map_err(|e| OrbitError::Io(e.to_string()))?;
                    if normalize_path(&current) != normalize_path(&target) {
                        fs::remove_file(&link_path).map_err(|e| OrbitError::Io(e.to_string()))?;
                        create_projection_symlink(&target, &link_path, &mut result)?;
                        result.repaired += 1;
                    } else {
                        result.projected += 1;
                    }
                }
                Ok(_) => {
                    return Err(OrbitError::Store(format!(
                        "projection entry '{}' already exists and is not a symlink",
                        link_path.display()
                    )));
                }
                Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                    create_projection_symlink(&target, &link_path, &mut result)?;
                }
                Err(err) => return Err(OrbitError::Io(err.to_string())),
            }

            if result.degraded_reason.is_some() {
                return Ok(result);
            }
        }

        Ok(result)
    }
}

pub fn task_registry_path(global_root: &Path) -> PathBuf {
    global_root.join("tasks").join("index.sqlite")
}

pub fn home_task_workspace_dir(global_root: &Path, workspace_id: &str) -> PathBuf {
    global_root
        .join("tasks")
        .join("workspaces")
        .join(workspace_id)
}

pub fn workspace_config_path(orbit_dir: &Path) -> PathBuf {
    orbit_dir.join("config.yaml")
}

pub fn read_workspace_config(orbit_dir: &Path) -> Result<WorkspaceConfig, OrbitError> {
    read_workspace_config_optional(orbit_dir)?.ok_or_else(|| {
        OrbitError::InvalidInput(format!(
            "workspace config is missing: {}",
            workspace_config_path(orbit_dir).display()
        ))
    })
}

pub fn read_workspace_config_optional(
    orbit_dir: &Path,
) -> Result<Option<WorkspaceConfig>, OrbitError> {
    let path = workspace_config_path(orbit_dir);
    let raw = match fs::read_to_string(&path) {
        Ok(raw) => raw,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(err) => return Err(OrbitError::Io(err.to_string())),
    };
    let doc: WorkspaceConfigDoc = serde_yaml::from_str(&raw).map_err(|e| {
        OrbitError::InvalidInput(format!(
            "invalid workspace config '{}': {e}",
            path.display()
        ))
    })?;
    validate_workspace_config_doc(doc).map(Some)
}

pub fn write_workspace_config(
    orbit_dir: &Path,
    config: &WorkspaceConfig,
) -> Result<(), OrbitError> {
    let workspace_id = validate_workspace_id(&config.workspace_id)?;
    if config.schema_version != CONFIG_SCHEMA_VERSION {
        return Err(OrbitError::InvalidInput(format!(
            "unsupported workspace config schema_version {}",
            config.schema_version
        )));
    }

    let doc = WorkspaceConfigDoc {
        schema_version: CONFIG_SCHEMA_VERSION,
        workspace_id,
    };
    let content = serde_yaml::to_string(&doc).map_err(|e| OrbitError::Store(e.to_string()))?;
    atomic_write_text(&workspace_config_path(orbit_dir), &content)
        .map_err(|e| OrbitError::Io(e.to_string()))
}

pub fn assign_workspace_id(slug_source: &str, path: &Path) -> String {
    workspace_id_candidate(&sanitize_slug(slug_source), path, 0)
}

fn validate_workspace_config_doc(doc: WorkspaceConfigDoc) -> Result<WorkspaceConfig, OrbitError> {
    if doc.schema_version != CONFIG_SCHEMA_VERSION {
        return Err(OrbitError::InvalidInput(format!(
            "unsupported workspace config schema_version {}",
            doc.schema_version
        )));
    }
    Ok(WorkspaceConfig {
        schema_version: doc.schema_version,
        workspace_id: validate_workspace_id(&doc.workspace_id)?,
    })
}

fn apply_schema(conn: &Connection) -> Result<(), OrbitError> {
    conn.execute_batch(
        "
        CREATE TABLE IF NOT EXISTS allocator_state (
            authority TEXT PRIMARY KEY,
            next_number INTEGER NOT NULL CHECK(next_number >= 0 AND next_number <= 100000),
            updated_at TEXT NOT NULL
        );

        CREATE TABLE IF NOT EXISTS workspace_bindings (
            workspace_id TEXT PRIMARY KEY,
            slug TEXT NOT NULL,
            repo_root TEXT NOT NULL,
            workspace_path TEXT NOT NULL,
            orbit_dir TEXT NOT NULL UNIQUE,
            repo_fingerprint TEXT,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_workspace_bindings_paths
            ON workspace_bindings(repo_root, workspace_path, orbit_dir);

        CREATE TABLE IF NOT EXISTS task_bundle_bindings (
            task_id TEXT PRIMARY KEY,
            workspace_id TEXT NOT NULL,
            canonical_path TEXT NOT NULL,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL,
            FOREIGN KEY(workspace_id) REFERENCES workspace_bindings(workspace_id) ON DELETE CASCADE
        );
        CREATE INDEX IF NOT EXISTS idx_task_bundle_bindings_workspace
            ON task_bundle_bindings(workspace_id, task_id);

        CREATE TABLE IF NOT EXISTS task_bundle_index (
            task_id TEXT PRIMARY KEY,
            workspace_id TEXT NOT NULL,
            status TEXT NOT NULL,
            priority TEXT NOT NULL,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL,
            terminal_month TEXT,
            FOREIGN KEY(task_id) REFERENCES task_bundle_bindings(task_id) ON DELETE CASCADE,
            FOREIGN KEY(workspace_id) REFERENCES workspace_bindings(workspace_id) ON DELETE CASCADE
        );
        CREATE INDEX IF NOT EXISTS idx_task_bundle_index_workspace_created
            ON task_bundle_index(workspace_id, created_at DESC, task_id ASC);
        CREATE INDEX IF NOT EXISTS idx_task_bundle_index_workspace_status
            ON task_bundle_index(workspace_id, status, created_at DESC, task_id ASC);
        CREATE INDEX IF NOT EXISTS idx_task_bundle_index_workspace_priority
            ON task_bundle_index(workspace_id, priority, created_at DESC, task_id ASC);
        CREATE INDEX IF NOT EXISTS idx_task_bundle_index_workspace_terminal
            ON task_bundle_index(workspace_id, terminal_month, task_id);

        CREATE TABLE IF NOT EXISTS task_bundle_tags (
            task_id TEXT NOT NULL,
            workspace_id TEXT NOT NULL,
            tag TEXT NOT NULL,
            PRIMARY KEY(task_id, tag),
            FOREIGN KEY(task_id) REFERENCES task_bundle_bindings(task_id) ON DELETE CASCADE,
            FOREIGN KEY(workspace_id) REFERENCES workspace_bindings(workspace_id) ON DELETE CASCADE
        );
        CREATE INDEX IF NOT EXISTS idx_task_bundle_tags_workspace_tag
            ON task_bundle_tags(workspace_id, tag, task_id);

        CREATE TABLE IF NOT EXISTS task_bundle_relations (
            source_task_id TEXT NOT NULL,
            workspace_id TEXT NOT NULL,
            relation_type TEXT NOT NULL,
            target_task_id TEXT NOT NULL,
            PRIMARY KEY(source_task_id, relation_type, target_task_id),
            FOREIGN KEY(source_task_id) REFERENCES task_bundle_bindings(task_id) ON DELETE CASCADE,
            FOREIGN KEY(workspace_id) REFERENCES workspace_bindings(workspace_id) ON DELETE CASCADE
        );
        CREATE INDEX IF NOT EXISTS idx_task_bundle_relations_workspace_type_target
            ON task_bundle_relations(workspace_id, relation_type, target_task_id, source_task_id);
        ",
    )
    .map_err(|e| OrbitError::Store(e.to_string()))?;

    conn.execute(
        "INSERT OR IGNORE INTO allocator_state(authority, next_number, updated_at)
         VALUES ('local', 0, ?1)",
        [now_string()],
    )
    .map_err(|e| OrbitError::Store(e.to_string()))?;
    conn.pragma_update(None, "user_version", i64::from(REGISTRY_SCHEMA_VERSION))
        .map_err(|e| OrbitError::Store(format!("failed to set registry user_version: {e}")))?;
    Ok(())
}

fn reject_unsupported_registry_schema(conn: &Connection) -> Result<(), OrbitError> {
    let version = registry_user_version(conn)?;
    if version > REGISTRY_SCHEMA_VERSION {
        return Err(OrbitError::Store(format!(
            "task registry schema version {version} is newer than supported version {REGISTRY_SCHEMA_VERSION}"
        )));
    }
    Ok(())
}

fn registry_user_version(conn: &Connection) -> Result<u32, OrbitError> {
    let version: i64 = conn
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .map_err(|e| OrbitError::Store(format!("failed to read registry user_version: {e}")))?;
    u32::try_from(version)
        .map_err(|e| OrbitError::Store(format!("invalid registry user_version {version}: {e}")))
}

fn assert_registry_user_version(conn: &Connection) -> Result<(), OrbitError> {
    let version = registry_user_version(conn)?;
    if version != REGISTRY_SCHEMA_VERSION {
        return Err(OrbitError::Store(format!(
            "task registry schema version {version} did not match expected version {REGISTRY_SCHEMA_VERSION}"
        )));
    }
    Ok(())
}

fn workspace_by_orbit_dir(
    conn: &Connection,
    orbit_dir: &Path,
) -> Result<Option<WorkspaceBinding>, OrbitError> {
    conn.query_row(
        "SELECT workspace_id, slug, repo_root, workspace_path, orbit_dir,
            repo_fingerprint, created_at, updated_at
         FROM workspace_bindings WHERE orbit_dir = ?1",
        [path_to_string(orbit_dir)],
        decode_workspace_binding,
    )
    .optional()
    .map_err(|e| OrbitError::Store(e.to_string()))
}

fn workspace_by_id(
    conn: &Connection,
    workspace_id: &str,
) -> Result<Option<WorkspaceBinding>, OrbitError> {
    conn.query_row(
        "SELECT workspace_id, slug, repo_root, workspace_path, orbit_dir,
            repo_fingerprint, created_at, updated_at
         FROM workspace_bindings WHERE workspace_id = ?1",
        [workspace_id],
        decode_workspace_binding,
    )
    .optional()
    .map_err(|e| OrbitError::Store(e.to_string()))
}

fn task_bundle_by_id(
    conn: &Connection,
    task_id: &str,
) -> Result<Option<TaskBundleBinding>, OrbitError> {
    conn.query_row(
        "SELECT task_id, workspace_id, canonical_path, created_at, updated_at
         FROM task_bundle_bindings WHERE task_id = ?1",
        [task_id],
        decode_task_bundle_binding,
    )
    .optional()
    .map_err(|e| OrbitError::Store(e.to_string()))
}

fn task_ids_for_workspace(
    conn: &Connection,
    workspace_id: &str,
) -> Result<BTreeSet<String>, OrbitError> {
    let mut stmt = conn
        .prepare(
            "SELECT task_id FROM task_bundle_bindings
             WHERE workspace_id = ?1
             ORDER BY task_id ASC",
        )
        .map_err(|e| OrbitError::Store(e.to_string()))?;
    let rows = stmt
        .query_map([workspace_id], |row| row.get::<_, String>(0))
        .map_err(|e| OrbitError::Store(e.to_string()))?;
    rows.collect::<Result<BTreeSet<_>, _>>()
        .map_err(|e| OrbitError::Store(e.to_string()))
}

fn write_task_index_rows(
    tx: &rusqlite::Transaction<'_>,
    workspace_id: &str,
    envelope: &TaskEnvelopeV2,
) -> Result<(), OrbitError> {
    tx.execute(
        "INSERT INTO task_bundle_index (
            task_id, workspace_id, status, priority, created_at, updated_at, terminal_month
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
        ON CONFLICT(task_id) DO UPDATE SET
            workspace_id = excluded.workspace_id,
            status = excluded.status,
            priority = excluded.priority,
            created_at = excluded.created_at,
            updated_at = excluded.updated_at,
            terminal_month = excluded.terminal_month",
        params![
            &envelope.id,
            workspace_id,
            envelope.status.to_string(),
            envelope.priority.to_string(),
            envelope.created_at.to_rfc3339(),
            envelope.updated_at.to_rfc3339(),
            terminal_month(envelope.status, envelope.updated_at),
        ],
    )
    .map_err(|e| OrbitError::Store(e.to_string()))?;

    for tag in normalize_task_tags(envelope.tags.clone()) {
        tx.execute(
            "INSERT OR IGNORE INTO task_bundle_tags(task_id, workspace_id, tag)
             VALUES (?1, ?2, ?3)",
            params![&envelope.id, workspace_id, &tag],
        )
        .map_err(|e| OrbitError::Store(e.to_string()))?;
    }

    for relation in &envelope.relations {
        tx.execute(
            "INSERT OR IGNORE INTO task_bundle_relations(
                source_task_id, workspace_id, relation_type, target_task_id
            ) VALUES (?1, ?2, ?3, ?4)",
            params![
                &envelope.id,
                workspace_id,
                relation_type_name(relation.relation_type),
                &relation.target
            ],
        )
        .map_err(|e| OrbitError::Store(e.to_string()))?;
    }

    Ok(())
}

fn next_workspace_id_candidate(
    conn: &Connection,
    slug: &str,
    path: &Path,
) -> Result<String, OrbitError> {
    for attempt in 0..1000 {
        let candidate = workspace_id_candidate(slug, path, attempt);
        if workspace_by_id(conn, &candidate)?.is_none() {
            return Ok(candidate);
        }
    }
    Err(OrbitError::Store(format!(
        "could not allocate workspace id for slug '{slug}'"
    )))
}

fn workspace_id_candidate(slug: &str, path: &Path, attempt: u32) -> String {
    let input = format!("{}:{}:{attempt}", slug, normalize_path(path).display());
    let hash = blake3::hash(input.as_bytes()).to_hex();
    format!("{slug}-{}", &hash[..6])
}

fn sanitize_slug(raw: &str) -> String {
    let mut out = String::new();
    let mut last_dash = false;
    for ch in raw.chars().flat_map(char::to_lowercase) {
        if ch.is_ascii_alphanumeric() {
            out.push(ch);
            last_dash = false;
        } else if !last_dash && !out.is_empty() {
            out.push('-');
            last_dash = true;
        }
    }
    while out.ends_with('-') {
        out.pop();
    }
    if out.is_empty() {
        "workspace".to_string()
    } else {
        out
    }
}

fn validate_workspace_id(raw: &str) -> Result<String, OrbitError> {
    let trimmed = raw.trim();
    let Some((slug, suffix)) = trimmed.rsplit_once('-') else {
        return Err(OrbitError::InvalidInput(format!(
            "workspace_id '{trimmed}' must use <slug>-<6char> form"
        )));
    };
    if !is_valid_workspace_slug(slug)
        || suffix.len() != 6
        || !suffix
            .as_bytes()
            .iter()
            .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
    {
        return Err(OrbitError::InvalidInput(format!(
            "workspace_id '{trimmed}' must use <slug>-<6char> form"
        )));
    }
    Ok(trimmed.to_string())
}

fn is_valid_workspace_slug(slug: &str) -> bool {
    if slug.is_empty() || slug.starts_with('-') || slug.ends_with('-') || slug.contains("--") {
        return false;
    }
    slug.as_bytes()
        .iter()
        .all(|byte| matches!(byte, b'a'..=b'z' | b'0'..=b'9' | b'-'))
}

fn create_projection_symlink(
    target: &Path,
    link_path: &Path,
    result: &mut ProjectionRebuildResult,
) -> Result<(), OrbitError> {
    match create_dir_symlink(target, link_path) {
        Ok(()) => {
            result.projected += 1;
            Ok(())
        }
        Err(err) if is_symlink_degraded_error(&err) => {
            result.degraded_reason = Some(format!(
                "directory symlinks are unavailable for '{}': {err}",
                link_path.display()
            ));
            Ok(())
        }
        Err(err) => Err(OrbitError::Io(err.to_string())),
    }
}

fn is_symlink_degraded_error(err: &std::io::Error) -> bool {
    matches!(
        err.kind(),
        std::io::ErrorKind::PermissionDenied | std::io::ErrorKind::Unsupported
    )
}

fn normalize_path(path: &Path) -> PathBuf {
    if let Ok(canonical) = path.canonicalize() {
        return canonical;
    }

    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                normalized.pop();
            }
            other => normalized.push(other.as_os_str()),
        }
    }
    normalized
}

fn path_to_string(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

fn now_string() -> String {
    Utc::now().to_rfc3339()
}

fn terminal_month(status: TaskStatus, updated_at: DateTime<Utc>) -> Option<String> {
    matches!(
        status,
        TaskStatus::Done | TaskStatus::Archived | TaskStatus::Rejected
    )
    .then(|| updated_at.format("%Y-%m").to_string())
}

fn relation_type_name(relation_type: TaskRelationType) -> &'static str {
    match relation_type {
        TaskRelationType::Blocks => "blocks",
        TaskRelationType::ParentOf => "parent_of",
        TaskRelationType::SpawnedFrom => "spawned_from",
        TaskRelationType::RegressionFrom => "regression_from",
        TaskRelationType::Supersedes => "supersedes",
        TaskRelationType::RelatedTo => "related_to",
    }
}

fn parse_timestamp(raw: &str) -> rusqlite::Result<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(raw)
        .map(|parsed| parsed.with_timezone(&Utc))
        .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))
}

fn decode_workspace_binding(row: &rusqlite::Row<'_>) -> rusqlite::Result<WorkspaceBinding> {
    Ok(WorkspaceBinding {
        workspace_id: row.get(0)?,
        slug: row.get(1)?,
        repo_root: PathBuf::from(row.get::<_, String>(2)?),
        workspace_path: PathBuf::from(row.get::<_, String>(3)?),
        orbit_dir: PathBuf::from(row.get::<_, String>(4)?),
        repo_fingerprint: row.get(5)?,
        created_at: parse_timestamp(&row.get::<_, String>(6)?)?,
        updated_at: parse_timestamp(&row.get::<_, String>(7)?)?,
    })
}

fn decode_task_bundle_binding(row: &rusqlite::Row<'_>) -> rusqlite::Result<TaskBundleBinding> {
    Ok(TaskBundleBinding {
        task_id: row.get(0)?,
        workspace_id: row.get(1)?,
        canonical_path: PathBuf::from(row.get::<_, String>(2)?),
        created_at: parse_timestamp(&row.get::<_, String>(3)?)?,
        updated_at: parse_timestamp(&row.get::<_, String>(4)?)?,
    })
}

fn enable_best_effort_wal_mode(conn: &Connection) {
    if let Err(err) =
        conn.pragma_update_and_check(None, "journal_mode", "WAL", |row| row.get::<_, String>(0))
    {
        orbit_common::tracing::warn!(
            target: "orbit.store.task_registry",
            error = %err,
            "could not set WAL mode on the task registry database; continuing with the default journal mode",
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use orbit_common::types::{TaskRelation, TaskType};
    use tempfile::TempDir;

    fn registry_path(temp: &TempDir) -> PathBuf {
        task_registry_path(temp.path())
    }

    fn store(temp: &TempDir) -> TaskRegistryStore {
        TaskRegistryStore::open(&registry_path(temp)).expect("open registry")
    }

    fn bind(store: &TaskRegistryStore, root: &Path) -> WorkspaceBinding {
        let orbit_dir = root.join(".orbit");
        fs::create_dir_all(&orbit_dir).expect("create orbit dir");
        store
            .bind_workspace(BindWorkspaceParams {
                workspace_id: Some("orbit-test-123456".into()),
                slug: "Orbit Test".into(),
                repo_root: root.to_path_buf(),
                workspace_path: root.to_path_buf(),
                orbit_dir,
                repo_fingerprint: None,
            })
            .expect("bind workspace")
    }

    fn create_canonical_bundle(
        store: &TaskRegistryStore,
        workspace: &WorkspaceBinding,
        task_id: &str,
    ) -> PathBuf {
        let bundle_dir = store
            .canonical_task_bundle_path(&workspace.workspace_id, task_id)
            .expect("canonical bundle path");
        fs::create_dir_all(&bundle_dir).expect("create bundle");
        bundle_dir
    }

    fn envelope(
        task_id: &str,
        status: TaskStatus,
        tags: Vec<String>,
        relations: Vec<TaskRelation>,
    ) -> TaskEnvelopeV2 {
        let now = Utc.with_ymd_and_hms(2026, 5, 11, 12, 0, 0).unwrap();
        TaskEnvelopeV2 {
            schema_version: orbit_common::types::TASK_ARTIFACT_SCHEMA_VERSION,
            id: task_id.to_string(),
            title: format!("Task {task_id}"),
            status,
            task_type: TaskType::Feature,
            priority: TaskPriority::High,
            complexity: None,
            relations,
            tags,
            context_files: Vec::new(),
            external_refs: Vec::new(),
            created_by: Some("codex:gpt-5.5".to_string()),
            planned_by: None,
            implemented_by: None,
            created_at: now,
            updated_at: now,
        }
    }

    fn projection_links_supported(result: &ProjectionRebuildResult) -> bool {
        if let Some(reason) = &result.degraded_reason {
            #[cfg(unix)]
            panic!("symlink projection unexpectedly degraded on unix: {reason}");

            #[cfg(not(unix))]
            {
                assert!(!reason.is_empty());
                return false;
            }
        }
        true
    }

    #[test]
    fn allocator_returns_monotonic_orb_ids() {
        let temp = TempDir::new().expect("tempdir");
        let store = store(&temp);
        let workspace = bind(&store, temp.path());

        assert_eq!(
            store.allocate_task_id(&workspace.workspace_id).expect("id"),
            "ORB-00000"
        );
        assert_eq!(
            store.allocate_task_id(&workspace.workspace_id).expect("id"),
            "ORB-00001"
        );
    }

    #[test]
    fn open_creates_registry_parent_and_workspaces_dir() {
        let temp = TempDir::new().expect("tempdir");
        let path = registry_path(&temp);

        let _store = TaskRegistryStore::open(&path).expect("open registry");

        assert!(path.is_file());
        assert!(temp.path().join("tasks").join("workspaces").is_dir());

        let conn = Connection::open(path).expect("open registry sqlite");
        assert_eq!(
            registry_user_version(&conn).expect("read user_version"),
            REGISTRY_SCHEMA_VERSION
        );
    }

    #[test]
    fn open_rejects_newer_registry_schema_version() {
        let temp = TempDir::new().expect("tempdir");
        let path = registry_path(&temp);
        fs::create_dir_all(path.parent().expect("registry parent")).expect("create parent");
        let conn = Connection::open(&path).expect("open sqlite");
        conn.pragma_update(None, "user_version", i64::from(REGISTRY_SCHEMA_VERSION + 1))
            .expect("set user_version");
        drop(conn);

        let err = match TaskRegistryStore::open(&path) {
            Ok(_) => panic!("opened newer registry schema"),
            Err(err) => err,
        };
        assert!(
            matches!(err, OrbitError::Store(message) if message.contains("newer than supported"))
        );
    }

    #[test]
    fn allocator_is_global_across_workspaces() {
        let temp = TempDir::new().expect("tempdir");
        let store = store(&temp);
        let first = bind(&store, temp.path());
        let second_root = temp.path().join("second");
        fs::create_dir_all(second_root.join(".orbit")).expect("create second orbit dir");
        let second = store
            .bind_workspace(BindWorkspaceParams {
                workspace_id: Some("second-abcdef".into()),
                slug: "Second".into(),
                repo_root: second_root.clone(),
                workspace_path: second_root.clone(),
                orbit_dir: second_root.join(".orbit"),
                repo_fingerprint: None,
            })
            .expect("bind second workspace");

        assert_eq!(
            store
                .allocate_task_id(&first.workspace_id)
                .expect("first id"),
            "ORB-00000"
        );
        assert_eq!(
            store
                .allocate_task_id(&second.workspace_id)
                .expect("second id"),
            "ORB-00001"
        );
    }

    #[test]
    fn allocator_reports_exhaustion() {
        let temp = TempDir::new().expect("tempdir");
        let store = store(&temp);
        let workspace = bind(&store, temp.path());

        {
            let conn = store.conn.lock().expect("lock registry");
            conn.execute(
                "UPDATE allocator_state SET next_number = ?1, updated_at = ?2
                 WHERE authority = 'local'",
                params![i64::from(ORB_TASK_ID_MAX) + 1, now_string()],
            )
            .expect("force allocator exhaustion");
        }

        assert!(matches!(
            store.allocate_task_id(&workspace.workspace_id),
            Err(OrbitError::Store(message)) if message.contains("exhausted")
        ));
    }

    #[test]
    fn bind_workspace_is_idempotent_for_orbit_dir() {
        let temp = TempDir::new().expect("tempdir");
        let store = store(&temp);
        let first = bind(&store, temp.path());
        let second = store
            .bind_workspace(BindWorkspaceParams {
                workspace_id: Some(first.workspace_id.clone()),
                slug: "Changed".into(),
                repo_root: temp.path().join("."),
                workspace_path: temp.path().join("."),
                orbit_dir: temp.path().join(".orbit").join("..").join(".orbit"),
                repo_fingerprint: Some("changed".into()),
            })
            .expect("idempotent bind");

        assert_eq!(first.workspace_id, second.workspace_id);
        assert_eq!(first.slug, second.slug);
    }

    #[test]
    fn bind_workspace_rejects_explicit_workspace_id_conflict() {
        let temp = TempDir::new().expect("tempdir");
        let store = store(&temp);
        bind(&store, temp.path());

        let result = store.bind_workspace(BindWorkspaceParams {
            workspace_id: Some("other-abcdef".into()),
            slug: "Changed".into(),
            repo_root: temp.path().join("."),
            workspace_path: temp.path().join("."),
            orbit_dir: temp.path().join(".orbit").join("..").join(".orbit"),
            repo_fingerprint: Some("changed".into()),
        });

        assert!(matches!(result, Err(OrbitError::InvalidInput(_))));
    }

    #[test]
    fn workspace_config_round_trips_and_validates() {
        let temp = TempDir::new().expect("tempdir");
        let orbit_dir = temp.path().join(".orbit");
        write_workspace_config(
            &orbit_dir,
            &WorkspaceConfig {
                schema_version: 1,
                workspace_id: "orbit-test-abcdef".into(),
            },
        )
        .expect("write config");

        let read = read_workspace_config(&orbit_dir).expect("read config");
        assert_eq!(read.workspace_id, "orbit-test-abcdef");

        atomic_write_text(
            &workspace_config_path(&orbit_dir),
            "schema_version: 2\nworkspace_id: orbit-test-abcdef\n",
        )
        .expect("write wrong schema");
        assert!(matches!(
            read_workspace_config(&orbit_dir),
            Err(OrbitError::InvalidInput(_))
        ));

        atomic_write_text(
            &workspace_config_path(&orbit_dir),
            "schema_version: 1\nworkspace_id: ''\n",
        )
        .expect("write empty id");
        assert!(matches!(
            read_workspace_config(&orbit_dir),
            Err(OrbitError::InvalidInput(_))
        ));

        atomic_write_text(
            &workspace_config_path(&orbit_dir),
            "schema_version: 1\nworkspace_id: orbit-test-abcdef\nextra: nope\n",
        )
        .expect("write unknown field");
        assert!(matches!(
            read_workspace_config(&orbit_dir),
            Err(OrbitError::InvalidInput(_))
        ));
    }

    #[test]
    fn workspace_config_optional_distinguishes_missing_file() {
        let temp = TempDir::new().expect("tempdir");

        assert_eq!(
            read_workspace_config_optional(&temp.path().join(".orbit"))
                .expect("read optional config"),
            None
        );
    }

    #[test]
    fn rebind_candidates_match_normalized_paths() {
        let temp = TempDir::new().expect("tempdir");
        let store = store(&temp);
        let workspace = bind(&store, temp.path());

        let candidates = store
            .find_rebind_candidates(
                &temp.path().join("."),
                &temp.path().join("nested").join(".."),
                &workspace.orbit_dir.join("..").join(".orbit"),
            )
            .expect("candidates");

        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].workspace_id, workspace.workspace_id);
    }

    #[test]
    fn register_task_bundle_rejects_non_canonical_path() {
        let temp = TempDir::new().expect("tempdir");
        let store = store(&temp);
        let workspace = bind(&store, temp.path());
        let wrong_path = temp.path().join("other-workspace").join("ORB-00000");
        fs::create_dir_all(&wrong_path).expect("create wrong bundle");

        assert!(matches!(
            store.register_task_bundle("ORB-00000", &workspace.workspace_id, &wrong_path),
            Err(OrbitError::InvalidInput(_))
        ));
    }

    #[test]
    fn generated_task_index_filters_by_status_priority_and_tags() {
        let temp = TempDir::new().expect("tempdir");
        let store = store(&temp);
        let workspace = bind(&store, temp.path());

        for task_id in ["ORB-00000", "ORB-00001"] {
            let bundle_dir = create_canonical_bundle(&store, &workspace, task_id);
            store
                .register_task_bundle(task_id, &workspace.workspace_id, &bundle_dir)
                .expect("register bundle");
        }

        store
            .replace_task_index(
                &workspace.workspace_id,
                &envelope(
                    "ORB-00000",
                    TaskStatus::Backlog,
                    vec!["Task-Artifacts".into(), "v2".into()],
                    Vec::new(),
                ),
            )
            .expect("index first task");
        store
            .replace_task_index(
                &workspace.workspace_id,
                &envelope(
                    "ORB-00001",
                    TaskStatus::Review,
                    vec!["v2".into(), "review".into()],
                    Vec::new(),
                ),
            )
            .expect("index second task");

        assert_eq!(
            store
                .indexed_task_count_for_workspace(&workspace.workspace_id)
                .expect("index count"),
            2
        );
        assert_eq!(
            store
                .indexed_task_ids_filtered(
                    &workspace.workspace_id,
                    &TaskIndexFilter {
                        status: Some(TaskStatus::Review),
                        priority: Some(TaskPriority::High),
                        tags: vec!["review".into()],
                    },
                )
                .expect("filtered ids"),
            vec!["ORB-00001"]
        );
        assert_eq!(
            store
                .indexed_task_ids_filtered(
                    &workspace.workspace_id,
                    &TaskIndexFilter {
                        status: None,
                        priority: None,
                        tags: vec!["task-artifacts".into(), "v2".into()],
                    },
                )
                .expect("tagged ids"),
            vec!["ORB-00000"]
        );
    }

    #[test]
    fn generated_relation_index_supports_forward_and_inverse_lookup() {
        let temp = TempDir::new().expect("tempdir");
        let store = store(&temp);
        let workspace = bind(&store, temp.path());
        let bundle_dir = create_canonical_bundle(&store, &workspace, "ORB-00000");
        store
            .register_task_bundle("ORB-00000", &workspace.workspace_id, &bundle_dir)
            .expect("register bundle");

        store
            .replace_task_index(
                &workspace.workspace_id,
                &envelope(
                    "ORB-00000",
                    TaskStatus::Backlog,
                    Vec::new(),
                    vec![
                        TaskRelation {
                            relation_type: TaskRelationType::Blocks,
                            target: "ORB-00001".to_string(),
                        },
                        TaskRelation {
                            relation_type: TaskRelationType::RelatedTo,
                            target: "ORB-00002".to_string(),
                        },
                    ],
                ),
            )
            .expect("index relations");

        assert_eq!(
            store
                .indexed_relation_targets(
                    &workspace.workspace_id,
                    "ORB-00000",
                    TaskRelationType::Blocks,
                )
                .expect("targets"),
            vec!["ORB-00001"]
        );
        assert_eq!(
            store
                .indexed_relation_sources(
                    &workspace.workspace_id,
                    "ORB-00001",
                    TaskRelationType::Blocks,
                )
                .expect("sources"),
            vec!["ORB-00000"]
        );
    }

    #[test]
    fn unregister_task_bundle_removes_binding_indexes_and_relation_edges() {
        let temp = TempDir::new().expect("tempdir");
        let store = store(&temp);
        let workspace = bind(&store, temp.path());
        for task_id in ["ORB-00000", "ORB-00001"] {
            let bundle_dir = create_canonical_bundle(&store, &workspace, task_id);
            store
                .register_task_bundle(task_id, &workspace.workspace_id, &bundle_dir)
                .expect("register bundle");
        }
        store
            .replace_task_index(
                &workspace.workspace_id,
                &envelope(
                    "ORB-00000",
                    TaskStatus::Backlog,
                    vec!["v2".into()],
                    vec![TaskRelation {
                        relation_type: TaskRelationType::Blocks,
                        target: "ORB-00001".to_string(),
                    }],
                ),
            )
            .expect("index source relation");

        assert!(
            store
                .unregister_task_bundle("ORB-00000", &workspace.workspace_id)
                .expect("unregister")
        );
        assert_eq!(
            store
                .tasks_for_workspace(&workspace.workspace_id)
                .expect("tasks")
                .into_iter()
                .map(|binding| binding.task_id)
                .collect::<Vec<_>>(),
            vec!["ORB-00001"]
        );
        assert_eq!(
            store
                .indexed_task_count_for_workspace(&workspace.workspace_id)
                .expect("index count"),
            0
        );
        assert_eq!(
            store
                .indexed_relation_sources(
                    &workspace.workspace_id,
                    "ORB-00001",
                    TaskRelationType::Blocks,
                )
                .expect("inverse relation"),
            Vec::<String>::new()
        );
    }

    #[test]
    fn projection_rebuild_creates_and_repairs_symlinks() {
        let temp = TempDir::new().expect("tempdir");
        let store = store(&temp);
        let workspace = bind(&store, temp.path());
        let bundle_dir = create_canonical_bundle(&store, &workspace, "ORB-00000");
        let wrong_bundle_dir = temp.path().join("wrong-task-target");
        fs::create_dir_all(&wrong_bundle_dir).expect("create wrong bundle");

        store
            .register_task_bundle("ORB-00000", &workspace.workspace_id, &bundle_dir)
            .expect("register bundle");
        let first = store
            .rebuild_projection(&workspace.orbit_dir, &workspace.workspace_id)
            .expect("rebuild");
        if !projection_links_supported(&first) {
            return;
        }
        assert_eq!(first.projected, 1);

        let link_path = workspace.orbit_dir.join("tasks").join("ORB-00000");
        fs::remove_file(&link_path).expect("remove correct link");
        create_dir_symlink(&wrong_bundle_dir, &link_path).expect("create wrong link");

        let second = store
            .rebuild_projection(&workspace.orbit_dir, &workspace.workspace_id)
            .expect("rebuild repair");
        assert_eq!(second.repaired, 1);
        assert_eq!(
            normalize_path(&fs::read_link(&link_path).expect("read link")),
            normalize_path(&bundle_dir)
        );
    }

    #[test]
    fn projection_rebuild_recovers_after_reopen_and_projection_delete() {
        let temp = TempDir::new().expect("tempdir");
        let path = registry_path(&temp);
        let store = TaskRegistryStore::open(&path).expect("open registry");
        let workspace = bind(&store, temp.path());
        let bundle_dir = create_canonical_bundle(&store, &workspace, "ORB-00000");
        store
            .register_task_bundle("ORB-00000", &workspace.workspace_id, &bundle_dir)
            .expect("register bundle");

        let first = store
            .rebuild_projection(&workspace.orbit_dir, &workspace.workspace_id)
            .expect("initial rebuild");
        if !projection_links_supported(&first) {
            return;
        }
        fs::remove_dir_all(workspace.orbit_dir.join("tasks")).expect("delete projection");
        drop(store);

        let reopened = TaskRegistryStore::open(&path).expect("reopen registry");
        let rebuilt = reopened
            .rebuild_projection(&workspace.orbit_dir, &workspace.workspace_id)
            .expect("rebuild after reopen");
        if !projection_links_supported(&rebuilt) {
            return;
        }
        assert_eq!(rebuilt.projected, 1);
        assert_eq!(
            normalize_path(
                &fs::read_link(workspace.orbit_dir.join("tasks").join("ORB-00000"))
                    .expect("read link")
            ),
            normalize_path(&bundle_dir)
        );
    }

    #[test]
    fn projection_rebuild_errors_on_non_symlink_blocker() {
        let temp = TempDir::new().expect("tempdir");
        let store = store(&temp);
        let workspace = bind(&store, temp.path());
        let bundle_dir = create_canonical_bundle(&store, &workspace, "ORB-00000");
        store
            .register_task_bundle("ORB-00000", &workspace.workspace_id, &bundle_dir)
            .expect("register bundle");
        let projection_dir = workspace.orbit_dir.join("tasks");
        fs::create_dir_all(&projection_dir).expect("create projection");
        fs::write(projection_dir.join("ORB-00000"), "blocker").expect("write blocker");

        assert!(matches!(
            store.rebuild_projection(&workspace.orbit_dir, &workspace.workspace_id),
            Err(OrbitError::Store(_))
        ));
    }
}
