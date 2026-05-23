use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use orbit_common::types::{OrbitError, TaskEnvelopeV2, normalize_task_tags};
use rusqlite::{Connection, OptionalExtension, params};

use super::types::{TaskBundleBinding, WorkspaceBinding};
use super::util::{path_to_string, relation_type_name, terminal_month};

pub(super) fn workspace_by_orbit_dir(
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

pub(super) fn workspace_by_id(
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

pub(super) fn task_bundle_by_id(
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

pub(super) fn task_ids_for_workspace(
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

pub(super) fn write_task_index_rows(
    tx: &rusqlite::Transaction<'_>,
    workspace_id: &str,
    envelope: &TaskEnvelopeV2,
) -> Result<(), OrbitError> {
    tx.execute(
        "INSERT INTO task_bundle_index (
            task_id, workspace_id, status, priority, job_run_id, created_at, updated_at, terminal_month
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
        ON CONFLICT(task_id) DO UPDATE SET
            workspace_id = excluded.workspace_id,
            status = excluded.status,
            priority = excluded.priority,
            job_run_id = excluded.job_run_id,
            created_at = excluded.created_at,
            updated_at = excluded.updated_at,
            terminal_month = excluded.terminal_month",
        params![
            &envelope.id,
            workspace_id,
            envelope.status.to_string(),
            envelope.priority.to_string(),
            envelope.job_run_id.as_deref(),
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

fn parse_timestamp(raw: &str) -> rusqlite::Result<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(raw)
        .map(|parsed| parsed.with_timezone(&Utc))
        .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))
}

pub(super) fn decode_workspace_binding(
    row: &rusqlite::Row<'_>,
) -> rusqlite::Result<WorkspaceBinding> {
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

pub(super) fn decode_task_bundle_binding(
    row: &rusqlite::Row<'_>,
) -> rusqlite::Result<TaskBundleBinding> {
    Ok(TaskBundleBinding {
        task_id: row.get(0)?,
        workspace_id: row.get(1)?,
        canonical_path: PathBuf::from(row.get::<_, String>(2)?),
        created_at: parse_timestamp(&row.get::<_, String>(3)?)?,
        updated_at: parse_timestamp(&row.get::<_, String>(4)?)?,
    })
}
