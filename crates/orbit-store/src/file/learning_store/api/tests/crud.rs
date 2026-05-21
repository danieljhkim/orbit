//! CRUD-focused tests for LearningFileStore (split from monolithic api.rs per ORB-00116).

use chrono::{TimeZone as _, Utc};
use orbit_common::types::{EvidenceKind, LearningEvidence, LearningScope, LearningStatus};
use tempfile::tempdir;

use super::super::store::LearningFileStore;
use super::test_support::{create_params, store_with_index};
use crate::Store;
use crate::backend::{LearningCreateParams, LearningSearchParams, LearningUpdateParams};

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
        let learning_dir = path_root.join(&learning.id);
        assert!(learning_dir.join("votes.jsonl").is_file());
        assert!(learning_dir.join("comments.jsonl").is_file());
        let allocation = store
            .id_allocator
            .learning_allocation(&learning.id)
            .expect("allocation")
            .expect("allocation exists");
        assert_eq!(
            allocation.worktree_root,
            std::fs::canonicalize(&path_root).expect("canonical learning root")
        );
        assert_eq!(
            allocation.body_path.as_deref(),
            Some(std::path::Path::new("L-0001/learning.yaml"))
        );
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

#[test]
fn forward_compat_fixture_with_symbols_and_semantic_seed_loads_and_round_trips() {
    let dir = tempdir().expect("tempdir");
    let id = "L-0009";
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

    assert_eq!(first.id, "L-0001");
    assert_eq!(second.id, "L-0002");
    assert_eq!(third.id, "L-0003");
}

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
    let gitignore_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.gitignore");
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
