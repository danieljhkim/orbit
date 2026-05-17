// ORB-00013: Existing expect calls in this module document local invariants; keep the allow scoped while the workspace lint is ratcheted.
#![allow(clippy::expect_used)]

use chrono::Utc;
use orbit_common::types::{LearningStatus, NotFoundKind, OrbitError};

use super::super::layout::{locate_learning, validate_learning_id};
use super::super::lock::{acquire_learning_allocation_lock, acquire_learning_lock};
use super::super::record::{read_learning_file, write_learning_file};
use super::store::LearningFileStore;

impl LearningFileStore {
    /// Atomically supersede `old_id` with `new_id`. Phase-1 contract:
    /// 1. Both records exist.
    /// 2. `old.status` flips to `Superseded` and `old.superseded_by = new_id`.
    /// 3. `new.supersedes = old_id`.
    /// 4. Both index rows reflect the new state.
    ///
    /// All four steps run inside a single allocation-lock window so concurrent
    /// readers see either the pre- or post-state, not a mid-state.
    pub(crate) fn supersede_learning(&self, old_id: &str, new_id: &str) -> Result<(), OrbitError> {
        validate_learning_id(old_id)?;
        validate_learning_id(new_id)?;
        if old_id == new_id {
            return Err(OrbitError::InvalidInput(format!(
                "learning '{old_id}' cannot supersede itself"
            )));
        }

        // Take the allocation lock so the two-file mutation appears atomic
        // to anyone holding only the per-id locks (we hold both per-id locks
        // too, but the allocation lock guards listings against concurrent
        // create_learning).
        let _allocation_lock = acquire_learning_allocation_lock(&self.root)?;
        let _old_lock = acquire_learning_lock(&self.root, old_id)?;
        let _new_lock = acquire_learning_lock(&self.root, new_id)?;

        let old_path = locate_learning(&self.root, old_id)?
            .ok_or_else(|| OrbitError::not_found(NotFoundKind::Learning, old_id.to_string()))?;
        let new_path = locate_learning(&self.root, new_id)?
            .ok_or_else(|| OrbitError::not_found(NotFoundKind::Learning, new_id.to_string()))?;

        let mut old = read_learning_file(&old_path)?;
        let mut new = read_learning_file(&new_path)?;

        let now = Utc::now();
        old.status = LearningStatus::Superseded;
        old.superseded_by = Some(new_id.to_string());
        old.updated_at = now;

        new.supersedes = Some(old_id.to_string());
        new.updated_at = now;

        // 1. Write the updated `new` record first; if anything below fails
        //    we can still recover the old state by re-reading from disk.
        write_learning_file(&new_path, &new, new.status)?;
        // 2. Write the updated `old` content at its stable per-entity path.
        write_learning_file(&old_path, &old, LearningStatus::Superseded)?;

        self.upsert_index_row(&old);
        self.upsert_index_row(&new);
        self.invalidate_envelope_cache();
        Ok(())
    }

    /// Archive a learning without a replacement: flip `status` to
    /// `Superseded` with `superseded_by = None` and mirror the state into the index. Used by the
    /// §7.3 `prune --delete` semantics: stale records are archived, not
    /// hard-deleted.
    pub(crate) fn archive_learning(&self, id: &str) -> Result<bool, OrbitError> {
        validate_learning_id(id)?;
        let _allocation_lock = acquire_learning_allocation_lock(&self.root)?;
        let _lock = acquire_learning_lock(&self.root, id)?;

        let Some(path) = locate_learning(&self.root, id)? else {
            return Ok(false);
        };
        let mut learning = read_learning_file(&path)?;
        if learning.status == LearningStatus::Superseded {
            // Already archived; idempotent no-op.
            return Ok(true);
        }
        learning.status = LearningStatus::Superseded;
        learning.superseded_by = None;
        learning.updated_at = Utc::now();

        write_learning_file(&path, &learning, LearningStatus::Superseded)?;
        self.upsert_index_row(&learning);
        self.invalidate_envelope_cache();
        Ok(true)
    }

    pub(crate) fn delete_learning(&self, id: &str) -> Result<bool, OrbitError> {
        validate_learning_id(id)?;
        let _lock = acquire_learning_lock(&self.root, id)?;

        let Some(path) = locate_learning(&self.root, id)? else {
            return Ok(false);
        };
        std::fs::remove_file(&path).map_err(|e| OrbitError::Io(e.to_string()))?;
        if let Some(parent) = path.parent()
            && parent
                .read_dir()
                .map(|mut entries| entries.next().is_none())
                .unwrap_or(false)
        {
            std::fs::remove_dir(parent).map_err(|e| OrbitError::Io(e.to_string()))?;
        }
        if let Some(index) = &self.index {
            index.delete_learning_index_row(id)?;
        }
        self.invalidate_envelope_cache();
        Ok(true)
    }
}
