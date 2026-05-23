use std::collections::BTreeMap;
use std::fs::{self, File, OpenOptions};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use chrono::{DateTime, Utc};
use fs2::FileExt;
use orbit_common::types::OrbitError;
use orbit_common::utility::fs::atomic_write_text;
use orbit_common::utility::git::{CurrentBranchStatus, current_branch};
use rusqlite::{Connection, Transaction, TransactionBehavior, params, types::Type};
use serde::{Deserialize, Serialize};
use serde_yaml::{Mapping, Value};

const KIND_ADR: &str = "adr";
const KIND_LEARNING: &str = "learning";
const STATUS_ABANDONED: &str = "abandoned";
const STATUS_MERGED: &str = "merged";
const STATUS_RESERVED: &str = "reserved";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IdAllocationKind {
    Adr,
    Learning,
}

impl IdAllocationKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Adr => KIND_ADR,
            Self::Learning => KIND_LEARNING,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IdAllocation {
    pub kind: IdAllocationKind,
    pub id: String,
    pub worktree_root: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IdAllocationRecord {
    pub kind: IdAllocationKind,
    pub id: String,
    pub allocated_at: i64,
    pub worktree_root: PathBuf,
    pub branch: Option<String>,
    pub status: String,
    pub body_path: Option<PathBuf>,
}

impl IdAllocationRecord {
    pub fn resolved_body_path(&self) -> Option<PathBuf> {
        self.body_path.as_ref().map(|body_path| {
            if body_path.is_absolute() {
                body_path.clone()
            } else {
                self.worktree_root.join(body_path)
            }
        })
    }
}

#[derive(Debug, Clone)]
pub struct IdAllocatorConfig {
    pub semantic_db_path: PathBuf,
    pub lock_path: PathBuf,
    pub shared_root: PathBuf,
    pub worktree_root: PathBuf,
    pub adr_root: PathBuf,
    pub learning_root: PathBuf,
}

#[derive(Clone)]
pub struct IdAllocator {
    inner: Arc<IdAllocatorInner>,
}

struct IdAllocatorInner {
    conn: Mutex<Connection>,
    lock_path: PathBuf,
    shared_root: PathBuf,
    worktree_root: PathBuf,
    adr_root: PathBuf,
    learning_root: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LearningIdRename {
    pub old_id: String,
    pub new_id: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LearningIdMigrationReport {
    pub renames: Vec<LearningIdRename>,
}

impl LearningIdMigrationReport {
    pub fn is_empty(&self) -> bool {
        self.renames.is_empty()
    }

    pub fn rename_map(&self) -> BTreeMap<String, String> {
        self.renames
            .iter()
            .map(|rename| (rename.old_id.clone(), rename.new_id.clone()))
            .collect()
    }
}

impl IdAllocatorConfig {
    pub fn new(
        semantic_db_path: PathBuf,
        lock_path: PathBuf,
        shared_root: PathBuf,
        worktree_root: PathBuf,
        adr_root: PathBuf,
        learning_root: PathBuf,
    ) -> Self {
        Self {
            semantic_db_path,
            lock_path,
            shared_root,
            worktree_root,
            adr_root,
            learning_root,
        }
    }
}

impl IdAllocator {
    pub fn open(config: IdAllocatorConfig) -> Result<Self, OrbitError> {
        if let Some(parent) = config.semantic_db_path.parent() {
            fs::create_dir_all(parent).map_err(|error| OrbitError::Store(error.to_string()))?;
        }
        let conn = Connection::open(&config.semantic_db_path)
            .map_err(|e| OrbitError::Store(e.to_string()))?;
        enable_best_effort_wal_mode(&conn);
        conn.pragma_update(None, "busy_timeout", "5000")
            .map_err(|e| OrbitError::Store(format!("failed to set busy_timeout: {e}")))?;
        ensure_id_allocation_schema(&conn)?;

        let allocator = Self {
            inner: Arc::new(IdAllocatorInner {
                conn: Mutex::new(conn),
                lock_path: config.lock_path,
                shared_root: absolutize(config.shared_root),
                worktree_root: absolutize(config.worktree_root),
                adr_root: config.adr_root,
                learning_root: config.learning_root,
            }),
        };
        allocator.backfill_existing_ids()?;
        Ok(allocator)
    }

    #[cfg(test)]
    pub(crate) fn for_test_roots(adr_root: PathBuf, learning_root: PathBuf) -> Self {
        let base = if learning_root.starts_with(&adr_root) {
            adr_root.clone()
        } else if adr_root.starts_with(&learning_root) {
            learning_root.clone()
        } else {
            adr_root.clone()
        };
        let state_dir = base.join(".id-allocator-test-state");
        Self::open(IdAllocatorConfig::new(
            state_dir.join("semantic.db"),
            state_dir.join(".id_alloc.lock"),
            base.clone(),
            base,
            adr_root,
            learning_root,
        ))
        .expect("test id allocator")
    }

    pub fn allocate_adr(&self) -> Result<IdAllocation, OrbitError> {
        self.allocate(IdAllocationKind::Adr)
    }

    pub fn allocate_learning(&self) -> Result<IdAllocation, OrbitError> {
        self.allocate(IdAllocationKind::Learning)
    }

    pub fn record_adr_body_path(&self, id: &str, body_path: &Path) -> Result<(), OrbitError> {
        self.record_body_path(IdAllocationKind::Adr, id, body_path)
    }

    pub fn record_learning_body_path(&self, id: &str, body_path: &Path) -> Result<(), OrbitError> {
        self.record_body_path(IdAllocationKind::Learning, id, body_path)
    }

    pub fn abandon_learning(&self, id: &str) -> Result<(), OrbitError> {
        self.abandon(IdAllocationKind::Learning, id)
    }

    pub fn adr_allocation(&self, id: &str) -> Result<Option<IdAllocationRecord>, OrbitError> {
        self.allocation(IdAllocationKind::Adr, id)
    }

    pub fn learning_allocation(&self, id: &str) -> Result<Option<IdAllocationRecord>, OrbitError> {
        self.allocation(IdAllocationKind::Learning, id)
    }

    pub fn adr_allocations(&self) -> Result<Vec<IdAllocationRecord>, OrbitError> {
        self.allocations(IdAllocationKind::Adr)
    }

    pub fn learning_allocations(&self) -> Result<Vec<IdAllocationRecord>, OrbitError> {
        self.allocations(IdAllocationKind::Learning)
    }

    pub fn migrate_learning_ids(&self) -> Result<LearningIdMigrationReport, OrbitError> {
        let _lock = self.acquire_lock()?;
        let mut conn = self.lock_conn()?;
        let tx = conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|error| OrbitError::Store(error.to_string()))?;

        let report = self.migrate_learning_ids_in_transaction(&tx)?;
        tx.commit()
            .map_err(|error| OrbitError::Store(error.to_string()))?;
        Ok(report)
    }

    fn allocate(&self, kind: IdAllocationKind) -> Result<IdAllocation, OrbitError> {
        let _lock = self.acquire_lock()?;
        let mut conn = self.lock_conn()?;
        let tx = conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|error| OrbitError::Store(error.to_string()))?;
        let next = next_id_for_kind(&tx, kind)?;
        insert_allocation(
            &tx,
            kind,
            &next,
            Utc::now().timestamp(),
            &self.inner.worktree_root,
            best_effort_branch(&self.inner.worktree_root),
            STATUS_RESERVED,
            None,
            false,
        )?;
        tx.commit()
            .map_err(|error| OrbitError::Store(error.to_string()))?;
        Ok(IdAllocation {
            kind,
            id: next,
            worktree_root: self.inner.worktree_root.clone(),
        })
    }

    fn backfill_existing_ids(&self) -> Result<(), OrbitError> {
        let _lock = self.acquire_lock()?;
        let mut conn = self.lock_conn()?;
        let tx = conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|error| OrbitError::Store(error.to_string()))?;
        self.backfill_adrs(&tx)?;
        self.backfill_canonical_learnings(&tx)?;
        tx.commit()
            .map_err(|error| OrbitError::Store(error.to_string()))?;
        Ok(())
    }

    fn backfill_adrs(&self, tx: &Transaction<'_>) -> Result<(), OrbitError> {
        for state in ["proposed", "accepted", "superseded", "deleted"] {
            let dir = self.inner.adr_root.join(state);
            if !dir.is_dir() {
                continue;
            }
            for child in child_dirs(&dir)? {
                let Some(id) = child.file_name().and_then(|name| name.to_str()) else {
                    continue;
                };
                if parse_adr_sequence(id).is_none() || !child.join("adr.yaml").is_file() {
                    continue;
                }
                let allocated_at =
                    yaml_epoch(&child.join("adr.yaml")).unwrap_or_else(|_| now_epoch());
                let body_path = relative_to(&child.join("body.md"), &self.inner.shared_root);
                insert_allocation(
                    tx,
                    IdAllocationKind::Adr,
                    id,
                    allocated_at,
                    &self.inner.shared_root,
                    None,
                    STATUS_MERGED,
                    Some(&body_path),
                    true,
                )?;
                update_body_metadata_if_missing(
                    tx,
                    IdAllocationKind::Adr,
                    id,
                    &self.inner.shared_root,
                    None,
                    &body_path,
                    STATUS_MERGED,
                )?;
            }
        }
        Ok(())
    }

    fn backfill_canonical_learnings(&self, tx: &Transaction<'_>) -> Result<(), OrbitError> {
        if !self.inner.learning_root.is_dir() {
            return Ok(());
        }
        for child in child_dirs(&self.inner.learning_root)? {
            let Some(id) = child.file_name().and_then(|name| name.to_str()) else {
                continue;
            };
            if parse_learning_sequence(id).is_none() || !child.join("learning.yaml").is_file() {
                continue;
            }
            let allocated_at =
                yaml_epoch(&child.join("learning.yaml")).unwrap_or_else(|_| now_epoch());
            let body_path = relative_to(&child.join("learning.yaml"), &self.inner.shared_root);
            insert_allocation(
                tx,
                IdAllocationKind::Learning,
                id,
                allocated_at,
                &self.inner.shared_root,
                None,
                STATUS_MERGED,
                Some(&body_path),
                true,
            )?;
            update_body_metadata_if_missing(
                tx,
                IdAllocationKind::Learning,
                id,
                &self.inner.shared_root,
                None,
                &body_path,
                STATUS_MERGED,
            )?;
        }
        Ok(())
    }

    fn migrate_learning_ids_in_transaction(
        &self,
        tx: &Transaction<'_>,
    ) -> Result<LearningIdMigrationReport, OrbitError> {
        if !self.inner.learning_root.is_dir() {
            return Ok(LearningIdMigrationReport::default());
        }

        let mut entries = Vec::new();
        for child in child_dirs(&self.inner.learning_root)? {
            let Some(old_id) = child.file_name().and_then(|name| name.to_str()) else {
                continue;
            };
            if !is_legacy_learning_id(old_id) {
                continue;
            }
            let yaml_path = child.join("learning.yaml");
            if !yaml_path.is_file() {
                continue;
            }
            let created_at = yaml_epoch(&yaml_path)?;
            entries.push((created_at, old_id.to_string(), child, yaml_path));
        }
        entries.sort_by(|left, right| left.0.cmp(&right.0).then_with(|| left.1.cmp(&right.1)));

        if entries.is_empty() {
            return Ok(LearningIdMigrationReport::default());
        }

        let existing_max = max_sequence(tx, IdAllocationKind::Learning)?;
        let mut renames = Vec::with_capacity(entries.len());
        for (offset, (_, old_id, _, _)) in entries.iter().enumerate() {
            let next = existing_max
                .checked_add(offset as u32)
                .and_then(|value| value.checked_add(1))
                .ok_or_else(|| OrbitError::Execution("learning id counter overflow".to_string()))?;
            renames.push(LearningIdRename {
                old_id: old_id.clone(),
                new_id: format_learning_id(next),
            });
        }
        let rename_map: BTreeMap<String, String> = renames
            .iter()
            .map(|rename| (rename.old_id.clone(), rename.new_id.clone()))
            .collect();

        for ((created_at, old_id, old_dir, yaml_path), rename) in entries.iter().zip(&renames) {
            let new_dir = self.inner.learning_root.join(&rename.new_id);
            if new_dir.exists() {
                return Err(OrbitError::Migration(format!(
                    "cannot migrate learning id {old_id} to {}: destination already exists",
                    rename.new_id
                )));
            }

            let mut value = read_yaml_value(yaml_path)?;
            rewrite_learning_yaml(&mut value, old_id, &rename.new_id, &rename_map)?;
            let rendered = serde_yaml::to_string(&value)
                .map_err(|error| OrbitError::Migration(error.to_string()))?;
            atomic_write_text(yaml_path, &rendered).map_err(|error| {
                OrbitError::Io(format!("write {}: {error}", yaml_path.display()))
            })?;
            fs::rename(old_dir, &new_dir).map_err(|error| {
                OrbitError::Io(format!(
                    "rename {} to {}: {error}",
                    old_dir.display(),
                    new_dir.display()
                ))
            })?;
            insert_allocation(
                tx,
                IdAllocationKind::Learning,
                &rename.new_id,
                *created_at,
                &self.inner.shared_root,
                None,
                STATUS_MERGED,
                Some(&relative_to(
                    &new_dir.join("learning.yaml"),
                    &self.inner.shared_root,
                )),
                true,
            )?;
        }

        Ok(LearningIdMigrationReport { renames })
    }

    fn record_body_path(
        &self,
        kind: IdAllocationKind,
        id: &str,
        body_path: &Path,
    ) -> Result<(), OrbitError> {
        let relative_body_path = relative_to(body_path, &self.inner.worktree_root);
        let branch = best_effort_branch(&self.inner.worktree_root);
        let _lock = self.acquire_lock()?;
        let mut conn = self.lock_conn()?;
        let tx = conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|error| OrbitError::Store(error.to_string()))?;
        let updated = tx
            .execute(
                "UPDATE id_allocations
                 SET worktree_root = ?3, branch = ?4, body_path = ?5
                 WHERE kind = ?1 AND id = ?2",
                params![
                    kind.as_str(),
                    id,
                    self.inner.worktree_root.to_string_lossy(),
                    branch,
                    relative_body_path.to_string_lossy(),
                ],
            )
            .map_err(|error| OrbitError::Store(error.to_string()))?;
        if updated == 0 {
            return Err(OrbitError::Store(format!(
                "id allocation row missing for {} {id}",
                kind.as_str()
            )));
        }
        tx.commit()
            .map_err(|error| OrbitError::Store(error.to_string()))?;
        Ok(())
    }

    fn abandon(&self, kind: IdAllocationKind, id: &str) -> Result<(), OrbitError> {
        let _lock = self.acquire_lock()?;
        let mut conn = self.lock_conn()?;
        let tx = conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|error| OrbitError::Store(error.to_string()))?;
        let updated = tx
            .execute(
                "UPDATE id_allocations
                 SET status = ?3
                 WHERE kind = ?1
                   AND id = ?2
                   AND status = ?4
                   AND (body_path IS NULL OR body_path = '')",
                params![kind.as_str(), id, STATUS_ABANDONED, STATUS_RESERVED],
            )
            .map_err(|error| OrbitError::Store(error.to_string()))?;
        if updated == 0 {
            return Err(OrbitError::Store(format!(
                "cannot abandon {} {id}: allocation is missing or already has a body path",
                kind.as_str()
            )));
        }
        tx.commit()
            .map_err(|error| OrbitError::Store(error.to_string()))?;
        Ok(())
    }

    fn allocation(
        &self,
        kind: IdAllocationKind,
        id: &str,
    ) -> Result<Option<IdAllocationRecord>, OrbitError> {
        let conn = self.lock_conn()?;
        let mut stmt = conn
            .prepare(
                "SELECT kind, id, allocated_at, worktree_root, branch, status, body_path
                 FROM id_allocations
                 WHERE kind = ?1 AND id = ?2 AND status != ?3",
            )
            .map_err(|error| OrbitError::Store(error.to_string()))?;
        let mut rows = stmt
            .query_map(
                params![kind.as_str(), id, STATUS_ABANDONED],
                allocation_record_from_row,
            )
            .map_err(|error| OrbitError::Store(error.to_string()))?;
        match rows.next() {
            Some(row) => row
                .map(Some)
                .map_err(|error| OrbitError::Store(error.to_string())),
            None => Ok(None),
        }
    }

    fn allocations(&self, kind: IdAllocationKind) -> Result<Vec<IdAllocationRecord>, OrbitError> {
        let conn = self.lock_conn()?;
        let mut stmt = conn
            .prepare(
                "SELECT kind, id, allocated_at, worktree_root, branch, status, body_path
                 FROM id_allocations
                 WHERE kind = ?1 AND status != ?2
                 ORDER BY id DESC",
            )
            .map_err(|error| OrbitError::Store(error.to_string()))?;
        let rows = stmt
            .query_map(
                params![kind.as_str(), STATUS_ABANDONED],
                allocation_record_from_row,
            )
            .map_err(|error| OrbitError::Store(error.to_string()))?;
        let mut records = Vec::new();
        for row in rows {
            records.push(row.map_err(|error| OrbitError::Store(error.to_string()))?);
        }
        Ok(records)
    }

    fn acquire_lock(&self) -> Result<File, OrbitError> {
        let parent = self.inner.lock_path.parent().ok_or_else(|| {
            OrbitError::Store(format!(
                "cannot determine id allocation lock parent for '{}'",
                self.inner.lock_path.display()
            ))
        })?;
        fs::create_dir_all(parent).map_err(|error| OrbitError::Io(error.to_string()))?;
        let file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(&self.inner.lock_path)
            .map_err(|error| OrbitError::Io(error.to_string()))?;
        file.lock_exclusive().map_err(|error| {
            OrbitError::Store(format!(
                "failed to acquire id allocation lock '{}': {error}",
                self.inner.lock_path.display()
            ))
        })?;
        Ok(file)
    }

    fn lock_conn(&self) -> Result<std::sync::MutexGuard<'_, Connection>, OrbitError> {
        self.inner
            .conn
            .lock()
            .map_err(|error| OrbitError::Store(format!("mutex poisoned: {error}")))
    }
}

pub fn ensure_id_allocation_schema(conn: &Connection) -> Result<(), OrbitError> {
    conn.execute_batch(
        r#"
            CREATE TABLE IF NOT EXISTS id_allocations (
                kind TEXT NOT NULL,
                id TEXT NOT NULL,
                allocated_at INTEGER NOT NULL,
                worktree_root TEXT NOT NULL,
                branch TEXT,
                status TEXT NOT NULL,
                body_path TEXT,
                PRIMARY KEY (kind, id),
                CHECK (kind IN ('adr', 'learning')),
                CHECK (status IN ('reserved', 'merged', 'abandoned'))
            );

            CREATE INDEX IF NOT EXISTS idx_id_allocations_kind_status
            ON id_allocations(kind, status);

            CREATE INDEX IF NOT EXISTS idx_id_allocations_allocated_at
            ON id_allocations(allocated_at);
        "#,
    )
    .map_err(|error| OrbitError::Store(error.to_string()))?;
    add_column_if_missing(conn, "id_allocations", "body_path", "TEXT")?;
    Ok(())
}

fn next_id_for_kind(tx: &Transaction<'_>, kind: IdAllocationKind) -> Result<String, OrbitError> {
    let next = max_sequence(tx, kind)?
        .checked_add(1)
        .ok_or_else(|| OrbitError::Execution("id counter overflow".to_string()))?;
    Ok(match kind {
        IdAllocationKind::Adr => format_adr_id(next),
        IdAllocationKind::Learning => format_learning_id(next),
    })
}

fn max_sequence(tx: &Transaction<'_>, kind: IdAllocationKind) -> Result<u32, OrbitError> {
    let mut stmt = tx
        .prepare("SELECT id FROM id_allocations WHERE kind = ?1")
        .map_err(|error| OrbitError::Store(error.to_string()))?;
    let rows = stmt
        .query_map([kind.as_str()], |row| row.get::<_, String>(0))
        .map_err(|error| OrbitError::Store(error.to_string()))?;

    let mut max_seen = 0u32;
    for row in rows {
        let id = row.map_err(|error| OrbitError::Store(error.to_string()))?;
        let sequence = match kind {
            IdAllocationKind::Adr => parse_adr_sequence(&id),
            IdAllocationKind::Learning => parse_learning_sequence(&id),
        };
        if let Some(sequence) = sequence {
            max_seen = max_seen.max(sequence);
        }
    }
    Ok(max_seen)
}

#[allow(clippy::too_many_arguments)]
fn insert_allocation(
    tx: &Transaction<'_>,
    kind: IdAllocationKind,
    id: &str,
    allocated_at: i64,
    worktree_root: &Path,
    branch: Option<String>,
    status: &str,
    body_path: Option<&Path>,
    ignore_existing: bool,
) -> Result<(), OrbitError> {
    let verb = if ignore_existing {
        "INSERT OR IGNORE"
    } else {
        "INSERT"
    };
    let sql = format!(
        "{verb} INTO id_allocations(kind, id, allocated_at, worktree_root, branch, status, body_path) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)"
    );
    tx.execute(
        &sql,
        params![
            kind.as_str(),
            id,
            allocated_at,
            worktree_root.to_string_lossy(),
            branch,
            status,
            body_path.map(|path| path.to_string_lossy().into_owned()),
        ],
    )
    .map_err(|error| OrbitError::Store(error.to_string()))?;
    Ok(())
}

fn update_body_metadata_if_missing(
    tx: &Transaction<'_>,
    kind: IdAllocationKind,
    id: &str,
    worktree_root: &Path,
    branch: Option<String>,
    body_path: &Path,
    status_if_missing: &str,
) -> Result<(), OrbitError> {
    tx.execute(
        "UPDATE id_allocations
         SET worktree_root = ?3,
             branch = COALESCE(branch, ?4),
             body_path = ?5,
             status = CASE
                 WHEN status = ?6 THEN ?7
                 ELSE status
             END
         WHERE kind = ?1
           AND id = ?2
           AND (body_path IS NULL OR body_path = '' OR status = ?6)",
        params![
            kind.as_str(),
            id,
            worktree_root.to_string_lossy(),
            branch,
            body_path.to_string_lossy(),
            STATUS_ABANDONED,
            status_if_missing,
        ],
    )
    .map_err(|error| OrbitError::Store(error.to_string()))?;
    Ok(())
}

fn add_column_if_missing(
    conn: &Connection,
    table: &str,
    column: &str,
    column_type: &str,
) -> Result<(), OrbitError> {
    let mut stmt = conn
        .prepare(&format!("PRAGMA table_info({table})"))
        .map_err(|error| OrbitError::Store(error.to_string()))?;
    let rows = stmt
        .query_map([], |row| row.get::<_, String>(1))
        .map_err(|error| OrbitError::Store(error.to_string()))?;
    for row in rows {
        if row.map_err(|error| OrbitError::Store(error.to_string()))? == column {
            return Ok(());
        }
    }
    conn.execute(
        &format!("ALTER TABLE {table} ADD COLUMN {column} {column_type}"),
        [],
    )
    .map_err(|error| OrbitError::Store(error.to_string()))?;
    Ok(())
}

fn allocation_record_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<IdAllocationRecord> {
    let kind_raw: String = row.get(0)?;
    let kind = match kind_raw.as_str() {
        KIND_ADR => IdAllocationKind::Adr,
        KIND_LEARNING => IdAllocationKind::Learning,
        _ => {
            return Err(rusqlite::Error::InvalidColumnType(
                0,
                "kind".to_string(),
                Type::Text,
            ));
        }
    };
    let body_path: Option<String> = row.get(6)?;
    Ok(IdAllocationRecord {
        kind,
        id: row.get(1)?,
        allocated_at: row.get(2)?,
        worktree_root: PathBuf::from(row.get::<_, String>(3)?),
        branch: row.get(4)?,
        status: row.get(5)?,
        body_path: body_path.map(PathBuf::from),
    })
}

pub fn parse_adr_sequence(id: &str) -> Option<u32> {
    let suffix = id.strip_prefix("ADR-")?;
    parse_padded_sequence(suffix)
}

pub fn parse_learning_sequence(id: &str) -> Option<u32> {
    let suffix = id.strip_prefix("L-")?;
    parse_padded_sequence(suffix)
}

fn parse_padded_sequence(suffix: &str) -> Option<u32> {
    if suffix.len() < 4 || !suffix.as_bytes().iter().all(u8::is_ascii_digit) {
        return None;
    }
    suffix.parse::<u32>().ok()
}

fn format_adr_id(sequence: u32) -> String {
    let width = sequence.to_string().len().max(4);
    format!("ADR-{sequence:0width$}")
}

fn format_learning_id(sequence: u32) -> String {
    let width = sequence.to_string().len().max(4);
    format!("L-{sequence:0width$}")
}

fn child_dirs(root: &Path) -> Result<Vec<PathBuf>, OrbitError> {
    let mut dirs = Vec::new();
    for entry in fs::read_dir(root).map_err(|error| OrbitError::Io(error.to_string()))? {
        let entry = entry.map_err(|error| OrbitError::Io(error.to_string()))?;
        let file_type = entry
            .file_type()
            .map_err(|error| OrbitError::Io(error.to_string()))?;
        if file_type.is_dir() {
            dirs.push(entry.path());
        }
    }
    dirs.sort();
    Ok(dirs)
}

fn yaml_epoch(path: &Path) -> Result<i64, OrbitError> {
    let value = read_yaml_value(path)?;
    let raw = value
        .as_mapping()
        .and_then(|mapping| mapping.get(Value::String("created_at".to_string())))
        .and_then(Value::as_str)
        .ok_or_else(|| OrbitError::Migration(format!("{}: missing created_at", path.display())))?;
    DateTime::parse_from_rfc3339(raw)
        .map(|timestamp| timestamp.timestamp())
        .map_err(|error| {
            OrbitError::Migration(format!(
                "{}: invalid created_at timestamp `{raw}`: {error}",
                path.display()
            ))
        })
}

fn read_yaml_value(path: &Path) -> Result<Value, OrbitError> {
    let raw = fs::read_to_string(path)
        .map_err(|error| OrbitError::Io(format!("read {}: {error}", path.display())))?;
    serde_yaml::from_str(&raw)
        .map_err(|error| OrbitError::Migration(format!("parse {}: {error}", path.display())))
}

fn rewrite_learning_yaml(
    value: &mut Value,
    old_id: &str,
    new_id: &str,
    rename_map: &BTreeMap<String, String>,
) -> Result<(), OrbitError> {
    let mapping = value
        .as_mapping_mut()
        .ok_or_else(|| OrbitError::Migration("learning yaml must be a mapping".to_string()))?;
    mapping.insert(
        Value::String("id".to_string()),
        Value::String(new_id.to_string()),
    );
    add_legacy_id(mapping, old_id);
    rewrite_value_strings(value, rename_map, false);
    Ok(())
}

fn add_legacy_id(mapping: &mut Mapping, old_id: &str) {
    let key = Value::String("legacy_ids".to_string());
    let entry = mapping
        .entry(key)
        .or_insert_with(|| Value::Sequence(Vec::new()));
    if let Value::Sequence(values) = entry
        && !values.iter().any(|value| value.as_str() == Some(old_id))
    {
        values.push(Value::String(old_id.to_string()));
    }
}

fn rewrite_value_strings(
    value: &mut Value,
    rename_map: &BTreeMap<String, String>,
    in_legacy_ids: bool,
) {
    match value {
        Value::String(raw) if !in_legacy_ids => {
            for (old_id, new_id) in rename_map {
                if raw.contains(old_id) {
                    *raw = raw.replace(old_id, new_id);
                }
            }
        }
        Value::Sequence(values) => {
            for item in values {
                rewrite_value_strings(item, rename_map, in_legacy_ids);
            }
        }
        Value::Mapping(mapping) => {
            for (key, child) in mapping {
                let child_in_legacy_ids = key.as_str() == Some("legacy_ids");
                rewrite_value_strings(child, rename_map, child_in_legacy_ids);
            }
        }
        _ => {}
    }
}

fn is_legacy_learning_id(id: &str) -> bool {
    let Some(rest) = id.strip_prefix('L') else {
        return false;
    };
    let Some((date, suffix)) = rest.split_once('-') else {
        return false;
    };
    date.len() == 8
        && date.as_bytes().iter().all(u8::is_ascii_digit)
        && !suffix.is_empty()
        && suffix.as_bytes().iter().all(u8::is_ascii_digit)
}

fn now_epoch() -> i64 {
    Utc::now().timestamp()
}

fn best_effort_branch(worktree_root: &Path) -> Option<String> {
    match current_branch(worktree_root).ok()? {
        CurrentBranchStatus::Named(branch) => Some(branch),
        CurrentBranchStatus::DetachedHead | CurrentBranchStatus::NoCurrentBranch => None,
    }
}

fn absolutize(path: PathBuf) -> PathBuf {
    fs::canonicalize(&path).unwrap_or(path)
}

fn relative_to(path: &Path, root: &Path) -> PathBuf {
    let path = absolutize(path.to_path_buf());
    let root = absolutize(root.to_path_buf());
    path.strip_prefix(&root)
        .map(Path::to_path_buf)
        .unwrap_or(path)
}

fn enable_best_effort_wal_mode(conn: &Connection) {
    match conn.pragma_update_and_check(None, "journal_mode", "WAL", |row| row.get::<_, String>(0)) {
        Ok(mode) if mode.eq_ignore_ascii_case("wal") => {}
        Ok(mode) => {
            orbit_common::tracing::warn!(
                target: "orbit.store.id_allocator",
                journal_mode = mode.as_str(),
                "requested WAL mode on semantic database, but SQLite kept the active journal mode",
            );
        }
        Err(error) => {
            orbit_common::tracing::warn!(
                target: "orbit.store.id_allocator",
                error = %error,
                "could not set WAL mode on semantic database; continuing with default journal mode",
            );
        }
    }
}

#[cfg(test)]
#[cfg(test)]
mod tests;
