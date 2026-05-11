use std::fs;
use std::path::PathBuf;
use std::sync::{Arc, RwLock};

use chrono::{DateTime, Utc};
use orbit_common::types::{
    Learning, LearningStatus, OrbitError, normalize_learning_paths, normalize_learning_tags,
};
use orbit_common::utility::glob::{compile_glob_regex, normalize_glob_path};

use super::layout::{
    LearningStateDir, learning_doc_path, locate_learning, move_learning_dir, next_learning_id,
    state_dir_path, validate_learning_id,
};
use super::lock::{acquire_learning_allocation_lock, acquire_learning_lock};
use super::record::{read_learning_file, write_learning_file};
use crate::Store;
use crate::backend::{
    LearningCreateParams, LearningSearchParams, LearningSearchResult, LearningUpdateParams,
};

/// Workspace-scoped, filesystem-backed learning store.
///
/// YAML files at `<root>/<id>.yaml` (active) and `<root>/superseded/<id>.yaml`
/// (superseded) are the source of truth. When `index` is attached, envelope
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
        };

        let path = learning_doc_path(&self.root, LearningStateDir::Active, &id);
        write_learning_file(&path, &learning, LearningStatus::Active)?;
        self.upsert_index_row(&learning);
        self.invalidate_envelope_cache();
        Ok(learning)
    }

    pub(crate) fn get_learning(&self, id: &str) -> Result<Option<Learning>, OrbitError> {
        validate_learning_id(id)?;
        let Some((_, path)) = locate_learning(&self.root, id)? else {
            return Ok(None);
        };
        Ok(Some(read_learning_file(&path)?))
    }

    pub(crate) fn list_learnings(
        &self,
        status: Option<LearningStatus>,
    ) -> Result<Vec<Learning>, OrbitError> {
        let mut out = Vec::new();
        for state in LearningStateDir::all() {
            if let Some(s) = status
                && state.to_status() != s
            {
                continue;
            }
            let dir = state_dir_path(&self.root, *state);
            if !dir.exists() {
                continue;
            }
            for entry in fs::read_dir(&dir).map_err(|e| OrbitError::Io(e.to_string()))? {
                let entry = entry.map_err(|e| OrbitError::Io(e.to_string()))?;
                let file_type = entry
                    .file_type()
                    .map_err(|e| OrbitError::Io(e.to_string()))?;
                if !file_type.is_file() {
                    continue;
                }
                let path = entry.path();
                let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
                    continue;
                };
                if !name.ends_with(".yaml") {
                    continue;
                }
                let learning = read_learning_file(&path)?;
                out.push(learning);
            }
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

        let Some((state, path)) = locate_learning(&self.root, id)? else {
            return Err(OrbitError::LearningNotFound(id.to_string()));
        };
        let mut learning = read_learning_file(&path)?;

        if let Some(summary) = params.summary {
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
        learning.updated_at = Utc::now();
        write_learning_file(&path, &learning, state.to_status())?;
        self.upsert_index_row(&learning);
        self.invalidate_envelope_cache();
        Ok(learning)
    }

    /// Atomically supersede `old_id` with `new_id`. Phase-1 contract:
    /// 1. Both records exist.
    /// 2. `old.status` flips to `Superseded`, `old.superseded_by = new_id`,
    ///    and the YAML moves under `superseded/`.
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

        let (old_state, old_path) = locate_learning(&self.root, old_id)?
            .ok_or_else(|| OrbitError::LearningNotFound(old_id.to_string()))?;
        let (_, new_path) = locate_learning(&self.root, new_id)?
            .ok_or_else(|| OrbitError::LearningNotFound(new_id.to_string()))?;

        let mut old = read_learning_file(&old_path)?;
        let mut new = read_learning_file(&new_path)?;

        let now = Utc::now();
        old.status = LearningStatus::Superseded;
        old.superseded_by = Some(new_id.to_string());
        old.updated_at = now;

        new.supersedes = Some(old_id.to_string());
        new.updated_at = now;

        let new_target_path = learning_doc_path(&self.root, LearningStateDir::Superseded, old_id);

        // 1. Write the updated `new` record first; if anything below fails
        //    we can still recover the old state by re-reading from disk.
        write_learning_file(&new_path, &new, new.status)?;
        // 2. Write the updated `old` content at its current path so the move
        //    only has to rename a fully-current file.
        write_learning_file(&old_path, &old, LearningStatus::Superseded)?;
        // 3. Move the active YAML into `superseded/` unless it's already
        //    there.
        if old_state != LearningStateDir::Superseded {
            move_learning_dir(&old_path, &new_target_path)?;
        }

        self.upsert_index_row(&old);
        self.upsert_index_row(&new);
        self.invalidate_envelope_cache();
        Ok(())
    }

    pub(crate) fn delete_learning(&self, id: &str) -> Result<bool, OrbitError> {
        validate_learning_id(id)?;
        let _lock = acquire_learning_lock(&self.root, id)?;

        let Some((_, path)) = locate_learning(&self.root, id)? else {
            return Ok(false);
        };
        fs::remove_file(&path).map_err(|e| OrbitError::Io(e.to_string()))?;
        if let Some(index) = &self.index {
            index.delete_learning_index_row(id)?;
        }
        self.invalidate_envelope_cache();
        Ok(true)
    }

    /// Rebuild the SQLite index from the YAML source of truth.
    ///
    /// No-op when no index is attached; otherwise wipes
    /// `learnings_index` and reinserts every record found on disk.
    pub(crate) fn reindex_learnings(&self) -> Result<(), OrbitError> {
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

        // Sort by `updated_at` desc. RFC3339 string compare is correct here
        // because `Learning::updated_at` is `DateTime<Utc>` (always `Z`
        // suffix) so the string ordering matches the chronological one.
        matched.sort_by(|a, b| {
            b.0.updated_at_key
                .cmp(&a.0.updated_at_key)
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
                created_at: updated_at,
                updated_at,
                created_by: None,
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
                .map(|row| build_envelope(row.id, row.paths, row.tags, row.summary, row.updated_at))
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
}

fn build_envelope(
    id: String,
    paths: Vec<String>,
    tags: Vec<String>,
    summary: String,
    updated_at_key: String,
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
    }
}

fn parse_rfc3339_or_epoch(raw: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(raw)
        .map(|dt| dt.with_timezone(&Utc))
        .unwrap_or_else(|_| DateTime::<Utc>::from_timestamp(0, 0).expect("epoch is valid"))
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use std::time::Instant;

    use chrono::TimeZone;
    use orbit_common::types::{EvidenceKind, LearningEvidence, LearningScope, LearningStatus};
    use tempfile::{TempDir, tempdir};

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
        }
    }

    fn store_with_index() -> (TempDir, LearningFileStore) {
        let dir = tempdir().expect("tempdir");
        let index = Store::open_in_memory().expect("open in-memory store");
        let store = LearningFileStore::new_with_index(dir.path().to_path_buf(), index);
        (dir, store)
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
        let path = dir.path().join(format!("{id}.yaml"));
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

    // ------ Acceptance criterion: layout assertions -----------------------

    #[test]
    fn layout_places_files_at_expected_paths_and_gitignore_is_respected() {
        let (dir, store) = store_with_index();
        let learning = store
            .create_learning(create_params("layout", vec![], vec![]))
            .expect("create");
        let active_path = dir.path().join(format!("{}.yaml", learning.id));
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
        let superseded_path = dir
            .path()
            .join("superseded")
            .join(format!("{}.yaml", learning.id));
        assert!(
            superseded_path.is_file(),
            "superseded file at {}",
            superseded_path.display()
        );
        assert!(!active_path.exists(), "active file must be moved out");

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
