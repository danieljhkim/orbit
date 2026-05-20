// ORB-00013: Existing expect calls in this module document local invariants; keep the allow scoped while the workspace lint is ratcheted.
#![allow(clippy::expect_used)]

use chrono::{DateTime, Utc};
use orbit_common::types::{
    Learning, LearningStatus, NotFoundKind, OrbitError, normalize_learning_paths,
    normalize_learning_tags,
};

use super::super::layout::{
    comments_jsonl_path, learning_doc_path, locate_learning, validate_learning_id, votes_jsonl_path,
};
use super::super::record::{read_learning_file, write_learning_file};
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

        let id = self.id_allocator.allocate_learning()?.id;

        let mut scope = params.scope;
        scope.paths = normalize_learning_paths(scope.paths);
        scope.tags = normalize_learning_tags(scope.tags);

        let learning = Learning {
            id: id.clone(),
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

        let path = learning_doc_path(&self.root, &id);
        write_learning_file(&path, &learning, LearningStatus::Active)?;
        ensure_empty_sidecar(&votes_jsonl_path(&self.root, &id))?;
        ensure_empty_sidecar(&comments_jsonl_path(&self.root, &id))?;
        self.id_allocator.record_learning_body_path(&id, &path)?;
        self.upsert_index_row(&learning);
        self.invalidate_envelope_cache();
        Ok(learning)
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
        let Some(path) = record.resolved_body_path() else {
            return self.get_learning(&record.id);
        };
        if !path.is_file() {
            return Ok(None);
        }
        Ok(Some(read_learning_file(&path)?))
    }
}

fn ensure_empty_sidecar(path: &std::path::Path) -> Result<(), OrbitError> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|error| OrbitError::Io(error.to_string()))?;
    }
    std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map(|_| ())
        .map_err(|error| OrbitError::Io(error.to_string()))
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
