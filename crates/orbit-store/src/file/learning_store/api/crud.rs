// Existing expect calls in this module document local invariants; keep the allow scoped while the workspace lint is ratcheted.
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

        let mut scope = params.scope;
        scope.paths = normalize_learning_paths(scope.paths);
        scope.tags = normalize_learning_tags(scope.tags);

        loop {
            let id = self.id_allocator.allocate_learning()?.id;
            let learning = Learning {
                id: id.clone(),
                status: LearningStatus::Active,
                scope: scope.clone(),
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
            };

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

    /// [ORB-10330] Finalize a hub-preallocated learning at the exact
    /// caller-supplied canonical `id` in this checkout-bound store.
    ///
    /// The multi-host counterpart of [`Self::create_learning`]: the id comes
    /// from ORB-10272's hub sequence, so this path has no allocation loop and
    /// never selects, abandons, retries, or requests a second id. A path
    /// collision fails deterministically and preserves the existing artifact
    /// (never adopts or overwrites it); a failure after the body is written
    /// removes only the local partial body and its projection, never the
    /// consumed hub allocation.
    pub(crate) fn finalize_preallocated_learning(
        &self,
        id: &str,
        params: LearningCreateParams,
    ) -> Result<Learning, OrbitError> {
        self.finalize_preallocated_learning_at(id, params, Utc::now())
    }

    pub(crate) fn finalize_preallocated_learning_at(
        &self,
        id: &str,
        params: LearningCreateParams,
        now: DateTime<Utc>,
    ) -> Result<Learning, OrbitError> {
        validate_learning_id(id)?;
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

        let mut scope = params.scope;
        scope.paths = normalize_learning_paths(scope.paths);
        scope.tags = normalize_learning_tags(scope.tags);

        let learning = Learning {
            id: id.to_string(),
            status: LearningStatus::Active,
            scope,
            summary: params.summary,
            body: params.body,
            evidence: params.evidence,
            supersedes: None,
            superseded_by: None,
            legacy_ids: Vec::new(),
            created_at: now,
            updated_at: now,
            created_by: params.created_by,
            priority: params.priority,
        };

        let path = learning_doc_path(&self.root, id);
        match create_learning_file_exclusive(&path, &learning, LearningStatus::Active) {
            Ok(true) => match self.finalize_preallocated_body(id, &path, &learning) {
                Ok(()) => Ok(learning),
                Err(error) => {
                    self.cleanup_preallocated_learning(id, &path);
                    Err(error)
                }
            },
            // A pre-existing artifact is never adopted, overwritten, or retried.
            Ok(false) => Err(OrbitError::InvalidInput(format!(
                "cannot finalize preallocated learning {id}: an artifact already exists at this id",
            ))),
            Err(error) => {
                // The exclusive create failed with an IO error — a pre-existing
                // file surfaces as `Ok(false)`, not `Err`, so any partial file
                // here is ours to clean up.
                let _ = std::fs::remove_file(&path);
                if let Some(dir) = path.parent() {
                    let _ = std::fs::remove_dir(dir);
                }
                Err(error)
            }
        }
    }

    /// Install the owner-local projection and index the finalized learning. The
    /// only fallible step is the projection insert; a returned `Err` therefore
    /// leaves no projection row (the insert is atomic), so the partial body is
    /// the sole local residue for [`Self::cleanup_preallocated_learning`].
    fn finalize_preallocated_body(
        &self,
        id: &str,
        path: &std::path::Path,
        learning: &Learning,
    ) -> Result<(), OrbitError> {
        self.id_allocator.project_preallocated_learning(id, path)?;
        self.upsert_index_row(learning);
        self.invalidate_envelope_cache();
        Ok(())
    }

    /// [ORB-10330] Remove a partially-finalized preallocated learning: the
    /// staged body and its now-empty id directory. The projection insert is the
    /// last fallible step, so a failed finalization never left a projection row
    /// of its own to remove; and this never abandons or renumbers the hub
    /// allocation, which stays consumed.
    fn cleanup_preallocated_learning(&self, _id: &str, path: &std::path::Path) {
        let _ = std::fs::remove_file(path);
        if let Some(dir) = path.parent() {
            let _ = std::fs::remove_dir(dir);
        }
        self.invalidate_envelope_cache();
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

    /// [ORB-10501] Learning allocations that can never resolve to a body
    /// again: the worktree they were pinned to is gone from disk *and* no
    /// canonical or recorded copy of the body is readable here.
    ///
    /// Both conditions are required. A live sibling worktree is an ordinary
    /// remote stub, and a canonically-present body makes a stale
    /// `worktree_root` harmless — neither is dead weight.
    pub(crate) fn list_orphaned_learning_allocations(
        &self,
    ) -> Result<Vec<IdAllocationRecord>, OrbitError> {
        let mut orphaned = Vec::new();
        for record in self.id_allocator.learning_allocations()? {
            if !record.worktree_is_missing() {
                continue;
            }
            if self.read_learning_allocation(&record)?.is_some() {
                continue;
            }
            orphaned.push(record);
        }
        Ok(orphaned)
    }

    /// [ORB-10501] Clear one orphaned allocation row, re-verifying both orphan
    /// conditions immediately before the write. Returns `false` when `id` has
    /// no live allocation row; refuses with `InvalidInput` when the row is
    /// still recoverable, so a caller working from a stale scan cannot retire
    /// a readable learning.
    pub(crate) fn abandon_orphaned_learning_allocation(
        &self,
        id: &str,
    ) -> Result<bool, OrbitError> {
        validate_learning_id(id)?;
        let Some(record) = self.id_allocator.learning_allocation(id)? else {
            return Ok(false);
        };
        if !record.worktree_is_missing() {
            return Err(OrbitError::InvalidInput(format!(
                "learning '{id}' is not orphaned: its recorded worktree '{}' still exists",
                record.worktree_root.display()
            )));
        }
        if self.read_learning_allocation(&record)?.is_some() {
            return Err(OrbitError::InvalidInput(format!(
                "learning '{id}' is not orphaned: its body is still readable"
            )));
        }
        if !self.id_allocator.abandon_orphaned_learning(id)? {
            return Ok(false);
        }
        // The envelope index row is pinned to the same dead body; leaving it
        // behind is the stale-`reserved`-row half of F2026-07-094.
        if let Some(index) = &self.index {
            index.delete_learning_index_row(&self.workspace_id, id)?;
        }
        self.invalidate_envelope_cache();
        Ok(true)
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
        // superseded learnings as active. The `body_path` is a
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
