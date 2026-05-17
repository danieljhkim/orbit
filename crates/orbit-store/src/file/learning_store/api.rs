// ORB-00013: Existing expect calls in this module document local invariants; keep the allow scoped while the workspace lint is ratcheted.
#![allow(clippy::expect_used)]

use std::env;
use std::fs;
use std::path::PathBuf;
use std::sync::{Arc, RwLock};

use chrono::{DateTime, Utc};
use orbit_common::types::{
    Learning, LearningComment, LearningCommentEvent, LearningCommentTombstone, LearningStatus,
    LearningVoteRow, LearningVoteSummary, NotFoundKind, OrbitError, decayed_vote_score,
    normalize_learning_paths, normalize_learning_tags,
};
use orbit_common::utility::glob::{compile_glob_regex, normalize_glob_path};

use super::layout::{
    comments_jsonl_path, learning_doc_path, locate_learning, next_learning_comment_id,
    next_learning_id, validate_learning_comment_id, validate_learning_id, votes_jsonl_path,
};
use super::lock::{acquire_learning_allocation_lock, acquire_learning_lock};
use super::record::{
    append_jsonl_comment_row, lookup_learning_comment, read_comment_events, read_learning_file,
    scan_learning_comments, write_learning_file,
};
use super::votes::{
    append_vote_row, deduped_vote_times, read_vote_rows, summarize_votes, validate_vote_files,
};
use crate::Store;
use crate::backend::{
    LearningCommentAddParams, LearningCommentDeleteParams, LearningCreateParams,
    LearningSearchParams, LearningSearchResult, LearningUpdateParams, LearningUpvoteParams,
};

/// Workspace-scoped, filesystem-backed learning store.
///
/// YAML files at `<root>/<id>/learning.yaml` are the source of truth. Status
/// lives in the YAML body. When `index` is attached, envelope
/// metadata mirrors into the shared SQLite `learnings_index` table for fast
/// scope-glob lookups; the filesystem walk is the fallback path when the
/// index is absent (e.g. tests using `LearningFileStore::new`).
///
/// Search is on the hot path (called from injection layers; budget < 10 ms
/// per the design's §5.2). The store keeps an in-memory `envelope_cache`
/// over the active envelope set so 1000 sequential `search` calls don't
/// each pay SQLite lock + JSON-array decode overhead. Cache is invalidated
/// on every mutating call.
pub(crate) struct LearningFileStore {
    root: PathBuf,
    index: Option<Store>,
    envelope_cache: RwLock<Option<Arc<Vec<EnvelopeSnapshot>>>>,
}

impl LearningFileStore {
    #[cfg(test)]
    pub(crate) fn new(root: PathBuf) -> Self {
        Self {
            root,
            index: None,
            envelope_cache: RwLock::new(None),
        }
    }

    pub(crate) fn new_with_index(root: PathBuf, index: Store) -> Self {
        Self {
            root,
            index: Some(index),
            envelope_cache: RwLock::new(None),
        }
    }

    pub(crate) fn reject_legacy_flat_layout(root: &std::path::Path) -> Result<(), OrbitError> {
        super::migration::reject_legacy_flat_layout(root)
    }

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

        let _allocation_lock = acquire_learning_allocation_lock(&self.root)?;
        let id = next_learning_id(&self.root, now)?;

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
            created_at: now,
            updated_at: now,
            created_by: params.created_by,
            priority: params.priority,
        };

        let path = learning_doc_path(&self.root, &id);
        write_learning_file(&path, &learning, LearningStatus::Active)?;
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

    pub(crate) fn list_learnings(
        &self,
        status: Option<LearningStatus>,
    ) -> Result<Vec<Learning>, OrbitError> {
        let mut out = Vec::new();
        if !self.root.exists() {
            return Ok(out);
        }
        for entry in fs::read_dir(&self.root).map_err(|e| OrbitError::Io(e.to_string()))? {
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

    pub(crate) fn update_learning(
        &self,
        id: &str,
        params: LearningUpdateParams,
    ) -> Result<Learning, OrbitError> {
        validate_learning_id(id)?;
        let _lock = acquire_learning_lock(&self.root, id)?;

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
        fs::remove_file(&path).map_err(|e| OrbitError::Io(e.to_string()))?;
        if let Some(parent) = path.parent()
            && parent
                .read_dir()
                .map(|mut entries| entries.next().is_none())
                .unwrap_or(false)
        {
            fs::remove_dir(parent).map_err(|e| OrbitError::Io(e.to_string()))?;
        }
        if let Some(index) = &self.index {
            index.delete_learning_index_row(id)?;
        }
        self.invalidate_envelope_cache();
        Ok(true)
    }

    pub(crate) fn upvote_learning(
        &self,
        params: LearningUpvoteParams,
    ) -> Result<LearningVoteSummary, OrbitError> {
        self.upvote_learning_at(params, Utc::now())
    }

    #[cfg(test)]
    pub(crate) fn upvote_learning_at(
        &self,
        params: LearningUpvoteParams,
        now: DateTime<Utc>,
    ) -> Result<LearningVoteSummary, OrbitError> {
        self.upvote_learning_at_impl(params, now)
    }

    #[cfg(not(test))]
    fn upvote_learning_at(
        &self,
        params: LearningUpvoteParams,
        now: DateTime<Utc>,
    ) -> Result<LearningVoteSummary, OrbitError> {
        self.upvote_learning_at_impl(params, now)
    }

    fn upvote_learning_at_impl(
        &self,
        params: LearningUpvoteParams,
        now: DateTime<Utc>,
    ) -> Result<LearningVoteSummary, OrbitError> {
        validate_learning_id(&params.learning_id)?;
        let Some(path) = locate_learning(&self.root, &params.learning_id)? else {
            return Err(OrbitError::not_found(
                NotFoundKind::Learning,
                params.learning_id,
            ));
        };
        let learning = read_learning_file(&path)?;
        if learning.status == LearningStatus::Superseded {
            return Err(OrbitError::InvalidInput(format!(
                "learning '{}' is superseded; use the superseding learning before voting",
                learning.id
            )));
        }

        let task_id = params
            .task_id
            .map(|task_id| task_id.trim().to_string())
            .filter(|task_id| !task_id.is_empty())
            .ok_or_else(|| {
                OrbitError::InvalidInput(
                    "learning upvote requires `task_id` in v1; free-floating votes are rejected by policy"
                        .to_string(),
                )
            })?;
        let voter_model = params.voter_model.trim().to_string();
        if voter_model.is_empty() {
            return Err(OrbitError::InvalidInput(
                "learning upvote requires a non-empty voter model".to_string(),
            ));
        }

        let _lock = acquire_learning_lock(&self.root, &learning.id)?;
        let votes_path = votes_jsonl_path(&self.root, &learning.id);
        let rows = read_vote_rows(&votes_path)?;
        let already_voted = rows.iter().any(|row| {
            row.learning_id == learning.id
                && row.voter_model == voter_model
                && row.task_id.as_deref() == Some(task_id.as_str())
        });
        if already_voted {
            return Ok(summarize_votes(&rows));
        }

        let mut next_rows = rows;
        let row = LearningVoteRow {
            learning_id: learning.id,
            voter_model,
            voted_at: now,
            task_id: Some(task_id),
        };
        append_vote_row(&votes_path, &row)?;
        next_rows.push(row);
        Ok(summarize_votes(&next_rows))
    }

    pub(crate) fn learning_vote_summary(
        &self,
        id: &str,
    ) -> Result<LearningVoteSummary, OrbitError> {
        validate_learning_id(id)?;
        if locate_learning(&self.root, id)?.is_none() {
            return Err(OrbitError::not_found(
                NotFoundKind::Learning,
                id.to_string(),
            ));
        }
        let rows = read_vote_rows(&votes_jsonl_path(&self.root, id))?;
        Ok(summarize_votes(&rows))
    }

    pub(crate) fn add_learning_comment(
        &self,
        params: LearningCommentAddParams,
    ) -> Result<LearningComment, OrbitError> {
        self.add_learning_comment_at(params, Utc::now())
    }

    #[cfg(test)]
    pub(crate) fn add_learning_comment_at(
        &self,
        params: LearningCommentAddParams,
        now: DateTime<Utc>,
    ) -> Result<LearningComment, OrbitError> {
        self.add_learning_comment_at_impl(params, now)
    }

    #[cfg(not(test))]
    fn add_learning_comment_at(
        &self,
        params: LearningCommentAddParams,
        now: DateTime<Utc>,
    ) -> Result<LearningComment, OrbitError> {
        self.add_learning_comment_at_impl(params, now)
    }

    fn add_learning_comment_at_impl(
        &self,
        params: LearningCommentAddParams,
        now: DateTime<Utc>,
    ) -> Result<LearningComment, OrbitError> {
        validate_learning_id(&params.learning_id)?;
        let body = validate_learning_comment_body(&params.body)?;
        let author_model = validate_learning_comment_model(&params.author_model)?;

        let Some(path) = locate_learning(&self.root, &params.learning_id)? else {
            return Err(OrbitError::not_found(
                NotFoundKind::Learning,
                params.learning_id,
            ));
        };
        let learning = read_learning_file(&path)?;
        if learning.status == LearningStatus::Superseded {
            return Err(OrbitError::InvalidInput(format!(
                "learning '{}' is superseded; use orbit.learning.supersede for the parent-replacement workflow",
                learning.id
            )));
        }

        let _allocation_lock = acquire_learning_allocation_lock(&self.root)?;
        let _lock = acquire_learning_lock(&self.root, &learning.id)?;

        let Some(path) = locate_learning(&self.root, &learning.id)? else {
            return Err(OrbitError::not_found(NotFoundKind::Learning, learning.id));
        };
        let learning = read_learning_file(&path)?;
        if learning.status == LearningStatus::Superseded {
            return Err(OrbitError::InvalidInput(format!(
                "learning '{}' is superseded; use orbit.learning.supersede for the parent-replacement workflow",
                learning.id
            )));
        }

        let comment = LearningComment {
            id: next_learning_comment_id(&self.root, now)?,
            learning_id: learning.id.clone(),
            body,
            author_model,
            created_at: now,
        };
        append_jsonl_comment_row(
            &comments_jsonl_path(&self.root, &learning.id),
            &LearningCommentEvent::Create(comment.clone()),
        )?;
        Ok(comment)
    }

    pub(crate) fn list_learning_comments(
        &self,
        learning_id: &str,
        include_deleted: bool,
    ) -> Result<Vec<LearningComment>, OrbitError> {
        validate_learning_id(learning_id)?;
        if locate_learning(&self.root, learning_id)?.is_none() {
            return Err(OrbitError::not_found(
                NotFoundKind::Learning,
                learning_id.to_string(),
            ));
        }
        scan_learning_comments(
            &comments_jsonl_path(&self.root, learning_id),
            include_deleted,
        )
    }

    pub(crate) fn delete_learning_comment(
        &self,
        params: LearningCommentDeleteParams,
    ) -> Result<(), OrbitError> {
        validate_learning_comment_id(&params.comment_id)?;
        let deleted_by = validate_learning_comment_model(&params.deleted_by)?;
        let Some(parent_id) = self.find_learning_for_comment(&params.comment_id)? else {
            return Err(OrbitError::not_found(
                NotFoundKind::LearningComment,
                params.comment_id,
            ));
        };
        let _lock = acquire_learning_lock(&self.root, &parent_id.learning_id)?;
        let path = comments_jsonl_path(&self.root, &parent_id.learning_id);
        let Some(lookup) = lookup_learning_comment(&path, &params.comment_id)? else {
            return Err(OrbitError::not_found(
                NotFoundKind::LearningComment,
                params.comment_id,
            ));
        };
        if lookup.deleted {
            return Ok(());
        }
        append_jsonl_comment_row(
            &path,
            &LearningCommentEvent::Tombstone(LearningCommentTombstone {
                id: params.comment_id,
                learning_id: lookup.learning_id,
                op: "delete".to_string(),
                deleted_at: Utc::now(),
                deleted_by,
            }),
        )
    }

    /// Rebuild the SQLite index from the YAML source of truth.
    ///
    /// No-op when no index is attached; otherwise wipes
    /// `learnings_index` and reinserts every record found on disk.
    pub(crate) fn reindex_learnings(&self) -> Result<(), OrbitError> {
        validate_vote_files(&self.root)?;
        validate_comment_files(&self.root)?;
        let Some(index) = &self.index else {
            self.invalidate_envelope_cache();
            return Ok(());
        };
        let learnings = self.list_learnings(None)?;
        index.truncate_learning_index()?;
        for learning in &learnings {
            index.upsert_learning_index_row(learning)?;
        }
        self.invalidate_envelope_cache();
        Ok(())
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
        self.search_learnings_at(params, Utc::now())
    }

    pub(crate) fn search_learnings_at(
        &self,
        params: LearningSearchParams,
        now: DateTime<Utc>,
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

        let half_life_days = vote_half_life_days();
        let mut matched: Vec<(&EnvelopeSnapshot, Vec<String>, f64)> = Vec::new();
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
            let vote_times = deduped_vote_times(&read_vote_rows(&votes_jsonl_path(
                &self.root,
                &envelope.id,
            ))?);
            let vote_score = decayed_vote_score(&vote_times, now, half_life_days);
            matched.push((envelope, axes, vote_score));
        }

        // Sort by decayed vote score first, then the prior priority and
        // recency keys. RFC3339 string compare is correct because
        // `Learning::updated_at` is `DateTime<Utc>`.
        matched.sort_by(|a, b| {
            b.2.total_cmp(&a.2)
                .then_with(|| priority_rank(b.0.priority).cmp(&priority_rank(a.0.priority)))
                .then_with(|| b.0.updated_at_key.cmp(&a.0.updated_at_key))
                .then_with(|| a.0.id.cmp(&b.0.id))
        });

        let mut results = Vec::with_capacity(limit.min(matched.len()));
        for (envelope, axes, _score) in matched.into_iter().take(limit) {
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
            let rows = index.list_active_learning_rows()?;
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

    fn invalidate_envelope_cache(&self) {
        if let Ok(mut guard) = self.envelope_cache.write() {
            *guard = None;
        }
    }

    fn upsert_index_row(&self, learning: &Learning) {
        let Some(index) = &self.index else {
            return;
        };
        if let Err(err) = index.upsert_learning_index_row(learning) {
            orbit_common::tracing::warn!(
                target: "orbit.store.learning",
                learning_id = learning.id.as_str(),
                error = %err,
                "failed to upsert learning envelope into index; filesystem is source of truth",
            );
        }
    }

    fn find_learning_for_comment(
        &self,
        comment_id: &str,
    ) -> Result<Option<super::record::LearningCommentLookup>, OrbitError> {
        if !self.root.exists() {
            return Ok(None);
        }
        for entry in fs::read_dir(&self.root).map_err(|e| OrbitError::Io(e.to_string()))? {
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
            let path = comments_jsonl_path(&self.root, &id);
            if let Some(lookup) = lookup_learning_comment(&path, comment_id)? {
                return Ok(Some(lookup));
            }
        }
        Ok(None)
    }
}

struct EnvelopeSnapshot {
    id: String,
    paths: Vec<String>,
    /// Pre-compiled regexes for `paths`, lazily co-built when the envelope
    /// snapshot is materialized. Search hot-path matches against these so
    /// per-call regex compilation does not dominate the budget.
    path_regexes: Vec<regex::Regex>,
    tags: Vec<String>,
    summary: String,
    updated_at_key: String,
    priority: Option<u8>,
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

fn vote_half_life_days() -> f64 {
    const DEFAULT_HALF_LIFE_DAYS: f64 = 180.0;
    env::var("ORBIT_LEARNING_VOTE_HALF_LIFE_DAYS")
        .ok()
        .and_then(|raw| raw.trim().parse::<f64>().ok())
        .filter(|value| value.is_finite() && *value >= 0.0)
        .unwrap_or(DEFAULT_HALF_LIFE_DAYS)
}

fn validate_learning_comment_body(raw: &str) -> Result<String, OrbitError> {
    let body = raw.trim().to_string();
    if body.is_empty() {
        return Err(OrbitError::InvalidInput(
            "learning comment body must not be empty".to_string(),
        ));
    }
    let count = body.chars().count();
    if count > 500 {
        return Err(OrbitError::InvalidInput(format!(
            "learning comment body must be at most 500 characters (got {count})"
        )));
    }
    Ok(body)
}

fn validate_learning_comment_model(raw: &str) -> Result<String, OrbitError> {
    let model = raw.trim().to_string();
    if model.is_empty() {
        return Err(OrbitError::InvalidInput(
            "learning comment requires a non-empty model".to_string(),
        ));
    }
    Ok(model)
}

fn validate_comment_files(root: &std::path::Path) -> Result<(), OrbitError> {
    if !root.exists() {
        return Ok(());
    }
    for entry in fs::read_dir(root).map_err(|err| OrbitError::Io(err.to_string()))? {
        let entry = entry.map_err(|err| OrbitError::Io(err.to_string()))?;
        let file_type = entry
            .file_type()
            .map_err(|err| OrbitError::Io(err.to_string()))?;
        if !file_type.is_dir() {
            continue;
        }
        let Some(id) = entry.file_name().to_str().map(str::to_string) else {
            continue;
        };
        if validate_learning_id(&id).is_err() {
            continue;
        }
        let path = comments_jsonl_path(root, &id);
        for event in read_comment_events(&path)? {
            match event {
                LearningCommentEvent::Create(comment) => {
                    validate_learning_comment_id(&comment.id)?;
                    if comment.learning_id != id {
                        return Err(OrbitError::Store(format!(
                            "invalid learning comment file {}: comment '{}' belongs to '{}'",
                            path.display(),
                            comment.id,
                            comment.learning_id
                        )));
                    }
                    validate_learning_comment_body(&comment.body)?;
                    validate_learning_comment_model(&comment.author_model)?;
                }
                LearningCommentEvent::Tombstone(tombstone) => {
                    validate_learning_comment_id(&tombstone.id)?;
                    if tombstone.learning_id != id {
                        return Err(OrbitError::Store(format!(
                            "invalid learning comment file {}: tombstone '{}' belongs to '{}'",
                            path.display(),
                            tombstone.id,
                            tombstone.learning_id
                        )));
                    }
                    if tombstone.op != "delete" {
                        return Err(OrbitError::Store(format!(
                            "invalid learning comment file {}: tombstone '{}' has op '{}'",
                            path.display(),
                            tombstone.id,
                            tombstone.op
                        )));
                    }
                    validate_learning_comment_model(&tombstone.deleted_by)?;
                }
            }
        }
    }
    Ok(())
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex, MutexGuard, OnceLock};
    use std::thread;
    use std::time::Instant;

    use chrono::{DateTime, TimeZone, Utc};
    use orbit_common::types::{
        EvidenceKind, LearningCommentEvent, LearningCommentTombstone, LearningEvidence,
        LearningScope, LearningStatus, LearningVoteRow, NotFoundKind, OrbitError,
    };
    use tempfile::{TempDir, tempdir};

    use super::super::layout::{comments_jsonl_path, votes_jsonl_path};
    use super::super::record::append_jsonl_comment_row;
    use super::super::votes::append_vote_row;
    use super::*;

    fn create_params(summary: &str, paths: Vec<&str>, tags: Vec<&str>) -> LearningCreateParams {
        LearningCreateParams {
            summary: summary.to_string(),
            scope: LearningScope {
                paths: paths.into_iter().map(str::to_string).collect(),
                tags: tags.into_iter().map(str::to_string).collect(),
                ..Default::default()
            },
            body: String::new(),
            evidence: Vec::new(),
            created_by: Some("test".to_string()),
            priority: None,
        }
    }

    fn store_with_index() -> (TempDir, LearningFileStore) {
        let dir = tempdir().expect("tempdir");
        let index = Store::open_in_memory().expect("open in-memory store");
        let store = LearningFileStore::new_with_index(dir.path().to_path_buf(), index);
        (dir, store)
    }

    fn upvote_params(id: &str, model: &str, task_id: Option<&str>) -> LearningUpvoteParams {
        LearningUpvoteParams {
            learning_id: id.to_string(),
            voter_model: model.to_string(),
            task_id: task_id.map(str::to_string),
        }
    }

    fn comment_params(id: &str, body: &str) -> LearningCommentAddParams {
        LearningCommentAddParams {
            learning_id: id.to_string(),
            body: body.to_string(),
            author_model: "codex".to_string(),
        }
    }

    fn vote_row(id: &str, model: &str, task_id: &str, voted_at: DateTime<Utc>) -> LearningVoteRow {
        LearningVoteRow {
            learning_id: id.to_string(),
            voter_model: model.to_string(),
            voted_at,
            task_id: Some(task_id.to_string()),
        }
    }

    fn line_count(path: &std::path::Path) -> usize {
        std::fs::read_to_string(path)
            .expect("read votes")
            .lines()
            .filter(|line| !line.trim().is_empty())
            .count()
    }

    struct EnvGuard {
        _lock: MutexGuard<'static, ()>,
        value: Option<String>,
    }

    fn set_half_life_env(value: Option<&str>) -> EnvGuard {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        let lock = LOCK
            .get_or_init(|| Mutex::new(()))
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let previous = std::env::var("ORBIT_LEARNING_VOTE_HALF_LIFE_DAYS").ok();
        unsafe {
            match value {
                Some(value) => std::env::set_var("ORBIT_LEARNING_VOTE_HALF_LIFE_DAYS", value),
                None => std::env::remove_var("ORBIT_LEARNING_VOTE_HALF_LIFE_DAYS"),
            }
        }
        EnvGuard {
            _lock: lock,
            value: previous,
        }
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            unsafe {
                match &self.value {
                    Some(value) => std::env::set_var("ORBIT_LEARNING_VOTE_HALF_LIFE_DAYS", value),
                    None => std::env::remove_var("ORBIT_LEARNING_VOTE_HALF_LIFE_DAYS"),
                }
            }
        }
    }

    // ------ Acceptance criterion: round-trip persistence ------------------

    #[test]
    fn round_trip_persistence_preserves_all_fields_including_phase_two_reservations() {
        let dir = tempdir().expect("tempdir");
        let path_root = dir.path().to_path_buf();
        let (id_to_check, expected) = {
            let index = Store::open_in_memory().expect("index");
            let store = LearningFileStore::new_with_index(path_root.clone(), index);
            let params = LearningCreateParams {
                summary: "Perf-equivalence rule".to_string(),
                scope: LearningScope {
                    paths: vec!["crates/orbit-engine/**/perf*.rs".to_string()],
                    tags: vec!["performance".to_string()],
                    symbols: vec!["orbit_engine::perf_runner::run".to_string()],
                    semantic_seed: Some("output equivalence".to_string()),
                },
                body: "Body explaining how to verify equivalence.".to_string(),
                evidence: vec![LearningEvidence {
                    kind: EvidenceKind::Task,
                    reference: "T20260510-1".to_string(),
                }],
                created_by: Some("claude-opus-4-7".to_string()),
                priority: None,
            };
            let learning = store.create_learning(params).expect("create");
            (learning.id.clone(), learning)
        };

        // Drop the store, reopen — verifies the YAML carries every field.
        let index = Store::open_in_memory().expect("index");
        let store = LearningFileStore::new_with_index(path_root, index);
        let loaded = store
            .get_learning(&id_to_check)
            .expect("get")
            .expect("present");
        assert_eq!(loaded, expected);
        assert_eq!(loaded.scope.symbols, vec!["orbit_engine::perf_runner::run"]);
        assert_eq!(
            loaded.scope.semantic_seed.as_deref(),
            Some("output equivalence")
        );
    }

    // ------ Acceptance criterion: forward-compat YAML fixture --------------

    #[test]
    fn forward_compat_fixture_with_symbols_and_semantic_seed_loads_and_round_trips() {
        let dir = tempdir().expect("tempdir");
        let id = "L20260511-9";
        let yaml = format!(
            "schema_version: 1\n\
             id: {id}\n\
             status: active\n\
             scope:\n\
             \x20\x20paths: []\n\
             \x20\x20tags: []\n\
             \x20\x20symbols:\n\
             \x20\x20\x20\x20- \"a::b\"\n\
             \x20\x20semantic_seed: \"x\"\n\
             summary: Forward-compat fixture\n\
             body: ''\n\
             created_at: 2026-05-11T00:00:00Z\n\
             updated_at: 2026-05-11T00:00:00Z\n"
        );
        let path = dir.path().join(id).join("learning.yaml");
        std::fs::create_dir_all(path.parent().expect("fixture parent")).expect("fixture dir");
        std::fs::write(&path, yaml).expect("write fixture");

        let store = LearningFileStore::new(dir.path().to_path_buf());
        let loaded = store.get_learning(id).expect("get").expect("present");
        assert_eq!(loaded.scope.symbols, vec!["a::b"]);
        assert_eq!(loaded.scope.semantic_seed.as_deref(), Some("x"));

        // Round-trip via update (which rewrites the file).
        store
            .update_learning(
                id,
                LearningUpdateParams {
                    body: Some("touched".to_string()),
                    ..Default::default()
                },
            )
            .expect("update");
        let after = store.get_learning(id).expect("get").expect("present");
        assert_eq!(after.scope.symbols, vec!["a::b"]);
        assert_eq!(after.scope.semantic_seed.as_deref(), Some("x"));
    }

    // ------ Acceptance criterion: ID format + monotonic increment ----------

    #[test]
    fn id_format_increments_within_a_day() {
        let dir = tempdir().expect("tempdir");
        let store = LearningFileStore::new(dir.path().to_path_buf());
        let now = Utc.with_ymd_and_hms(2026, 5, 11, 9, 0, 0).unwrap();

        let first = store
            .create_learning_at(create_params("a", vec![], vec![]), now)
            .expect("first");
        let second = store
            .create_learning_at(create_params("b", vec![], vec![]), now)
            .expect("second");
        let third = store
            .create_learning_at(create_params("c", vec![], vec![]), now)
            .expect("third");

        assert_eq!(first.id, "L20260511-1");
        assert_eq!(second.id, "L20260511-2");
        assert_eq!(third.id, "L20260511-3");
    }

    // ------ Acceptance criterion: migration test for partial index ---------

    #[test]
    fn learnings_index_partial_index_present_after_apply_schema() {
        let store = Store::open_in_memory().expect("open in-memory store");
        let conn_arc = store.connection();
        let conn = conn_arc.lock().expect("lock");

        // Confirm the table exists.
        let table_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master
                 WHERE type = 'table' AND name = 'learnings_index'",
                [],
                |row| row.get(0),
            )
            .expect("query table");
        assert_eq!(table_count, 1);

        // PRAGMA index_list returns rows: (seq, name, unique, origin, partial).
        let mut stmt = conn
            .prepare("PRAGMA index_list('learnings_index')")
            .expect("prepare pragma");
        let rows = stmt
            .query_map([], |row| {
                let name: String = row.get(1)?;
                let partial: i64 = row.get(4)?;
                Ok((name, partial))
            })
            .expect("query pragma");

        let mut found_partial = false;
        for row in rows {
            let (name, partial) = row.expect("row");
            if name == "learnings_active" {
                assert_eq!(partial, 1, "learnings_active must be a partial index");
                found_partial = true;
            }
        }
        assert!(found_partial, "expected learnings_active partial index");
    }

    // ------ Acceptance criterion: index sync on create/update/supersede ----

    #[test]
    fn index_reflects_create_update_and_supersede() {
        let (_dir, store) = store_with_index();
        let learning = store
            .create_learning(create_params("Original", vec!["foo/**"], vec!["alpha"]))
            .expect("create");
        let row = store
            .index
            .as_ref()
            .expect("index")
            .get_learning_index_row(&learning.id)
            .expect("query")
            .expect("present");
        assert_eq!(row.status, LearningStatus::Active);
        assert_eq!(row.paths, vec!["foo/**"]);
        assert_eq!(row.tags, vec!["alpha"]);
        assert_eq!(row.summary, "Original");

        store
            .update_learning(
                &learning.id,
                LearningUpdateParams {
                    summary: Some("Revised".to_string()),
                    scope: Some(LearningScope {
                        paths: vec!["bar/**".to_string()],
                        tags: vec!["beta".to_string()],
                        ..Default::default()
                    }),
                    ..Default::default()
                },
            )
            .expect("update");
        let row = store
            .index
            .as_ref()
            .expect("index")
            .get_learning_index_row(&learning.id)
            .expect("query")
            .expect("present");
        assert_eq!(row.summary, "Revised");
        assert_eq!(row.paths, vec!["bar/**"]);
        assert_eq!(row.tags, vec!["beta"]);

        let new_learning = store
            .create_learning(create_params("Replacement", vec![], vec![]))
            .expect("create new");
        store
            .supersede_learning(&learning.id, &new_learning.id)
            .expect("supersede");

        let old_row = store
            .index
            .as_ref()
            .expect("index")
            .get_learning_index_row(&learning.id)
            .expect("query old")
            .expect("present");
        assert_eq!(old_row.status, LearningStatus::Superseded);

        let new_row = store
            .index
            .as_ref()
            .expect("index")
            .get_learning_index_row(&new_learning.id)
            .expect("query new")
            .expect("present");
        assert_eq!(new_row.status, LearningStatus::Active);
    }

    // ------ Acceptance criterion: glob via promoted helper -----------------

    #[test]
    fn glob_double_star_matches_via_search() {
        let (_dir, store) = store_with_index();
        let target_paths: Vec<String> = vec!["**/perf*.rs".to_string()];
        let _hit = store
            .create_learning(LearningCreateParams {
                summary: "perf rule".to_string(),
                scope: LearningScope {
                    paths: target_paths,
                    ..Default::default()
                },
                body: String::new(),
                evidence: Vec::new(),
                created_by: None,
                priority: None,
            })
            .expect("create hit");

        let hits = store
            .search_learnings(LearningSearchParams {
                path: Some("crates/orbit-engine/perf_runner.rs".to_string()),
                ..Default::default()
            })
            .expect("search");
        assert_eq!(hits.len(), 1, "**/perf*.rs should match perf_runner.rs");
        assert!(
            hits[0]
                .matched_by
                .iter()
                .any(|axis| axis.starts_with("path:"))
        );

        let miss = store
            .search_learnings(LearningSearchParams {
                path: Some("crates/orbit-engine/runner.rs".to_string()),
                ..Default::default()
            })
            .expect("search");
        assert!(miss.is_empty(), "**/perf*.rs should not match runner.rs");
    }

    // ------ Acceptance criterion: scope-OR semantics with dedup ------------

    #[test]
    fn scope_or_matches_paths_only_tags_only_and_both_with_dedup() {
        let (_dir, store) = store_with_index();
        let paths_only = store
            .create_learning(create_params("paths only", vec!["foo/**"], vec![]))
            .expect("paths only");
        let tags_only = store
            .create_learning(create_params("tags only", vec![], vec!["perf"]))
            .expect("tags only");
        let both = store
            .create_learning(create_params("both", vec!["foo/**"], vec!["perf"]))
            .expect("both");

        // Path search finds paths_only and both, not tags_only.
        let by_path = store
            .search_learnings(LearningSearchParams {
                path: Some("foo/bar.rs".to_string()),
                ..Default::default()
            })
            .expect("by path");
        let ids: Vec<String> = by_path.iter().map(|r| r.learning.id.clone()).collect();
        assert!(ids.contains(&paths_only.id));
        assert!(ids.contains(&both.id));
        assert!(!ids.contains(&tags_only.id));

        // Tag search finds tags_only and both, not paths_only.
        let by_tag = store
            .search_learnings(LearningSearchParams {
                tag: Some("perf".to_string()),
                ..Default::default()
            })
            .expect("by tag");
        let ids: Vec<String> = by_tag.iter().map(|r| r.learning.id.clone()).collect();
        assert!(ids.contains(&tags_only.id));
        assert!(ids.contains(&both.id));
        assert!(!ids.contains(&paths_only.id));

        // Combined: every learning surfaces exactly once; `both` matches on
        // both axes.
        let combined = store
            .search_learnings(LearningSearchParams {
                path: Some("foo/bar.rs".to_string()),
                tag: Some("perf".to_string()),
                ..Default::default()
            })
            .expect("combined");
        let ids: Vec<String> = combined.iter().map(|r| r.learning.id.clone()).collect();
        assert_eq!(ids.len(), 3);
        let both_result = combined
            .iter()
            .find(|r| r.learning.id == both.id)
            .expect("both present");
        assert!(
            both_result
                .matched_by
                .iter()
                .any(|a| a.starts_with("path:"))
        );
        assert!(both_result.matched_by.iter().any(|a| a.starts_with("tag:")));
    }

    // ------ Acceptance criterion: supersession atomicity -------------------

    #[test]
    fn supersession_moves_yaml_and_updates_both_records() {
        let (_dir, store) = store_with_index();
        let old = store
            .create_learning(create_params("old", vec![], vec![]))
            .expect("old");
        let new = store
            .create_learning(create_params("new", vec![], vec![]))
            .expect("new");

        store
            .supersede_learning(&old.id, &new.id)
            .expect("supersede");

        let loaded_old = store
            .get_learning(&old.id)
            .expect("get old")
            .expect("present");
        let loaded_new = store
            .get_learning(&new.id)
            .expect("get new")
            .expect("present");

        assert_eq!(loaded_old.status, LearningStatus::Superseded);
        assert_eq!(loaded_old.superseded_by.as_deref(), Some(new.id.as_str()));
        assert_eq!(loaded_new.supersedes.as_deref(), Some(old.id.as_str()));
        assert_eq!(loaded_new.status, LearningStatus::Active);

        // Both index rows reflect new state.
        let index = store.index.as_ref().expect("index");
        let old_row = index
            .get_learning_index_row(&old.id)
            .expect("query old")
            .expect("present");
        let new_row = index
            .get_learning_index_row(&new.id)
            .expect("query new")
            .expect("present");
        assert_eq!(old_row.status, LearningStatus::Superseded);
        assert_eq!(new_row.status, LearningStatus::Active);
    }

    // ------ Acceptance criterion: reindex from YAML ------------------------

    #[test]
    fn reindex_rebuilds_index_from_yaml() {
        let (_dir, store) = store_with_index();
        let _a = store
            .create_learning(create_params("a", vec!["a/**"], vec!["x"]))
            .expect("a");
        let _b = store
            .create_learning(create_params("b", vec!["b/**"], vec!["y"]))
            .expect("b");

        // Wipe the index from under the store.
        store
            .index
            .as_ref()
            .expect("index")
            .truncate_learning_index()
            .expect("truncate");
        let active = store
            .index
            .as_ref()
            .expect("index")
            .list_active_learning_rows()
            .expect("list");
        assert!(active.is_empty());

        store.reindex_learnings().expect("reindex");
        let active = store
            .index
            .as_ref()
            .expect("index")
            .list_active_learning_rows()
            .expect("list");
        assert_eq!(active.len(), 2);
    }

    #[test]
    fn migrate_layout_preserves_list_parity_and_reindex_projection() {
        let dir = tempdir().expect("tempdir");
        let root = dir.path().join("learnings");
        std::fs::create_dir_all(root.join("superseded")).expect("legacy dirs");
        std::fs::write(
            root.join("L20260517-1.yaml"),
            legacy_learning_yaml("L20260517-1", "active", "Active rule", 10),
        )
        .expect("active yaml");
        std::fs::write(
            root.join("superseded").join("L20260517-2.yaml"),
            legacy_learning_yaml("L20260517-2", "superseded", "Old rule", 20),
        )
        .expect("superseded yaml");

        let legacy_active = read_learning_file(&root.join("L20260517-1.yaml")).expect("active");
        let legacy_superseded =
            read_learning_file(&root.join("superseded").join("L20260517-2.yaml"))
                .expect("superseded");
        let index = Store::open_in_memory().expect("index");
        index
            .upsert_learning_index_row(&legacy_active)
            .expect("index active");
        index
            .upsert_learning_index_row(&legacy_superseded)
            .expect("index superseded");
        let before_active_row = index
            .get_learning_index_row("L20260517-1")
            .expect("row active")
            .expect("present");
        let before_superseded_row = index
            .get_learning_index_row("L20260517-2")
            .expect("row superseded")
            .expect("present");

        super::super::migration::migrate_learning_layout(&root, dir.path()).expect("migrate");
        let store = LearningFileStore::new_with_index(root.clone(), index.clone());
        store.reindex_learnings().expect("reindex");

        let active = store
            .list_learnings(Some(LearningStatus::Active))
            .expect("list active");
        let superseded = store
            .list_learnings(Some(LearningStatus::Superseded))
            .expect("list superseded");
        assert_eq!(active, vec![legacy_active]);
        assert_eq!(superseded, vec![legacy_superseded]);
        assert_eq!(
            index
                .get_learning_index_row("L20260517-1")
                .expect("row active after")
                .expect("present"),
            before_active_row
        );
        assert_eq!(
            index
                .get_learning_index_row("L20260517-2")
                .expect("row superseded after")
                .expect("present"),
            before_superseded_row
        );
    }

    // ------ Acceptance criterion: layout assertions -----------------------

    #[test]
    fn layout_places_files_at_expected_paths_and_gitignore_is_respected() {
        let (dir, store) = store_with_index();
        let learning = store
            .create_learning(create_params("layout", vec![], vec![]))
            .expect("create");
        let active_path = dir.path().join(&learning.id).join("learning.yaml");
        assert!(
            active_path.is_file(),
            "active file at {}",
            active_path.display()
        );

        let new = store
            .create_learning(create_params("replacement", vec![], vec![]))
            .expect("replacement");
        store
            .supersede_learning(&learning.id, &new.id)
            .expect("supersede");
        let superseded_path = dir.path().join(&learning.id).join("learning.yaml");
        assert!(
            superseded_path.is_file(),
            "superseded file at {}",
            superseded_path.display()
        );
        assert!(
            active_path.is_file(),
            "superseded status stays in the per-entity YAML"
        );

        // Repo `.gitignore` content check: `.orbit/learnings/` must not be
        // effectively ignored (ADR-003 says learnings travel with the repo);
        // `.orbit/state/` must be ignored (rebuildable index is not checked in).
        let gitignore_path =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.gitignore");
        let gitignore = std::fs::read_to_string(&gitignore_path).expect("read .gitignore");
        let lines: Vec<&str> = gitignore.lines().map(|l| l.trim()).collect();
        assert!(
            !lines
                .iter()
                .any(|l| *l == ".orbit/learnings/" || *l == ".orbit/learnings"),
            ".gitignore must not explicitly ignore .orbit/learnings/",
        );
        let has_blanket = lines
            .iter()
            .any(|l| matches!(*l, ".orbit/" | ".orbit" | ".orbit/*"));
        let has_unignore = lines
            .iter()
            .any(|l| *l == "!.orbit/learnings/" || *l == "!.orbit/learnings/**");
        assert!(
            !has_blanket || has_unignore,
            ".gitignore has a blanket `.orbit/` rule but no `!.orbit/learnings/` re-include — learnings would not be tracked",
        );
        let ignores_state = has_blanket
            || lines
                .iter()
                .any(|l| matches!(*l, ".orbit/state/" | ".orbit/state"));
        assert!(
            ignores_state,
            ".gitignore must ignore .orbit/state/ (or the wider .orbit/) so the rebuildable index is not checked in",
        );
    }

    #[test]
    fn upvote_creates_lazy_votes_file_and_show_summary_reads_it() {
        let (dir, store) = store_with_index();
        let learning = store
            .create_learning(create_params("vote target", vec![], vec![]))
            .expect("create");
        let votes_path = votes_jsonl_path(dir.path(), &learning.id);
        assert!(!votes_path.exists(), "votes file should be lazy");

        let now = Utc.with_ymd_and_hms(2026, 5, 17, 12, 0, 0).unwrap();
        let summary = store
            .upvote_learning_at(upvote_params(&learning.id, "claude", Some("ORB-1")), now)
            .expect("upvote");

        assert_eq!(summary.vote_count, 1);
        assert_eq!(summary.last_voted_at, Some(now));
        assert!(votes_path.is_file());
        assert_eq!(line_count(&votes_path), 1);

        let reread = store.learning_vote_summary(&learning.id).expect("summary");
        assert_eq!(reread, summary);
    }

    #[test]
    fn duplicate_upvote_same_key_is_noop_but_cross_task_counts() {
        let (dir, store) = store_with_index();
        let learning = store
            .create_learning(create_params("vote target", vec![], vec![]))
            .expect("create");
        let first = Utc.with_ymd_and_hms(2026, 5, 17, 12, 0, 0).unwrap();
        let second = Utc.with_ymd_and_hms(2026, 5, 17, 13, 0, 0).unwrap();
        let third = Utc.with_ymd_and_hms(2026, 5, 17, 14, 0, 0).unwrap();

        store
            .upvote_learning_at(upvote_params(&learning.id, "claude", Some("ORB-1")), first)
            .expect("first");
        let duplicate = store
            .upvote_learning_at(upvote_params(&learning.id, "claude", Some("ORB-1")), second)
            .expect("duplicate");
        assert_eq!(duplicate.vote_count, 1);
        assert_eq!(duplicate.last_voted_at, Some(first));
        assert_eq!(line_count(&votes_jsonl_path(dir.path(), &learning.id)), 1);

        let cross_task = store
            .upvote_learning_at(upvote_params(&learning.id, "claude", Some("ORB-2")), third)
            .expect("cross task");
        assert_eq!(cross_task.vote_count, 2);
        assert_eq!(cross_task.last_voted_at, Some(third));
        assert_eq!(line_count(&votes_jsonl_path(dir.path(), &learning.id)), 2);
    }

    #[test]
    fn upvote_rejects_missing_task_missing_learning_and_superseded_learning() {
        let (dir, store) = store_with_index();
        let learning = store
            .create_learning(create_params("vote target", vec![], vec![]))
            .expect("create");

        let error = store
            .upvote_learning(upvote_params(&learning.id, "claude", None))
            .expect_err("missing task rejected");
        assert!(
            matches!(error, OrbitError::InvalidInput(message) if message.contains("free-floating votes"))
        );
        assert!(!votes_jsonl_path(dir.path(), &learning.id).exists());

        let error = store
            .upvote_learning(upvote_params("L20260517-404", "claude", Some("ORB-1")))
            .expect_err("missing learning rejected");
        assert!(matches!(
            error,
            OrbitError::NotFound {
                kind: NotFoundKind::Learning,
                ..
            }
        ));
        assert!(
            !dir.path()
                .join("L20260517-404")
                .join("votes.jsonl")
                .exists()
        );

        let replacement = store
            .create_learning(create_params("replacement", vec![], vec![]))
            .expect("replacement");
        store
            .supersede_learning(&learning.id, &replacement.id)
            .expect("supersede");
        let error = store
            .upvote_learning(upvote_params(&learning.id, "claude", Some("ORB-2")))
            .expect_err("superseded rejected");
        assert!(
            matches!(error, OrbitError::InvalidInput(message) if message.contains("superseded"))
        );
    }

    #[test]
    fn per_learning_vote_files_are_isolated() {
        let (_dir, store) = store_with_index();
        let a = store
            .create_learning(create_params("a", vec![], vec![]))
            .expect("a");
        let b = store
            .create_learning(create_params("b", vec![], vec![]))
            .expect("b");
        store
            .upvote_learning(upvote_params(&a.id, "claude", Some("ORB-1")))
            .expect("vote a");

        assert_eq!(
            store
                .learning_vote_summary(&a.id)
                .expect("summary a")
                .vote_count,
            1
        );
        assert_eq!(
            store
                .learning_vote_summary(&b.id)
                .expect("summary b")
                .vote_count,
            0
        );
    }

    #[test]
    fn concurrent_upvotes_append_complete_json_lines() {
        let dir = tempdir().expect("tempdir");
        let store = Arc::new(LearningFileStore::new(dir.path().to_path_buf()));
        let learning = store
            .create_learning(create_params("concurrent", vec![], vec![]))
            .expect("create");
        let n = 12;
        let mut handles = Vec::new();
        for idx in 0..n {
            let store = Arc::clone(&store);
            let learning_id = learning.id.clone();
            handles.push(thread::spawn(move || {
                store
                    .upvote_learning(upvote_params(
                        &learning_id,
                        "claude",
                        Some(&format!("ORB-{idx}")),
                    ))
                    .expect("upvote");
            }));
        }
        for handle in handles {
            handle.join().expect("thread join");
        }

        let votes_path = votes_jsonl_path(dir.path(), &learning.id);
        let rows = super::super::votes::read_vote_rows(&votes_path).expect("read rows");
        assert_eq!(rows.len(), n);
        assert_eq!(
            store
                .learning_vote_summary(&learning.id)
                .expect("summary")
                .vote_count,
            n
        );
    }

    #[test]
    fn search_ranks_recent_decayed_votes_ahead_of_many_old_votes() {
        let _env = set_half_life_env(None);
        let (dir, store) = store_with_index();
        let now = Utc.with_ymd_and_hms(2026, 5, 17, 12, 0, 0).unwrap();
        let recent = store
            .create_learning(create_params("recent", vec!["foo/**"], vec![]))
            .expect("recent");
        let old = store
            .create_learning(create_params("old", vec!["foo/**"], vec![]))
            .expect("old");

        append_vote_row(
            &votes_jsonl_path(dir.path(), &recent.id),
            &vote_row(
                &recent.id,
                "claude",
                "ORB-recent",
                now - chrono::Duration::days(30),
            ),
        )
        .expect("recent vote");
        for idx in 0..3 {
            append_vote_row(
                &votes_jsonl_path(dir.path(), &old.id),
                &vote_row(
                    &old.id,
                    "claude",
                    &format!("ORB-old-{idx}"),
                    now - chrono::Duration::days(730 + idx),
                ),
            )
            .expect("old vote");
        }

        let hits = store
            .search_learnings_at(
                LearningSearchParams {
                    path: Some("foo/bar.rs".to_string()),
                    ..Default::default()
                },
                now,
            )
            .expect("search");

        assert_eq!(hits[0].learning.id, recent.id);
    }

    #[test]
    fn zero_half_life_disables_decay_for_search_ranking() {
        let _env = set_half_life_env(Some("0"));
        let (dir, store) = store_with_index();
        let now = Utc.with_ymd_and_hms(2026, 5, 17, 12, 0, 0).unwrap();
        let recent = store
            .create_learning(create_params("recent", vec!["foo/**"], vec![]))
            .expect("recent");
        let old = store
            .create_learning(create_params("old", vec!["foo/**"], vec![]))
            .expect("old");

        append_vote_row(
            &votes_jsonl_path(dir.path(), &recent.id),
            &vote_row(
                &recent.id,
                "claude",
                "ORB-recent",
                now - chrono::Duration::days(30),
            ),
        )
        .expect("recent vote");
        for idx in 0..3 {
            append_vote_row(
                &votes_jsonl_path(dir.path(), &old.id),
                &vote_row(
                    &old.id,
                    "claude",
                    &format!("ORB-old-{idx}"),
                    now - chrono::Duration::days(730 + idx),
                ),
            )
            .expect("old vote");
        }

        let hits = store
            .search_learnings_at(
                LearningSearchParams {
                    path: Some("foo/bar.rs".to_string()),
                    ..Default::default()
                },
                now,
            )
            .expect("search");

        assert_eq!(hits[0].learning.id, old.id);
    }

    #[test]
    fn reindex_validates_votes_and_external_valid_lines_are_visible() {
        let (dir, store) = store_with_index();
        let learning = store
            .create_learning(create_params("target", vec![], vec![]))
            .expect("create");
        let votes_path = votes_jsonl_path(dir.path(), &learning.id);
        let row = vote_row(
            &learning.id,
            "claude",
            "ORB-external",
            Utc.with_ymd_and_hms(2026, 5, 17, 12, 0, 0).unwrap(),
        );
        append_vote_row(&votes_path, &row).expect("append external");

        store.reindex_learnings().expect("reindex");
        assert_eq!(
            store
                .learning_vote_summary(&learning.id)
                .expect("summary")
                .vote_count,
            1
        );

        std::fs::write(&votes_path, b"{not-json}\n").expect("write invalid");
        let error = store.reindex_learnings().expect_err("invalid vote line");
        assert!(matches!(error, OrbitError::Store(message) if message.contains("line 1")));
    }

    #[test]
    fn learning_comments_round_trip_and_create_file_lazily() {
        let dir = tempdir().expect("tempdir");
        let store = LearningFileStore::new(dir.path().to_path_buf());
        let learning = store
            .create_learning(create_params("target", vec![], vec![]))
            .expect("create");
        let comments_path = comments_jsonl_path(dir.path(), &learning.id);
        assert!(!comments_path.exists());

        let now = Utc.with_ymd_and_hms(2026, 5, 17, 12, 0, 0).unwrap();
        let comment = store
            .add_learning_comment_at(comment_params(&learning.id, "  useful note  "), now)
            .expect("comment");

        assert_eq!(comment.id, "C20260517-1");
        assert_eq!(comment.body, "useful note");
        assert!(comments_path.exists());
        let listed = store
            .list_learning_comments(&learning.id, false)
            .expect("list");
        assert_eq!(listed, vec![comment]);
    }

    #[test]
    fn learning_comment_validation_rejects_bad_bodies_and_missing_parent_before_file_creation() {
        let dir = tempdir().expect("tempdir");
        let store = LearningFileStore::new(dir.path().to_path_buf());
        let learning = store
            .create_learning(create_params("target", vec![], vec![]))
            .expect("create");

        let too_long = "x".repeat(501);
        for body in ["", "   ", too_long.as_str()] {
            let error = store
                .add_learning_comment(comment_params(&learning.id, body))
                .expect_err("invalid body");
            assert!(matches!(error, OrbitError::InvalidInput(_)));
        }

        let missing = "L20260517-404";
        let error = store
            .add_learning_comment(comment_params(missing, "valid"))
            .expect_err("missing parent");
        assert!(matches!(
            error,
            OrbitError::NotFound {
                kind: NotFoundKind::Learning,
                ..
            }
        ));
        assert!(!comments_jsonl_path(dir.path(), missing).exists());
    }

    #[test]
    fn learning_comment_rejects_superseded_parent_before_append() {
        let dir = tempdir().expect("tempdir");
        let store = LearningFileStore::new(dir.path().to_path_buf());
        let old = store
            .create_learning(create_params("old", vec![], vec![]))
            .expect("old");
        let new = store
            .create_learning(create_params("new", vec![], vec![]))
            .expect("new");
        store
            .supersede_learning(&old.id, &new.id)
            .expect("supersede");

        let error = store
            .add_learning_comment(comment_params(&old.id, "valid"))
            .expect_err("superseded");
        assert!(
            matches!(error, OrbitError::InvalidInput(message) if message.contains("orbit.learning.supersede"))
        );
        assert!(!comments_jsonl_path(dir.path(), &old.id).exists());
    }

    #[test]
    fn superseding_learning_leaves_comments_on_original_parent_only() {
        let dir = tempdir().expect("tempdir");
        let store = LearningFileStore::new(dir.path().to_path_buf());
        let old = store
            .create_learning(create_params("old", vec![], vec![]))
            .expect("old");
        let new = store
            .create_learning(create_params("new", vec![], vec![]))
            .expect("new");
        let comment = store
            .add_learning_comment(comment_params(&old.id, "old note"))
            .expect("comment");

        store
            .supersede_learning(&old.id, &new.id)
            .expect("supersede");

        assert!(comments_jsonl_path(dir.path(), &old.id).exists());
        assert_eq!(
            store
                .list_learning_comments(&old.id, false)
                .expect("old comments"),
            vec![comment]
        );
        assert!(
            store
                .list_learning_comments(&new.id, false)
                .expect("new comments")
                .is_empty()
        );
    }

    #[test]
    fn learning_comment_delete_is_tombstone_idempotent_and_include_deleted_restores() {
        let dir = tempdir().expect("tempdir");
        let store = LearningFileStore::new(dir.path().to_path_buf());
        let learning = store
            .create_learning(create_params("target", vec![], vec![]))
            .expect("create");
        let comment = store
            .add_learning_comment_at(
                comment_params(&learning.id, "delete me"),
                Utc.with_ymd_and_hms(2026, 5, 17, 12, 0, 0).unwrap(),
            )
            .expect("comment");
        let path = comments_jsonl_path(dir.path(), &learning.id);

        store
            .delete_learning_comment(LearningCommentDeleteParams {
                comment_id: comment.id.clone(),
                deleted_by: "codex".to_string(),
            })
            .expect("delete");
        store
            .delete_learning_comment(LearningCommentDeleteParams {
                comment_id: comment.id.clone(),
                deleted_by: "codex".to_string(),
            })
            .expect("delete again");

        assert!(
            store
                .list_learning_comments(&learning.id, false)
                .expect("list active")
                .is_empty()
        );
        assert_eq!(
            store
                .list_learning_comments(&learning.id, true)
                .expect("list deleted"),
            vec![comment]
        );
        assert_eq!(line_count(&path), 2);
    }

    #[test]
    fn tombstone_before_create_suppresses_comment_on_read() {
        let dir = tempdir().expect("tempdir");
        let store = LearningFileStore::new(dir.path().to_path_buf());
        let learning = store
            .create_learning(create_params("target", vec![], vec![]))
            .expect("create");
        let path = comments_jsonl_path(dir.path(), &learning.id);
        let ts = Utc.with_ymd_and_hms(2026, 5, 17, 12, 0, 0).unwrap();
        append_jsonl_comment_row(
            &path,
            &LearningCommentEvent::Tombstone(LearningCommentTombstone {
                id: "C20260517-1".to_string(),
                learning_id: learning.id.clone(),
                op: "delete".to_string(),
                deleted_at: ts,
                deleted_by: "codex".to_string(),
            }),
        )
        .expect("append tombstone");
        append_jsonl_comment_row(
            &path,
            &LearningCommentEvent::Create(orbit_common::types::LearningComment {
                id: "C20260517-1".to_string(),
                learning_id: learning.id.clone(),
                body: "late create".to_string(),
                author_model: "codex".to_string(),
                created_at: ts,
            }),
        )
        .expect("append create");

        assert!(
            store
                .list_learning_comments(&learning.id, true)
                .expect("list")
                .is_empty()
        );
    }

    #[test]
    fn concurrent_learning_comment_adds_persist_complete_lines() {
        let dir = tempdir().expect("tempdir");
        let store = Arc::new(LearningFileStore::new(dir.path().to_path_buf()));
        let learning = store
            .create_learning(create_params("target", vec![], vec![]))
            .expect("create");
        let mut handles = Vec::new();
        for idx in 0..16 {
            let store = Arc::clone(&store);
            let learning_id = learning.id.clone();
            handles.push(thread::spawn(move || {
                store
                    .add_learning_comment(comment_params(&learning_id, &format!("comment {idx}")))
                    .expect("add comment")
            }));
        }
        let comments: Vec<_> = handles
            .into_iter()
            .map(|handle| handle.join().expect("join"))
            .collect();
        let path = comments_jsonl_path(dir.path(), &learning.id);
        let raw = std::fs::read_to_string(&path).expect("read comments");

        assert_eq!(comments.len(), 16);
        assert_eq!(raw.lines().count(), 16);
        for line in raw.lines() {
            let value: serde_json::Value = serde_json::from_str(line).expect("line json");
            assert!(
                value
                    .get("id")
                    .and_then(serde_json::Value::as_str)
                    .is_some()
            );
        }
        let listed = store
            .list_learning_comments(&learning.id, false)
            .expect("list");
        assert_eq!(listed.len(), 16);
    }

    #[test]
    fn reindex_validates_comments_and_external_valid_lines_are_visible() {
        let (dir, store) = store_with_index();
        let learning = store
            .create_learning(create_params("target", vec![], vec![]))
            .expect("create");
        let path = comments_jsonl_path(dir.path(), &learning.id);
        let ts = Utc.with_ymd_and_hms(2026, 5, 17, 12, 0, 0).unwrap();
        append_jsonl_comment_row(
            &path,
            &LearningCommentEvent::Create(orbit_common::types::LearningComment {
                id: "C20260517-1".to_string(),
                learning_id: learning.id.clone(),
                body: "external note".to_string(),
                author_model: "codex".to_string(),
                created_at: ts,
            }),
        )
        .expect("append external");

        store.reindex_learnings().expect("reindex");
        assert_eq!(
            store
                .list_learning_comments(&learning.id, false)
                .expect("list")[0]
                .body,
            "external note"
        );

        std::fs::write(&path, b"{not-json}\n").expect("write invalid");
        let error = store.reindex_learnings().expect_err("invalid comment line");
        assert!(matches!(error, OrbitError::Store(message) if message.contains("line 1")));
    }

    fn legacy_learning_yaml(id: &str, status: &str, summary: &str, priority: u8) -> String {
        let second = priority % 10;
        format!(
            "schema_version: 1\n\
             id: {id}\n\
             status: {status}\n\
             scope:\n\
             \x20\x20paths:\n\
             \x20\x20\x20\x20- crates/orbit-store/**\n\
             \x20\x20tags:\n\
             \x20\x20\x20\x20- migration\n\
             summary: {summary}\n\
             body: body for {id}\n\
             evidence:\n\
             \x20\x20- kind: task\n\
             \x20\x20\x20\x20reference: ORB-00096\n\
             created_at: 2026-05-17T00:00:00Z\n\
             updated_at: 2026-05-17T00:00:0{second}Z\n\
             created_by: codex\n\
             priority: {priority}\n"
        )
    }

    // ------ Acceptance criterion: latency benchmark (gated) ----------------

    #[test]
    #[ignore]
    fn learning_search_latency_p50_under_10ms_at_500_records() {
        let (_dir, store) = store_with_index();

        // Seed 500 active learnings with varied scopes.
        let path_pool = [
            "crates/orbit-engine/**/perf*.rs",
            "crates/orbit-knowledge/**/*.rs",
            "crates/orbit-tools/**/handlers/*.rs",
            "benchmarks/**/*.rs",
            "docs/**/*.md",
        ];
        let tag_pool = ["performance", "knowledge", "tools", "bench", "docs"];

        for i in 0..500 {
            let path = path_pool[i % path_pool.len()].to_string();
            let tag = tag_pool[i % tag_pool.len()].to_string();
            store
                .create_learning(LearningCreateParams {
                    summary: format!("Learning {i}"),
                    scope: LearningScope {
                        paths: vec![path],
                        tags: vec![tag],
                        ..Default::default()
                    },
                    body: String::new(),
                    evidence: Vec::new(),
                    created_by: Some("bench".to_string()),
                    priority: None,
                })
                .expect("seed");
        }

        // 1000 search calls against a representative path.
        let mut durations_ns: Vec<u128> = Vec::with_capacity(1000);
        for _ in 0..1000 {
            let start = Instant::now();
            let _ = store
                .search_learnings(LearningSearchParams {
                    path: Some("crates/orbit-engine/perf_runner.rs".to_string()),
                    limit: Some(5),
                    ..Default::default()
                })
                .expect("search");
            durations_ns.push(start.elapsed().as_nanos());
        }
        durations_ns.sort_unstable();
        let p = |q: f64| -> u128 {
            let idx = ((durations_ns.len() as f64) * q).floor() as usize;
            durations_ns[idx.min(durations_ns.len() - 1)]
        };
        let p50_ns = p(0.50);
        let p95_ns = p(0.95);
        let p99_ns = p(0.99);
        let p50_ms = (p50_ns as f64) / 1_000_000.0;
        let p95_ms = (p95_ns as f64) / 1_000_000.0;
        let p99_ms = (p99_ns as f64) / 1_000_000.0;
        // Print methodology + raw numbers to stdout. This bench is gated as
        // `#[ignore]` so default `cargo test` skips it; run explicitly with
        // `cargo test -p orbit-store --release --lib learning_search_latency
        // -- --ignored --nocapture`.
        #[allow(clippy::print_stdout)]
        {
            println!(
                "learning_search_latency: 500 records, 1000 calls, target path=crates/orbit-engine/perf_runner.rs"
            );
            println!(
                "learning_search_latency: p50={p50_ms:.3}ms p95={p95_ms:.3}ms p99={p99_ms:.3}ms"
            );
        }
        assert!(
            p50_ms < 10.0,
            "median search latency must be < 10ms; got {p50_ms:.3}ms (p95={p95_ms:.3}ms p99={p99_ms:.3}ms)"
        );
    }
}
