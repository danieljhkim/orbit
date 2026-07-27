// Existing expect calls in this module document local invariants; keep the allow scoped while the workspace lint is ratcheted.
#![allow(clippy::expect_used)]

use std::sync::Arc;

use chrono::{DateTime, Utc};
use orbit_common::types::{Learning, LearningStatus, OrbitError};
use orbit_common::utility::glob::{compile_glob_regex, normalize_glob_path};

use super::store::LearningFileStore;
use crate::backend::{LearningSearchParams, LearningSearchResult};

/// [ORB-00413] An on-disk learning body whose ID the allocator has no active
/// record of — the fingerprint of a legacy partial create (body written, the
/// allocation never recorded). Returned by
/// [`LearningFileStore::reconcile_learning_orphans`] and surfaced as warnings by
/// [`LearningFileStore::sync_learnings`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LearningOrphan {
    pub(crate) id: String,
    pub(crate) body_path: std::path::PathBuf,
    pub(crate) remedy: String,
}

pub(crate) struct EnvelopeSnapshot {
    pub(super) id: String,
    pub(super) paths: Vec<String>,
    /// Pre-compiled regexes for `paths`, lazily co-built when the envelope
    /// snapshot is materialized. Search hot-path matches against these so
    /// per-call regex compilation does not dominate the budget.
    pub(super) path_regexes: Vec<regex::Regex>,
    pub(super) tags: Vec<String>,
    pub(super) summary: String,
    pub(super) updated_at_key: String,
    pub(super) priority: Option<u8>,
}

impl LearningFileStore {
    /// Reconcile the SQLite envelope index from the YAML source of truth.
    ///
    /// No-op when no index is attached; otherwise wipes
    /// `learnings_index` and reinserts every record found on disk.
    pub(crate) fn sync_learnings(&self) -> Result<(), OrbitError> {
        // [ORB-00413] Surface orphaned body files (the fingerprint of a legacy
        // partial create: body on disk, allocation never recorded) so the
        // resync command reports them with a remedy. Non-fatal to the sync.
        match self.reconcile_learning_orphans() {
            Ok(orphans) => {
                for orphan in &orphans {
                    orbit_common::tracing::warn!(
                        target: "orbit.store.learning",
                        id = %orphan.id,
                        body_path = %orphan.body_path.display(),
                        "orphaned learning body detected: {}",
                        orphan.remedy,
                    );
                }
            }
            Err(error) => orbit_common::tracing::warn!(
                target: "orbit.store.learning",
                error = %error,
                "learning orphan reconcile failed during sync",
            ),
        }
        let Some(index) = &self.index else {
            self.invalidate_envelope_cache();
            return Ok(());
        };
        let learnings = self.list_learnings(None)?;
        index.truncate_learning_index(&self.workspace_id)?;
        for learning in &learnings {
            index.upsert_learning_index_row(&self.workspace_id, learning)?;
        }
        self.invalidate_envelope_cache();
        Ok(())
    }

    /// [ORB-00413] Detect learning body files on disk whose ID the allocator has
    /// no active record of. Such an orphan is the residue of a legacy partial
    /// create (body written before the allocation was recorded) that predates
    /// the write-time rollback in `crud`. Read-only: reports, never mutates.
    pub(crate) fn reconcile_learning_orphans(&self) -> Result<Vec<LearningOrphan>, OrbitError> {
        let mut orphans = Vec::new();
        if !self.root.exists() {
            return Ok(orphans);
        }
        for entry in std::fs::read_dir(&self.root).map_err(|e| OrbitError::Io(e.to_string()))? {
            let entry = entry.map_err(|e| OrbitError::Io(e.to_string()))?;
            if !entry
                .file_type()
                .map_err(|e| OrbitError::Io(e.to_string()))?
                .is_dir()
            {
                continue;
            }
            let Some(id) = entry.file_name().to_str().map(str::to_string) else {
                continue;
            };
            if super::super::layout::validate_learning_id(&id).is_err() {
                continue;
            }
            let body_path = super::super::layout::learning_doc_path(&self.root, &id);
            if !body_path.is_file() {
                continue;
            }
            if self.id_allocator.learning_allocation(&id)?.is_none() {
                orphans.push(LearningOrphan {
                    id: id.clone(),
                    body_path,
                    remedy: format!(
                        "learning body exists on disk but the allocator has no record of '{id}'; \
                         reopen the workspace to backfill the allocation, or remove the orphaned body"
                    ),
                });
            }
        }
        orphans.sort_by(|left, right| left.id.cmp(&right.id));
        Ok(orphans)
    }

    /// Run the phase-1 scope-OR search.
    ///
    /// When an index is attached the active row list is pulled from SQLite;
    /// otherwise we fall back to a filesystem walk. Path globs match against
    /// `normalize_glob_path(params.path)` via [`match_glob`]; tags match as
    /// exact lowercase strings; `query` substring-matches `summary`. Search
    /// is active-only by design — superseded records are excluded from
    /// injection per ADR-003.
    ///
    /// **Hot path.** Per ADR-002 / §5.2 of the design doc, this call must
    /// stay sub-10 ms at expected scale. The returned `Learning` payloads
    /// are reconstituted from index columns only (no YAML I/O), which is
    /// safe because §4.5 specifies that injection only consumes `summary`
    /// + scope axes; full bodies and evidence are loaded on demand via
    ///   `get_learning`. Callers that need a full record should follow up
    ///   with [`Self::get_learning`] using the returned `learning.id`.
    pub(crate) fn search_learnings(
        &self,
        params: LearningSearchParams,
    ) -> Result<Vec<LearningSearchResult>, OrbitError> {
        let limit = params.limit.unwrap_or(usize::MAX);
        let normalized_path = params
            .path
            .as_deref()
            .map(normalize_glob_path)
            .transpose()?;
        let tag_lower = params.tag.as_deref().map(|t| t.trim().to_lowercase());
        let query_lower = params.query.as_deref().map(|q| q.to_lowercase());

        let candidates = self.active_envelopes()?;

        let unfiltered = normalized_path.is_none() && tag_lower.is_none() && query_lower.is_none();

        let mut matched: Vec<(&EnvelopeSnapshot, Vec<String>)> = Vec::new();
        for envelope in candidates.iter() {
            let mut axes = Vec::new();
            if let Some(path) = &normalized_path {
                for (rule, regex) in envelope.paths.iter().zip(envelope.path_regexes.iter()) {
                    if regex.is_match(path) {
                        axes.push(format!("path:{rule}"));
                        break;
                    }
                }
            }
            if let Some(tag) = &tag_lower
                && envelope.tags.iter().any(|t| t == tag)
            {
                axes.push(format!("tag:{tag}"));
            }
            if let Some(q) = &query_lower
                && envelope.summary.to_lowercase().contains(q)
            {
                axes.push("query:summary".to_string());
            }

            if axes.is_empty() && !unfiltered {
                continue;
            }
            matched.push((envelope, axes));
        }

        // Sort by priority then recency. RFC3339 string compare is correct
        // because `Learning::updated_at` is `DateTime<Utc>`.
        matched.sort_by(|a, b| {
            priority_rank(b.0.priority)
                .cmp(&priority_rank(a.0.priority))
                .then_with(|| b.0.updated_at_key.cmp(&a.0.updated_at_key))
                .then_with(|| a.0.id.cmp(&b.0.id))
        });

        let mut results = Vec::with_capacity(limit.min(matched.len()));
        for (envelope, axes) in matched.into_iter().take(limit) {
            let updated_at = parse_rfc3339_or_epoch(&envelope.updated_at_key);
            let learning = Learning {
                id: envelope.id.clone(),
                status: LearningStatus::Active,
                scope: orbit_common::types::LearningScope {
                    paths: envelope.paths.clone(),
                    tags: envelope.tags.clone(),
                    ..Default::default()
                },
                summary: envelope.summary.clone(),
                body: String::new(),
                evidence: Vec::new(),
                supersedes: None,
                superseded_by: None,
                legacy_ids: Vec::new(),
                created_at: updated_at,
                updated_at,
                created_by: None,
                priority: envelope.priority,
            };
            results.push(LearningSearchResult {
                learning,
                matched_by: axes,
            });
        }
        Ok(results)
    }

    /// Read-through accessor for the active envelope set. Cached after the
    /// first call; invalidated on every mutating operation. Returns an
    /// `Arc`-shaped clone so the read lock isn't held across the match
    /// loop.
    fn active_envelopes(&self) -> Result<Arc<Vec<EnvelopeSnapshot>>, OrbitError> {
        // Fast path: cached.
        {
            let guard = self
                .envelope_cache
                .read()
                .map_err(|e| OrbitError::Store(format!("envelope cache poisoned: {e}")))?;
            if let Some(cached) = guard.as_ref() {
                return Ok(Arc::clone(cached));
            }
        }

        // Build under the index/yaml path, then publish.
        let built: Vec<EnvelopeSnapshot> = if let Some(index) = &self.index {
            let rows = index.list_active_learning_rows(&self.workspace_id)?;
            rows.into_iter()
                .map(|row| {
                    build_envelope(
                        row.id,
                        row.paths,
                        row.tags,
                        row.summary,
                        row.updated_at,
                        row.priority,
                    )
                })
                .collect()
        } else {
            let active = self.list_learnings(Some(LearningStatus::Active))?;
            active
                .into_iter()
                .map(|l| {
                    build_envelope(
                        l.id,
                        l.scope.paths,
                        l.scope.tags,
                        l.summary,
                        l.updated_at.to_rfc3339(),
                        l.priority,
                    )
                })
                .collect()
        };
        let arc = Arc::new(built);
        let mut guard = self
            .envelope_cache
            .write()
            .map_err(|e| OrbitError::Store(format!("envelope cache poisoned: {e}")))?;
        *guard = Some(Arc::clone(&arc));
        Ok(arc)
    }

    pub(super) fn invalidate_envelope_cache(&self) {
        if let Ok(mut guard) = self.envelope_cache.write() {
            *guard = None;
        }
    }

    pub(super) fn upsert_index_row(&self, learning: &Learning) {
        let Some(index) = &self.index else {
            return;
        };
        if let Err(err) = index.upsert_learning_index_row(&self.workspace_id, learning) {
            orbit_common::tracing::warn!(
                target: "orbit.store.learning",
                learning_id = learning.id.as_str(),
                error = %err,
                "failed to upsert learning envelope into index; filesystem is source of truth",
            );
        }
    }
}

fn build_envelope(
    id: String,
    paths: Vec<String>,
    tags: Vec<String>,
    summary: String,
    updated_at_key: String,
    priority: Option<u8>,
) -> EnvelopeSnapshot {
    let path_regexes = paths
        .iter()
        .filter_map(|rule| compile_glob_regex(rule).ok())
        .collect();
    EnvelopeSnapshot {
        id,
        paths,
        path_regexes,
        tags,
        summary,
        updated_at_key,
        priority,
    }
}

fn parse_rfc3339_or_epoch(raw: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(raw)
        .map(|dt| dt.with_timezone(&Utc))
        .unwrap_or_else(|_| DateTime::<Utc>::from_timestamp(0, 0).expect("epoch is valid"))
}

/// Map an optional priority to a comparable rank where `Some(N)` always
/// outranks `None` and higher `N` wins among `Some`. Used as the primary
/// sort key in `search_learnings`.
fn priority_rank(priority: Option<u8>) -> i16 {
    match priority {
        // None ranks below every Some; pick a value strictly below 0.
        None => -1,
        Some(value) => value as i16,
    }
}
