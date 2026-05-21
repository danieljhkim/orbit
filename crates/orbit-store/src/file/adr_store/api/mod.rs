// ORB-00013: Existing expect calls in this module document local invariants; keep the allow scoped while the workspace lint is ratcheted.
#![allow(clippy::expect_used)]

use std::fs;
use std::path::{Path, PathBuf};

use chrono::Utc;
use orbit_common::types::{
    Adr, AdrStatus, LegacyValidation, NotFoundKind, OrbitError, normalize_adr_paths,
    normalize_adr_tags,
};
use orbit_common::utility::glob::{compile_glob_regex, normalize_glob_path};
use rusqlite::params;

use super::bundle::{AdrBundle, bundle_to_adr, read_bundle_at, validate_bundle, write_bundle_at};
use super::constants::ADR_SCHEMA_VERSION;
use super::doc::AdrFileDocument;
use super::layout::{AdrStateDir, adr_dir, state_dir_path, validate_adr_id};
use super::lock::acquire_adr_lock;
use crate::backend::{AdrCreateParams, AdrDocumentUpdateParams};
use crate::file::layout::read_child_dirs;
use crate::{AdrListEntry, IdAllocationRecord, IdAllocator, RemoteArtifactStub, Store};

pub(crate) struct AdrFileStore {
    root: PathBuf,
    index: Option<Store>,
    id_allocator: IdAllocator,
}

impl AdrFileStore {
    #[cfg(test)]
    pub(crate) fn new(root: PathBuf) -> Self {
        let id_allocator = IdAllocator::for_test_roots(root.clone(), root.join(".learnings"));
        Self {
            root,
            index: None,
            id_allocator,
        }
    }

    #[cfg(test)]
    pub(crate) fn new_with_index(root: PathBuf, index: Store) -> Self {
        let id_allocator = IdAllocator::for_test_roots(root.clone(), root.join(".learnings"));
        Self::new_with_index_and_allocator(root, index, id_allocator)
    }

    pub(crate) fn new_with_index_and_allocator(
        root: PathBuf,
        index: Store,
        id_allocator: IdAllocator,
    ) -> Self {
        Self {
            root,
            index: Some(index),
            id_allocator,
        }
    }

    /// Creates a new ADR.
    ///
    /// Filesystem write happens first (atomic via `write_bundle_at`); if it
    /// succeeds and an index is attached, the envelope is upserted. A failed
    /// index write is logged but does **not** roll back the filesystem: the
    /// filesystem is the source of truth and `rebuild_index` can recover.
    /// The inverse direction (FS failure → index rollback) is implicit since
    /// the index INSERT is gated on FS success.
    pub(crate) fn add_adr(&self, params: AdrCreateParams) -> Result<Adr, OrbitError> {
        if params.title.trim().is_empty() {
            return Err(OrbitError::InvalidInput(
                "ADR title must not be empty".to_string(),
            ));
        }
        if params.owner.trim().is_empty() {
            return Err(OrbitError::InvalidInput(
                "ADR owner must not be empty".to_string(),
            ));
        }

        let id = self.id_allocator.allocate_adr()?.id;
        let now = Utc::now();
        let adr = Adr {
            id: id.clone(),
            title: params.title,
            status: AdrStatus::Proposed,
            owner: params.owner,
            created_at: now,
            accepted_at: None,
            last_updated: now,
            related_features: params.related_features,
            related_tasks: params.related_tasks,
            tags: normalize_adr_tags(params.tags),
            paths: normalize_adr_paths(params.paths),
            supersedes: Vec::new(),
            superseded_by: None,
            legacy_ids: Vec::new(),
            validation_warnings: Vec::new(),
            legacy_validation: LegacyValidation::None,
        };
        let bundle = AdrBundle {
            doc: AdrFileDocument {
                schema_version: ADR_SCHEMA_VERSION,
                adr,
            },
            body: params.body,
        };
        validate_bundle(&bundle)?;

        let target_dir = adr_dir(&self.root, AdrStateDir::Proposed, &id);
        write_bundle_at(&target_dir, &bundle)?;
        self.id_allocator
            .record_adr_body_path(&id, &super::layout::body_path(&target_dir))?;

        let adr = bundle_to_adr(bundle);
        self.upsert_index_row(&adr);
        Ok(adr)
    }

    pub(crate) fn get_adr(&self, id: &str) -> Result<Option<Adr>, OrbitError> {
        validate_adr_id(id)?;
        let Some((_, dir)) = self.locate_adr(id)? else {
            return Ok(None);
        };
        let bundle = read_bundle_at(&dir)?;
        Ok(Some(bundle_to_adr(bundle)))
    }

    pub(crate) fn get_adr_federated(&self, id: &str) -> Result<Option<Adr>, OrbitError> {
        validate_adr_id(id)?;
        if let Some(record) = self.id_allocator.adr_allocation(id)?
            && let Some(adr) = self.read_adr_allocation(&record)?
        {
            return Ok(Some(adr));
        }
        self.get_adr(id)
    }

    pub(crate) fn list_adrs(&self) -> Result<Vec<Adr>, OrbitError> {
        let mut adrs = Vec::new();
        for state in AdrStateDir::all() {
            let dir = state_dir_path(&self.root, *state);
            if !dir.exists() {
                continue;
            }
            for adr_dir_path in read_child_dirs(&dir)? {
                // Skip directories without an adr.yaml (e.g., partial / scratch dirs).
                let doc_path = adr_dir_path.join(super::constants::ADR_YAML);
                if !doc_path.is_file() {
                    continue;
                }
                let bundle = read_bundle_at(&adr_dir_path)?;
                adrs.push(bundle_to_adr(bundle));
            }
        }
        Ok(adrs)
    }

    /// Lists ADRs filtered by envelope fields.
    ///
    /// When an index is attached, the filter pushes WHERE clauses down to
    /// SQLite, then reads each matching bundle from disk (filesystem is the
    /// source of truth). Without an index, the call falls back to listing
    /// every ADR and filtering in-memory.
    ///
    /// IDs are sorted lexicographic-descending, which matches numeric
    /// descending given the `ADR-NNNN` zero-padded format.
    pub(crate) fn list_adrs_filtered(
        &self,
        status: Option<AdrStatus>,
        owner: Option<&str>,
        feature: Option<&str>,
        task_id: Option<&str>,
        legacy_id: Option<&str>,
        tag: Option<&str>,
        path: Option<&str>,
        validation_warned: Option<bool>,
    ) -> Result<Vec<Adr>, OrbitError> {
        let normalized_path = normalized_filter_path(path)?;
        if self.index.is_none() {
            // Fall back to filesystem walk + in-memory filter. Preserves
            // test ergonomics where `AdrFileStore::new` skips the index.
            let mut adrs = self.list_adrs()?;
            adrs.retain(|adr| {
                matches_filter(
                    adr,
                    status,
                    owner,
                    feature,
                    task_id,
                    legacy_id,
                    tag,
                    normalized_path.as_deref(),
                    validation_warned,
                )
            });
            sort_by_id_desc(&mut adrs);
            return Ok(adrs);
        }

        let ids = self.query_filtered_ids(
            status,
            owner,
            feature,
            task_id,
            legacy_id,
            validation_warned,
        )?;

        let mut adrs = Vec::with_capacity(ids.len());
        for id in ids {
            if let Some(adr) = self.get_adr(&id)?
                && matches_filter(
                    &adr,
                    status,
                    owner,
                    feature,
                    task_id,
                    legacy_id,
                    tag,
                    normalized_path.as_deref(),
                    validation_warned,
                )
            {
                adrs.push(adr);
            }
        }
        sort_by_id_desc(&mut adrs);
        Ok(adrs)
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn list_adr_entries_filtered(
        &self,
        status: Option<AdrStatus>,
        owner: Option<&str>,
        feature: Option<&str>,
        task_id: Option<&str>,
        legacy_id: Option<&str>,
        tag: Option<&str>,
        path: Option<&str>,
        validation_warned: Option<bool>,
        include_remote: bool,
    ) -> Result<Vec<AdrListEntry>, OrbitError> {
        let normalized_path = normalized_filter_path(path)?;
        let mut entries = Vec::new();
        for record in self.id_allocator.adr_allocations()? {
            if let Some(adr) = self.read_adr_allocation(&record)? {
                if matches_filter(
                    &adr,
                    status,
                    owner,
                    feature,
                    task_id,
                    legacy_id,
                    tag,
                    normalized_path.as_deref(),
                    validation_warned,
                ) {
                    entries.push(AdrListEntry::Local(adr));
                }
                continue;
            }

            if include_remote
                && remote_adr_matches_filter(
                    &record,
                    status,
                    owner,
                    feature,
                    task_id,
                    legacy_id,
                    tag,
                    normalized_path.as_deref(),
                    validation_warned,
                )
            {
                entries.push(AdrListEntry::Remote(remote_stub_from_allocation(&record)));
            }
        }
        entries.sort_by(|left, right| adr_entry_id(right).cmp(adr_entry_id(left)));
        Ok(entries)
    }

    pub(crate) fn get_adr_remote_stub(
        &self,
        id: &str,
    ) -> Result<Option<RemoteArtifactStub>, OrbitError> {
        validate_adr_id(id)?;
        let Some(record) = self.id_allocator.adr_allocation(id)? else {
            return Ok(None);
        };
        if self.read_adr_allocation(&record)?.is_some() {
            return Ok(None);
        }
        Ok(Some(remote_stub_from_allocation(&record)))
    }

    /// Updates ADR status, moving the bundle between state directories and
    /// refreshing the index row. Index update is best-effort: a failure is
    /// logged but does not roll back the filesystem move.
    pub(crate) fn update_adr_status(
        &self,
        id: &str,
        new_status: AdrStatus,
    ) -> Result<(), OrbitError> {
        validate_adr_id(id)?;
        let _lock = acquire_adr_lock(&self.root, id)?;

        let Some((current_state, current_dir)) = self.locate_adr(id)? else {
            return Err(OrbitError::not_found(NotFoundKind::Adr, id.to_string()));
        };
        let current_status = current_state.to_status();
        if current_status == new_status {
            return Ok(());
        }
        AdrStatus::validate_transition(current_status, new_status)?;

        let target_state = AdrStateDir::from_status(new_status);
        let target_dir = adr_dir(&self.root, target_state, id);
        if let Some(parent) = target_dir.parent() {
            fs::create_dir_all(parent).map_err(|e| OrbitError::Io(e.to_string()))?;
        }
        fs::rename(&current_dir, &target_dir).map_err(|e| OrbitError::Io(e.to_string()))?;

        let mut bundle = read_bundle_at(&target_dir)?;
        let now = Utc::now();
        bundle.doc.adr.status = new_status;
        bundle.doc.adr.last_updated = now;
        if new_status == AdrStatus::Accepted && bundle.doc.adr.accepted_at.is_none() {
            bundle.doc.adr.accepted_at = Some(now);
        }
        write_bundle_at(&target_dir, &bundle)?;
        self.id_allocator
            .record_adr_body_path(id, &super::layout::body_path(&target_dir))?;
        self.upsert_index_row(&bundle.doc.adr);
        Ok(())
    }

    /// Updates an ADR's document fields in place and refreshes the index row.
    /// Index update is best-effort: a failure is logged but does not roll back
    /// the filesystem write.
    pub(crate) fn update_adr_document(
        &self,
        id: &str,
        fields: &AdrDocumentUpdateParams,
    ) -> Result<(), OrbitError> {
        validate_adr_id(id)?;
        let _lock = acquire_adr_lock(&self.root, id)?;

        let Some((_, current_dir)) = self.locate_adr(id)? else {
            return Err(OrbitError::not_found(NotFoundKind::Adr, id.to_string()));
        };
        let mut bundle = read_bundle_at(&current_dir)?;

        if let Some(ref title) = fields.title {
            bundle.doc.adr.title = title.clone();
        }
        if let Some(ref owner) = fields.owner {
            bundle.doc.adr.owner = owner.clone();
        }
        if let Some(ref body) = fields.body {
            bundle.body = body.clone();
        }
        if let Some(ref features) = fields.related_features {
            bundle.doc.adr.related_features = features.clone();
        }
        if let Some(ref tasks) = fields.related_tasks {
            bundle.doc.adr.related_tasks = tasks.clone();
        }
        if let Some(ref tags) = fields.tags {
            bundle.doc.adr.tags = normalize_adr_tags(tags.clone());
        }
        if let Some(ref paths) = fields.paths {
            bundle.doc.adr.paths = normalize_adr_paths(paths.clone());
        }
        if let Some(ref supersedes) = fields.supersedes {
            bundle.doc.adr.supersedes = supersedes.clone();
        }
        if let Some(ref superseded_by) = fields.superseded_by {
            bundle.doc.adr.superseded_by = superseded_by.clone();
        }
        if let Some(ref legacy_ids) = fields.legacy_ids {
            bundle.doc.adr.legacy_ids = legacy_ids.clone();
        }
        if let Some(ref warnings) = fields.validation_warnings {
            bundle.doc.adr.validation_warnings = warnings.clone();
        }
        if let Some(legacy_validation) = fields.legacy_validation {
            bundle.doc.adr.legacy_validation = legacy_validation;
        }

        bundle.doc.adr.last_updated = Utc::now();
        write_bundle_at(&current_dir, &bundle)?;
        self.upsert_index_row(&bundle.doc.adr);
        Ok(())
    }

    /// Deletes an ADR bundle and removes the index row. Index delete is
    /// best-effort: a failure is logged but does not roll back the filesystem
    /// delete.
    pub(crate) fn delete_adr(&self, id: &str) -> Result<bool, OrbitError> {
        validate_adr_id(id)?;
        let _lock = acquire_adr_lock(&self.root, id)?;

        let Some((_, dir)) = self.locate_adr(id)? else {
            return Ok(false);
        };
        fs::remove_dir_all(dir).map_err(|e| OrbitError::Io(e.to_string()))?;
        self.delete_index_row(id);
        Ok(true)
    }

    /// Writes the bidirectional supersession edge between two ADRs.
    ///
    /// Validates that both ADRs exist, that `new` is `Accepted`, and that
    /// `old` is not already `Superseded` or `Deleted`. Then performs three
    /// sequential writes under per-ADR locks: clears `old`'s document
    /// (`superseded_by = new`), moves `old` into the `superseded/` directory,
    /// and appends `old` to `new.supersedes`.
    ///
    /// See the trait doc-comment on `AdrStoreBackend::supersede_adr` for the
    /// atomicity caveat.
    pub(crate) fn supersede_adr(&self, old_id: &str, new_id: &str) -> Result<(), OrbitError> {
        validate_adr_id(old_id)?;
        validate_adr_id(new_id)?;
        if old_id == new_id {
            return Err(OrbitError::InvalidInput(
                "cannot supersede an ADR with itself".to_string(),
            ));
        }

        // Hold locks for both ADRs for the duration. Acquire in ID order to
        // avoid lock-order deadlocks when concurrent supersedes touch the
        // same pair.
        let (first, second) = if old_id < new_id {
            (old_id, new_id)
        } else {
            (new_id, old_id)
        };
        let _lock_a = acquire_adr_lock(&self.root, first)?;
        let _lock_b = acquire_adr_lock(&self.root, second)?;

        let old = self
            .get_adr(old_id)?
            .ok_or_else(|| OrbitError::not_found(NotFoundKind::Adr, old_id.to_string()))?;
        let new = self
            .get_adr(new_id)?
            .ok_or_else(|| OrbitError::not_found(NotFoundKind::Adr, new_id.to_string()))?;

        if new.status != AdrStatus::Accepted {
            return Err(OrbitError::AdrInvalidTransition(format!(
                "supersede target {new_id} must be accepted (was {:?})",
                new.status
            )));
        }
        if old.status == AdrStatus::Superseded {
            return Err(OrbitError::AdrInvalidTransition(format!(
                "{old_id} is already superseded"
            )));
        }
        if old.status == AdrStatus::Deleted {
            return Err(OrbitError::AdrInvalidTransition(format!(
                "cannot supersede a deleted ADR ({old_id})"
            )));
        }

        // Write `old.superseded_by = new` first (in its current state dir,
        // before the rename). The status transition update will pick up the
        // new envelope from disk.
        let old_dir = adr_dir(&self.root, AdrStateDir::from_status(old.status), old_id);
        let mut old_bundle = read_bundle_at(&old_dir)?;
        old_bundle.doc.adr.superseded_by = Some(new_id.to_string());
        old_bundle.doc.adr.last_updated = Utc::now();
        write_bundle_at(&old_dir, &old_bundle)?;

        // Move `old` into superseded/.
        let target_state = AdrStateDir::from_status(AdrStatus::Superseded);
        let target_dir = adr_dir(&self.root, target_state, old_id);
        if let Some(parent) = target_dir.parent() {
            fs::create_dir_all(parent).map_err(|e| OrbitError::Io(e.to_string()))?;
        }
        fs::rename(&old_dir, &target_dir).map_err(|e| OrbitError::Io(e.to_string()))?;
        let mut moved = read_bundle_at(&target_dir)?;
        moved.doc.adr.status = AdrStatus::Superseded;
        moved.doc.adr.last_updated = Utc::now();
        write_bundle_at(&target_dir, &moved)?;
        self.id_allocator
            .record_adr_body_path(old_id, &super::layout::body_path(&target_dir))?;
        self.upsert_index_row(&moved.doc.adr);

        // Append `old` to `new.supersedes` (idempotent — skip if already present).
        let new_dir = adr_dir(&self.root, AdrStateDir::Accepted, new_id);
        let mut new_bundle = read_bundle_at(&new_dir)?;
        if !new_bundle.doc.adr.supersedes.iter().any(|s| s == old_id) {
            new_bundle.doc.adr.supersedes.push(old_id.to_string());
            new_bundle.doc.adr.last_updated = Utc::now();
            write_bundle_at(&new_dir, &new_bundle)?;
            self.upsert_index_row(&new_bundle.doc.adr);
        }

        Ok(())
    }

    /// Rebuilds the SQLite envelope index from the filesystem source of truth.
    ///
    /// Without an attached index this is a no-op (filesystem-only stores have
    /// nothing to rebuild). Otherwise wipes the `adrs` table inside a
    /// transaction and reinserts every ADR found on disk.
    pub(crate) fn rebuild_index(&self) -> Result<(), OrbitError> {
        let Some(index) = &self.index else {
            return Ok(());
        };
        let adrs = self.list_adrs()?;
        index.with_transaction(|tx| {
            tx.tx
                .execute("DELETE FROM adrs", [])
                .map_err(|e| OrbitError::Store(e.to_string()))?;
            for adr in &adrs {
                insert_adr_row(&tx.tx, adr)?;
            }
            Ok(())
        })
    }

    fn locate_adr(&self, id: &str) -> Result<Option<(AdrStateDir, PathBuf)>, OrbitError> {
        for state in AdrStateDir::all() {
            let dir = adr_dir(&self.root, *state, id);
            if dir.is_dir() {
                // A stray same-named dir without adr.yaml still counts as "located"
                // here so the caller gets a sensible missing-ADR / corruption error
                // from read_bundle_at.
                return Ok(Some((*state, dir)));
            }
        }
        Ok(None)
    }

    fn read_adr_allocation(&self, record: &IdAllocationRecord) -> Result<Option<Adr>, OrbitError> {
        let Some(body_path) = record.resolved_body_path() else {
            return self.get_adr(&record.id);
        };
        let Some(dir) = body_path.parent() else {
            return Ok(None);
        };
        if !super::layout::adr_doc_path(dir).is_file() {
            return Ok(None);
        }
        let bundle = read_bundle_at(dir)?;
        Ok(Some(bundle_to_adr(bundle)))
    }

    fn upsert_index_row(&self, adr: &Adr) {
        let Some(index) = &self.index else {
            return;
        };
        let result = index.with_transaction(|tx| {
            tx.tx
                .execute("DELETE FROM adrs WHERE id = ?1", params![adr.id])
                .map_err(|e| OrbitError::Store(e.to_string()))?;
            insert_adr_row(&tx.tx, adr)?;
            Ok(())
        });
        if let Err(err) = result {
            orbit_common::tracing::warn!(
                target: "orbit.store.adr",
                adr_id = adr.id.as_str(),
                error = %err,
                "failed to upsert ADR envelope into index; filesystem is source of truth",
            );
        }
    }

    fn delete_index_row(&self, id: &str) {
        let Some(index) = &self.index else {
            return;
        };
        let result = index.with_transaction(|tx| {
            tx.tx
                .execute("DELETE FROM adrs WHERE id = ?1", params![id])
                .map_err(|e| OrbitError::Store(e.to_string()))?;
            Ok(())
        });
        if let Err(err) = result {
            orbit_common::tracing::warn!(
                target: "orbit.store.adr",
                adr_id = id,
                error = %err,
                "failed to delete ADR envelope from index; filesystem is source of truth",
            );
        }
    }

    fn query_filtered_ids(
        &self,
        status: Option<AdrStatus>,
        owner: Option<&str>,
        feature: Option<&str>,
        task_id: Option<&str>,
        legacy_id: Option<&str>,
        validation_warned: Option<bool>,
    ) -> Result<Vec<String>, OrbitError> {
        let index = self
            .index
            .as_ref()
            .expect("query_filtered_ids only invoked when index is attached");

        let mut sql = String::from("SELECT id FROM adrs WHERE 1=1");
        let mut bound: Vec<String> = Vec::new();

        if let Some(status) = status {
            sql.push_str(" AND status = ?");
            bound.push(status.cli_name().to_string());
        }
        if let Some(owner) = owner {
            sql.push_str(" AND owner = ?");
            bound.push(owner.to_string());
        }
        if let Some(feature) = feature {
            // JSON-encoded substring match so partial values (e.g. "feature-a"
            // vs "feature-ab") don't collide.
            sql.push_str(" AND related_features LIKE ?");
            bound.push(format!("%\"{feature}\"%"));
        }
        if let Some(task_id) = task_id {
            sql.push_str(" AND related_tasks LIKE ?");
            bound.push(format!("%\"{task_id}\"%"));
        }
        if let Some(legacy_id) = legacy_id {
            sql.push_str(" AND legacy_ids LIKE ?");
            bound.push(format!("%\"{legacy_id}\"%"));
        }
        if let Some(warned) = validation_warned {
            sql.push_str(" AND legacy_validation = ?");
            bound.push(
                if warned {
                    LegacyValidation::Warned
                } else {
                    LegacyValidation::None
                }
                .to_string(),
            );
        }
        sql.push_str(" ORDER BY id DESC");

        let conn_arc = index.connection();
        let conn = conn_arc
            .lock()
            .map_err(|e| OrbitError::Store(format!("mutex poisoned: {e}")))?;
        let mut stmt = conn
            .prepare(&sql)
            .map_err(|e| OrbitError::Store(e.to_string()))?;

        let param_refs: Vec<&dyn rusqlite::ToSql> =
            bound.iter().map(|s| s as &dyn rusqlite::ToSql).collect();
        let rows = stmt
            .query_map(param_refs.as_slice(), |row| row.get::<_, String>(0))
            .map_err(|e| OrbitError::Store(e.to_string()))?;

        let mut ids = Vec::new();
        for row in rows {
            ids.push(row.map_err(|e| OrbitError::Store(e.to_string()))?);
        }
        Ok(ids)
    }
}

fn matches_filter(
    adr: &Adr,
    status: Option<AdrStatus>,
    owner: Option<&str>,
    feature: Option<&str>,
    task_id: Option<&str>,
    legacy_id: Option<&str>,
    tag: Option<&str>,
    normalized_path: Option<&str>,
    validation_warned: Option<bool>,
) -> bool {
    if let Some(status) = status
        && adr.status != status
    {
        return false;
    }
    if let Some(owner) = owner
        && adr.owner != owner
    {
        return false;
    }
    if let Some(feature) = feature
        && !adr.related_features.iter().any(|f| f == feature)
    {
        return false;
    }
    if let Some(task_id) = task_id
        && !adr.related_tasks.iter().any(|t| t == task_id)
    {
        return false;
    }
    if let Some(legacy_id) = legacy_id
        && !adr.legacy_ids.iter().any(|l| l == legacy_id)
    {
        return false;
    }
    if let Some(tag) = tag
        && !tag.trim().is_empty()
        && !adr
            .tags
            .iter()
            .any(|candidate| candidate.eq_ignore_ascii_case(tag))
    {
        return false;
    }
    if let Some(path) = normalized_path
        && !adr_paths_contain_path(&adr.paths, path)
    {
        return false;
    }
    if let Some(warned) = validation_warned {
        let is_warned = matches!(adr.legacy_validation, LegacyValidation::Warned);
        if warned != is_warned {
            return false;
        }
    }
    true
}

fn normalized_filter_path(path: Option<&str>) -> Result<Option<String>, OrbitError> {
    path.map(normalize_glob_path).transpose()
}

fn adr_paths_contain_path(rules: &[String], normalized_path: &str) -> bool {
    rules.iter().any(|rule| {
        compile_glob_regex(rule)
            .map(|regex| regex.is_match(normalized_path))
            .unwrap_or(false)
    })
}

fn sort_by_id_desc(adrs: &mut [Adr]) {
    adrs.sort_by(|a, b| b.id.cmp(&a.id));
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

fn adr_entry_id(entry: &AdrListEntry) -> &str {
    match entry {
        AdrListEntry::Local(adr) => &adr.id,
        AdrListEntry::Remote(stub) => &stub.id,
    }
}

#[allow(clippy::too_many_arguments)]
fn remote_adr_matches_filter(
    record: &IdAllocationRecord,
    status: Option<AdrStatus>,
    owner: Option<&str>,
    feature: Option<&str>,
    task_id: Option<&str>,
    legacy_id: Option<&str>,
    tag: Option<&str>,
    normalized_path: Option<&str>,
    validation_warned: Option<bool>,
) -> bool {
    if owner.is_some()
        || feature.is_some()
        || task_id.is_some()
        || legacy_id.is_some()
        || tag.is_some()
        || normalized_path.is_some()
        || validation_warned.is_some()
    {
        return false;
    }
    if let Some(status) = status
        && remote_adr_status(record.body_path.as_deref()) != Some(status)
    {
        return false;
    }
    true
}

fn remote_adr_status(body_path: Option<&Path>) -> Option<AdrStatus> {
    let path = body_path?;
    path.components()
        .filter_map(|component| component.as_os_str().to_str())
        .find_map(|component| match component {
            "proposed" => Some(AdrStatus::Proposed),
            "accepted" => Some(AdrStatus::Accepted),
            "superseded" => Some(AdrStatus::Superseded),
            "deleted" => Some(AdrStatus::Deleted),
            _ => None,
        })
}

fn insert_adr_row(conn: &rusqlite::Connection, adr: &Adr) -> Result<(), OrbitError> {
    let related_features = serde_json::to_string(&adr.related_features)
        .map_err(|e| OrbitError::Store(e.to_string()))?;
    let related_tasks =
        serde_json::to_string(&adr.related_tasks).map_err(|e| OrbitError::Store(e.to_string()))?;
    let tags = serde_json::to_string(&adr.tags).map_err(|e| OrbitError::Store(e.to_string()))?;
    let paths = serde_json::to_string(&adr.paths).map_err(|e| OrbitError::Store(e.to_string()))?;
    let legacy_ids =
        serde_json::to_string(&adr.legacy_ids).map_err(|e| OrbitError::Store(e.to_string()))?;
    let supersedes =
        serde_json::to_string(&adr.supersedes).map_err(|e| OrbitError::Store(e.to_string()))?;
    let validation_warnings = serde_json::to_string(&adr.validation_warnings)
        .map_err(|e| OrbitError::Store(e.to_string()))?;

    conn.execute(
        "INSERT INTO adrs (
            id, status, title, owner,
            related_features, related_tasks, tags, paths, legacy_ids, supersedes,
            superseded_by, validation_warnings, legacy_validation,
            created_at, accepted_at, last_updated
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16)",
        params![
            adr.id,
            adr.status.cli_name(),
            adr.title,
            adr.owner,
            related_features,
            related_tasks,
            tags,
            paths,
            legacy_ids,
            supersedes,
            adr.superseded_by,
            validation_warnings,
            adr.legacy_validation.to_string(),
            adr.created_at.to_rfc3339(),
            adr.accepted_at.map(|ts| ts.to_rfc3339()),
            adr.last_updated.to_rfc3339(),
        ],
    )
    .map_err(|e| OrbitError::Store(e.to_string()))?;
    Ok(())
}

#[cfg(test)]
#[cfg(test)]
mod tests;
