// ORB-00013: Existing expect calls in this module document local invariants; keep the allow scoped while the workspace lint is ratcheted.
#![allow(clippy::expect_used)]

use std::env;
use std::sync::Arc;

use chrono::{DateTime, Utc};
use orbit_common::types::{Learning, LearningStatus, OrbitError, decayed_vote_score};
use orbit_common::utility::glob::{compile_glob_regex, normalize_glob_path};

use super::super::votes::validate_vote_files;
use super::super::votes::{deduped_vote_times, read_vote_rows};
use super::store::LearningFileStore;
use crate::backend::{LearningSearchParams, LearningSearchResult};

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
    /// Rebuild the SQLite index from the YAML source of truth.
    ///
    /// No-op when no index is attached; otherwise wipes
    /// `learnings_index` and reinserts every record found on disk.
    pub(crate) fn reindex_learnings(&self) -> Result<(), OrbitError> {
        validate_vote_files(&self.root)?;
        super::validation::validate_comment_files(&self.root)?;
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
            let vote_times = deduped_vote_times(&read_vote_rows(
                &super::super::layout::votes_jsonl_path(&self.root, &envelope.id),
            )?);
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

    pub(super) fn invalidate_envelope_cache(&self) {
        if let Ok(mut guard) = self.envelope_cache.write() {
            *guard = None;
        }
    }

    pub(super) fn upsert_index_row(&self, learning: &Learning) {
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
