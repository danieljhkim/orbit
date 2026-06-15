use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use orbit_common::types::{
    NotFoundKind, ORB_TASK_ID_MAX, OrbitError, TaskEnvelopeV2, TaskRelationType,
    format_orb_task_id, normalize_task_tags, validate_orb_task_id,
};
use rusqlite::{Connection, TransactionBehavior, params, params_from_iter};

use super::projection::create_projection_symlink;
use super::queries::{
    decode_task_bundle_binding, decode_workspace_binding, task_bundle_by_id,
    task_ids_for_workspace, workspace_by_id, workspace_by_orbit_dir, write_task_index_rows,
};
use super::schema::{
    apply_schema, assert_registry_user_version, reject_unsupported_registry_schema,
};
use super::types::{
    BindWorkspaceParams, ProjectionRebuildResult, TaskBundleBinding, TaskIndexFilter,
    WorkspaceBinding,
};
use super::util::{
    enable_best_effort_wal_mode, normalize_path, now_string, path_to_string, relation_type_name,
};
use super::workspace_id::{next_workspace_id_candidate, sanitize_slug, validate_workspace_id};

#[derive(Clone)]
pub struct TaskRegistryStore {
    pub(super) conn: Arc<Mutex<Connection>>,
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
        // The registry is the commit point that makes a created task official, so
        // its writes must be durable against power loss the moment they ack. WAL's
        // default synchronous=NORMAL only fsyncs the WAL at checkpoint, leaving an
        // acked register_task_bundle exposed to rollback on a hard reset. FULL
        // fsyncs the WAL on every commit, closing that window. The registry is
        // low-write (≈one commit per task create/bind/unregister), so the extra
        // fsync cost is negligible. Scoped to this connection only — the shared
        // Store::open stays at NORMAL for higher-write stores.
        conn.pragma_update(None, "synchronous", "FULL")
            .map_err(|e| OrbitError::Store(format!("failed to set synchronous=FULL: {e}")))?;
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
            if let Some(requested) = &requested_workspace_id
                && requested != &existing.workspace_id
            {
                return Err(OrbitError::InvalidInput(format!(
                    "orbit dir '{}' is already bound to workspace '{}', not '{}'",
                    orbit_dir.display(),
                    existing.workspace_id,
                    requested
                )));
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
            return Err(OrbitError::not_found(NotFoundKind::Workspace, workspace_id));
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
            return Err(OrbitError::not_found(NotFoundKind::Workspace, workspace_id));
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
            .ok_or_else(|| OrbitError::not_found(NotFoundKind::Task, envelope.id.clone()))?;
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
        if let Some(job_run_id) = &filter.job_run_id {
            sql.push_str(" AND job_run_id = ?");
            values.push(job_run_id.clone());
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
