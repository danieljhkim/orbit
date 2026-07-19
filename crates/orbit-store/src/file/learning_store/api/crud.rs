// ORB-00013: Existing expect calls in this module document local invariants; keep the allow scoped while the workspace lint is ratcheted.
#![allow(clippy::expect_used)]

use chrono::{DateTime, Utc};
use orbit_common::types::{
    Learning, LearningStatus, NotFoundKind, OrbitError, normalize_learning_paths,
    normalize_learning_tags,
};

use super::super::layout::{learning_doc_path, locate_learning, validate_learning_id};
use super::super::record::{
    create_learning_file_exclusive, read_learning_file, write_learning_file,
};
use super::store::LearningFileStore;
use crate::backend::{LearningCreateParams, LearningUpdateParams};
use crate::{IdAllocationRecord, LearningListEntry, RemoteArtifactStub};

impl LearningFileStore {
    pub(crate) fn create_learning(
        &self,
        params: LearningCreateParams,
    ) -> Result<Learning, OrbitError> {
        self.create_learning_at(params, Utc::now())
    }

    /// Test-only entry point that injects the allocation clock so id-format
    /// tests can assert deterministic dates without sleeping.
    pub(crate) fn create_learning_at(
        &self,
        params: LearningCreateParams,
        now: DateTime<Utc>,
    ) -> Result<Learning, OrbitError> {
        let params = normalize_learning_create_params(params)?;

        loop {
            let id = self.id_allocator.allocate_learning()?.id;
            let learning = new_learning(id.clone(), &params, now);

            let path = learning_doc_path(&self.root, &id);

            // [ORB-00413] The SQLite reservation from `allocate_learning` is the
            // source of truth for the ID; guard the body/sidecar/record steps so
            // a partial create rolls the reservation back rather than leaving a
            // half-visible ID (reserved-without-body) or an orphaned body file.
            // A learning after this point either fully exists (allocated + body
            // present + indexed) or not at all.
            match create_learning_file_exclusive(&path, &learning, LearningStatus::Active) {
                Ok(true) => match self.finalize_created_learning(&id, &path, &learning) {
                    Ok(()) => return Ok(learning),
                    Err(error) => {
                        self.rollback_partial_learning(&id, &path);
                        return Err(error);
                    }
                },
                Ok(false) => {
                    // The allocated id's path already exists: adopt it (same id)
                    // or reject (different id). `adopt_or_reject_existing_learning_path`
                    // owns cleanup of the reservation in the reject case, so no
                    // body of ours is left behind.
                    self.adopt_or_reject_existing_learning_path(&id, &path)?;
                    continue;
                }
                Err(error) => {
                    // The exclusive create failed with an IO error — a
                    // pre-existing file surfaces as `Ok(false)`, not `Err`, so
                    // any partial file here is ours to clean up.
                    self.rollback_partial_learning(&id, &path);
                    return Err(error);
                }
            }
        }
    }

    /// Finalize exactly one learning ID allocated by the hub in this store's
    /// already-bound checkout. Supplied-ID collisions are deterministic
    /// failures; this path never adopts, abandons, retries, or allocates.
    pub(crate) fn finalize_preallocated_learning(
        &self,
        id: &str,
        params: LearningCreateParams,
    ) -> Result<Learning, OrbitError> {
        validate_learning_id(id)?;
        let params = normalize_learning_create_params(params)?;
        let learning = new_learning(id.to_string(), &params, Utc::now());
        let _lock = super::super::lock::acquire_learning_lock(&self.root, id)?;
        let path = learning_doc_path(&self.root, id);
        if path.exists() || self.id_allocator.learning_allocation(id)?.is_some() {
            return Err(preallocated_learning_collision(id));
        }
        if !create_learning_file_exclusive(&path, &learning, LearningStatus::Active)? {
            return Err(preallocated_learning_collision(id));
        }
        if let Err(error) = self
            .id_allocator
            .install_preallocated_learning_projection(id, &path)
        {
            remove_learning_body(&path);
            return Err(OrbitError::Store(format!(
                "finalize preallocated learning {id}: {error}"
            )));
        }
        if let Err(error) = self.upsert_index_row_strict(&learning) {
            let cleanup = self.cleanup_preallocated_learning(id, &path);
            return match cleanup {
                Ok(()) => Err(error),
                Err(cleanup_error) => Err(OrbitError::Store(format!(
                    "finalize preallocated learning {id} failed: {error}; cleanup failed: {cleanup_error}"
                ))),
            };
        }
        self.invalidate_envelope_cache();
        Ok(learning)
    }

    pub(crate) fn rollback_preallocated_learning(&self, id: &str) -> Result<bool, OrbitError> {
        validate_learning_id(id)?;
        let _lock = super::super::lock::acquire_learning_lock(&self.root, id)?;
        let Some(record) = self.id_allocator.learning_allocation(id)? else {
            return Ok(false);
        };
        if !record.is_projection || record.worktree_root != self.id_allocator.worktree_root() {
            return Err(OrbitError::Store(format!(
                "refusing to roll back non-projection learning {id}"
            )));
        }
        let path = learning_doc_path(&self.root, id);
        self.cleanup_preallocated_learning(id, &path)?;
        Ok(true)
    }

    fn cleanup_preallocated_learning(
        &self,
        id: &str,
        path: &std::path::Path,
    ) -> Result<(), OrbitError> {
        self.delete_index_row_strict(id)?;
        remove_learning_body(path);
        if !self
            .id_allocator
            .remove_preallocated_learning_projection(id)?
        {
            return Err(OrbitError::Store(format!(
                "owner-local learning projection missing during cleanup for {id}"
            )));
        }
        self.invalidate_envelope_cache();
        Ok(())
    }

    /// Finalize a freshly-created learning body: write its empty sidecars,
    /// record the allocation's body path, and refresh the index. The last
    /// fallible step is `record_learning_body_path`; everything after it is
    /// infallible, so a returned `Err` always leaves the reservation's
    /// `body_path` unset and thus abandonable by [`Self::rollback_partial_learning`].
    fn finalize_created_learning(
        &self,
        id: &str,
        path: &std::path::Path,
        learning: &Learning,
    ) -> Result<(), OrbitError> {
        self.id_allocator.record_learning_body_path(id, path)?;
        self.upsert_index_row(learning);
        self.invalidate_envelope_cache();
        Ok(())
    }

    /// [ORB-00413] Best-effort rollback of a partially-created learning: remove
    /// the staged body, drop the now-empty id directory, and abandon the
    /// reservation so the ID is never left half-visible. Never fails the caller
    /// — the original error is what propagates — and only logs if the abandon
    /// itself fails.
    fn rollback_partial_learning(&self, id: &str, path: &std::path::Path) {
        let _ = std::fs::remove_file(path);
        if let Some(dir) = path.parent() {
            // Only succeeds if the directory is now empty — leaves any
            // pre-existing/foreign content in place.
            let _ = std::fs::remove_dir(dir);
        }
        if let Err(error) = self.id_allocator.abandon_learning(id) {
            orbit_common::tracing::warn!(
                target: "orbit.store.learning",
                id,
                error = %error,
                "rollback: failed to abandon reserved learning after a partial create",
            );
        }
        self.invalidate_envelope_cache();
    }

    pub(crate) fn get_learning(&self, id: &str) -> Result<Option<Learning>, OrbitError> {
        validate_learning_id(id)?;
        let Some(path) = locate_learning(&self.root, id)? else {
            return Ok(None);
        };
        Ok(Some(read_learning_file(&path)?))
    }

    pub(crate) fn get_learning_federated(&self, id: &str) -> Result<Option<Learning>, OrbitError> {
        validate_learning_id(id)?;
        if let Some(record) = self.id_allocator.learning_allocation(id)?
            && let Some(learning) = self.read_learning_allocation(&record)?
        {
            return Ok(Some(learning));
        }
        self.get_learning(id)
    }

    pub(crate) fn list_learnings(
        &self,
        status: Option<LearningStatus>,
    ) -> Result<Vec<Learning>, OrbitError> {
        let mut out = Vec::new();
        if !self.root.exists() {
            return Ok(out);
        }
        for entry in std::fs::read_dir(&self.root).map_err(|e| OrbitError::Io(e.to_string()))? {
            let entry = entry.map_err(|e| OrbitError::Io(e.to_string()))?;
            let file_type = entry
                .file_type()
                .map_err(|e| OrbitError::Io(e.to_string()))?;
            if !file_type.is_dir() {
                continue;
            }
            let Some(id) = entry.file_name().to_str().map(str::to_string) else {
                continue;
            };
            if validate_learning_id(&id).is_err() {
                continue;
            }
            let path = learning_doc_path(&self.root, &id);
            if !path.is_file() {
                continue;
            }
            let learning = read_learning_file(&path)?;
            if let Some(s) = status
                && learning.status != s
            {
                continue;
            }
            out.push(learning);
        }
        out.sort_by(|a, b| {
            b.updated_at
                .cmp(&a.updated_at)
                .then_with(|| a.id.cmp(&b.id))
        });
        Ok(out)
    }

    pub(crate) fn list_learning_entries(
        &self,
        status: Option<LearningStatus>,
        include_remote: bool,
    ) -> Result<Vec<LearningListEntry>, OrbitError> {
        let mut entries = Vec::new();
        for record in self.id_allocator.learning_allocations()? {
            if let Some(learning) = self.read_learning_allocation(&record)? {
                if status.is_none_or(|expected| learning.status == expected) {
                    entries.push(LearningListEntry::Local(learning));
                }
                continue;
            }
            if include_remote && status.is_none() {
                entries.push(LearningListEntry::Remote(remote_stub_from_allocation(
                    &record,
                )));
            }
        }
        entries.sort_by(|left, right| learning_entry_id(right).cmp(learning_entry_id(left)));
        Ok(entries)
    }

    pub(crate) fn get_learning_remote_stub(
        &self,
        id: &str,
    ) -> Result<Option<RemoteArtifactStub>, OrbitError> {
        validate_learning_id(id)?;
        let Some(record) = self.id_allocator.learning_allocation(id)? else {
            return Ok(None);
        };
        if self.read_learning_allocation(&record)?.is_some() {
            return Ok(None);
        }
        Ok(Some(remote_stub_from_allocation(&record)))
    }

    pub(crate) fn update_learning(
        &self,
        id: &str,
        params: LearningUpdateParams,
    ) -> Result<Learning, OrbitError> {
        validate_learning_id(id)?;
        let _lock = super::super::lock::acquire_learning_lock(&self.root, id)?;

        let Some(path) = locate_learning(&self.root, id)? else {
            return Err(OrbitError::not_found(
                NotFoundKind::Learning,
                id.to_string(),
            ));
        };
        let mut learning = read_learning_file(&path)?;

        if learning.status == LearningStatus::Superseded {
            return Err(OrbitError::InvalidInput(format!(
                "learning '{id}' is superseded and cannot be updated"
            )));
        }

        if let Some(summary) = params.summary {
            if summary.chars().count() > 280 {
                return Err(OrbitError::InvalidInput(format!(
                    "learning summary must be at most 280 characters (got {})",
                    summary.chars().count()
                )));
            }
            learning.summary = summary;
        }
        if let Some(mut scope) = params.scope {
            scope.paths = normalize_learning_paths(scope.paths);
            scope.tags = normalize_learning_tags(scope.tags);
            learning.scope = scope;
        }
        if let Some(body) = params.body {
            learning.body = body;
        }
        if let Some(evidence) = params.evidence {
            learning.evidence = evidence;
        }
        if let Some(priority) = params.priority {
            learning.priority = priority;
        }
        learning.updated_at = Utc::now();
        write_learning_file(&path, &learning, learning.status)?;
        self.upsert_index_row(&learning);
        self.invalidate_envelope_cache();
        Ok(learning)
    }

    fn read_learning_allocation(
        &self,
        record: &IdAllocationRecord,
    ) -> Result<Option<Learning>, OrbitError> {
        // ORB-00373: the canonical copy under `self.root` is the source of
        // truth (docs/design/project-learnings/2_design.md). Resolve it FIRST,
        // ahead of the allocator's recorded `body_path`. A learning first
        // allocated inside a job-run worktree records that worktree's path; a
        // later supersede/update/sync rewrites only the canonical `self.root`
        // copy (+ the SQLite index), leaving stale worktree copies behind. If
        // we read the recorded `body_path` first, `list`/`show` report
        // superseded learnings as active (F2026-06-001). The `body_path` is a
        // fallback only for learnings genuinely absent from `self.root`
        // (sibling-worktree / remote stubs).
        if let Some(local) = self.get_learning(&record.id)? {
            return Ok(Some(local));
        }
        let Some(path) = record.resolved_body_path() else {
            return Ok(None);
        };
        if !path.is_file() {
            return Ok(None);
        }
        Ok(Some(read_learning_file(&path)?))
    }

    fn adopt_or_reject_existing_learning_path(
        &self,
        id: &str,
        path: &std::path::Path,
    ) -> Result<(), OrbitError> {
        let existing = match read_learning_file(path) {
            Ok(existing) => existing,
            Err(error) => {
                self.id_allocator.abandon_learning(id)?;
                return Err(OrbitError::Store(format!(
                    "allocated learning id {id} conflicts with unreadable existing path '{}': {error}",
                    path.display()
                )));
            }
        };
        if existing.id != id {
            self.id_allocator.abandon_learning(id)?;
            return Err(OrbitError::Store(format!(
                "allocated learning id {id} conflicts with existing path '{}' containing learning '{}'",
                path.display(),
                existing.id
            )));
        }
        self.id_allocator.record_learning_body_path(id, path)?;
        self.upsert_index_row(&existing);
        self.invalidate_envelope_cache();
        Ok(())
    }
}

fn normalize_learning_create_params(
    mut params: LearningCreateParams,
) -> Result<LearningCreateParams, OrbitError> {
    if params.summary.trim().is_empty() {
        return Err(OrbitError::InvalidInput(
            "learning summary must not be empty".to_string(),
        ));
    }
    if params.summary.chars().count() > 280 {
        return Err(OrbitError::InvalidInput(format!(
            "learning summary must be at most 280 characters (got {})",
            params.summary.chars().count()
        )));
    }
    params.scope.paths = normalize_learning_paths(params.scope.paths);
    params.scope.tags = normalize_learning_tags(params.scope.tags);
    Ok(params)
}

fn new_learning(id: String, params: &LearningCreateParams, now: DateTime<Utc>) -> Learning {
    Learning {
        id,
        status: LearningStatus::Active,
        scope: params.scope.clone(),
        summary: params.summary.clone(),
        body: params.body.clone(),
        evidence: params.evidence.clone(),
        supersedes: None,
        superseded_by: None,
        legacy_ids: Vec::new(),
        created_at: now,
        updated_at: now,
        created_by: params.created_by.clone(),
        priority: params.priority,
    }
}

fn preallocated_learning_collision(id: &str) -> OrbitError {
    OrbitError::Store(format!(
        "preallocated learning {id} already has an owner-local artifact or allocation projection; refusing overwrite, adoption, or replacement ID"
    ))
}

fn remove_learning_body(path: &std::path::Path) {
    let _ = std::fs::remove_file(path);
    if let Some(dir) = path.parent() {
        let _ = std::fs::remove_dir(dir);
    }
}

fn remote_stub_from_allocation(record: &IdAllocationRecord) -> RemoteArtifactStub {
    RemoteArtifactStub {
        id: record.id.clone(),
        kind: record.kind.as_str().to_string(),
        status: record.status.clone(),
        worktree_root: record.worktree_root.clone(),
        branch: record.branch.clone(),
        body_path: record.body_path.clone(),
    }
}

fn learning_entry_id(entry: &LearningListEntry) -> &str {
    match entry {
        LearningListEntry::Local(learning) => &learning.id,
        LearningListEntry::Remote(stub) => &stub.id,
    }
}
