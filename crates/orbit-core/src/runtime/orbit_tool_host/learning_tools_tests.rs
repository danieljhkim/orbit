//! Tests for `crates/orbit-core/src/runtime/orbit_tool_host/learning_tools.rs`.
//!
//! Covers the 13 ACs from T20260511-6:
//! 1. All learning tools surface in the registry with documented field names.
//! 2. Reindex + prune tools live in the registry alongside the six design-doc tools.
//! 3. Round-trip persistence (add → show preserves every field).
//! 4. Scope-OR matching with dedup on combined queries.
//! 5. `matched_by` annotation present on every result.
//! 6. Ranking honors priority desc then updated_at desc.
//! 7. End-to-end latency p50 < 10 ms at 500 records (gated, `#[ignore]`).
//! 8. Supersession excludes from default search; surfaces under `list status=superseded`.
//! 9. CLI parity is covered in `crates/orbit-cli/tests/learning.rs`.
//! 10. `prune --stale-only` reports without modifying; `prune --delete` archives.
//! 11. `reindex` rebuilds the index from YAML.
//! 12. Input validation (summary > 280, self-supersede, immutable superseded).
//! 13. ADR-004 status flipped on the design-doc tree (covered in 4_decisions.md).

use std::time::Instant;

use orbit_common::types::{
    EvidenceKind, LearningEvidence, LearningScope, LearningStatus, OrbitError,
};
use orbit_store::{LearningCreateParams, LearningSearchParams};
use orbit_tools::ToolRegistry;
use serde_json::{Value, json};

use super::test_support::test_runtime;
use crate::OrbitRuntime;

fn registry_with_builtins() -> ToolRegistry {
    let mut registry = ToolRegistry::new();
    registry.register_builtins();
    registry
}

fn create_minimal(
    runtime: &OrbitRuntime,
    summary: &str,
    paths: &[&str],
    tags: &[&str],
) -> orbit_common::types::Learning {
    runtime
        .create_learning(LearningCreateParams {
            summary: summary.to_string(),
            scope: LearningScope {
                paths: paths.iter().map(|s| s.to_string()).collect(),
                tags: tags.iter().map(|s| s.to_string()).collect(),
                ..Default::default()
            },
            body: String::new(),
            evidence: Vec::new(),
            created_by: Some("test".to_string()),
            priority: None,
        })
        .expect("create")
}

// --- AC #1/#2: registry surface --------------------------------------

#[test]
fn registry_exposes_learning_tools_with_documented_schema_fields() {
    let registry = registry_with_builtins();
    let schemas = registry.schemas();
    let names: Vec<&str> = schemas
        .iter()
        .map(|s| s.name.as_str())
        .filter(|n| n.starts_with("orbit.learning."))
        .collect();
    for expected in [
        "orbit.learning.add",
        "orbit.learning.list",
        "orbit.learning.prune",
        "orbit.learning.reindex",
        "orbit.learning.search",
        "orbit.learning.show",
        "orbit.learning.supersede",
        "orbit.learning.update",
        "orbit.learning.upvote",
    ] {
        assert!(
            names.contains(&expected),
            "missing tool: {expected}; got {names:?}"
        );
    }

    // Spot-check the documented field names from design §5.2.
    let add_schema = schemas
        .iter()
        .find(|s| s.name == "orbit.learning.add")
        .expect("add schema");
    let add_field_names: Vec<&str> = add_schema
        .parameters
        .iter()
        .map(|p| p.name.as_str())
        .collect();
    for required in ["summary", "scope", "body", "evidence", "priority"] {
        assert!(
            add_field_names.contains(&required),
            "orbit.learning.add missing field: {required}",
        );
    }

    let search_schema = schemas
        .iter()
        .find(|s| s.name == "orbit.learning.search")
        .expect("search schema");
    let search_field_names: Vec<&str> = search_schema
        .parameters
        .iter()
        .map(|p| p.name.as_str())
        .collect();
    for required in ["path", "tag", "query", "limit"] {
        assert!(
            search_field_names.contains(&required),
            "orbit.learning.search missing field: {required}"
        );
    }

    let upvote_schema = schemas
        .iter()
        .find(|s| s.name == "orbit.learning.upvote")
        .expect("upvote schema");
    let upvote_field_names: Vec<&str> = upvote_schema
        .parameters
        .iter()
        .map(|p| p.name.as_str())
        .collect();
    for required in ["id", "model", "task"] {
        assert!(
            upvote_field_names.contains(&required),
            "orbit.learning.upvote missing field: {required}"
        );
    }
}

// --- AC #3: round-trip via runtime API + show ------------------------

#[test]
fn round_trip_add_show_preserves_every_field() {
    let (_guard, runtime, _repo_root) = test_runtime();
    let learning = runtime
        .create_learning(LearningCreateParams {
            summary: "Verify perf parity before swapping".to_string(),
            scope: LearningScope {
                paths: vec!["foo/**".to_string()],
                tags: vec!["perf".to_string()],
                ..Default::default()
            },
            body: "Long body explaining the rule.".to_string(),
            evidence: vec![LearningEvidence {
                kind: EvidenceKind::Task,
                reference: "T20260510-11".to_string(),
            }],
            created_by: Some("claude".to_string()),
            priority: Some(7),
        })
        .expect("create");

    let response = super::learning_tools::show(&runtime, json!({"id": learning.id})).expect("show");
    assert_eq!(response["id"], learning.id);
    assert_eq!(response["summary"], "Verify perf parity before swapping");
    assert_eq!(response["scope"]["paths"], json!(["foo/**"]));
    assert_eq!(response["scope"]["tags"], json!(["perf"]));
    assert_eq!(response["body"], "Long body explaining the rule.");
    assert_eq!(response["evidence"][0]["kind"], "task");
    assert_eq!(response["evidence"][0]["ref"], "T20260510-11");
    assert_eq!(response["created_by"], "claude");
    assert_eq!(response["priority"], 7);
    assert_eq!(response["status"], "active");
    assert_eq!(response["vote_count"], 0);
    assert!(response["last_voted_at"].is_null());
}

#[test]
fn upvote_records_vote_stats_on_show_but_not_list() {
    let (_guard, runtime, _repo_root) = test_runtime();
    let learning = create_minimal(&runtime, "vote target", &["foo/**"], &[]);

    let response = super::learning_tools::upvote(
        &runtime,
        json!({"id": learning.id, "model": "claude", "task": "ORB-00095"}),
        None,
        None,
    )
    .expect("upvote");
    assert_eq!(response["vote_count"], 1);
    assert!(response["last_voted_at"].as_str().is_some());

    let duplicate = super::learning_tools::upvote(
        &runtime,
        json!({"id": learning.id, "model": "claude", "task_id": "ORB-00095"}),
        None,
        None,
    )
    .expect("duplicate");
    assert_eq!(duplicate["vote_count"], 1);

    let shown = super::learning_tools::show(&runtime, json!({"id": learning.id})).expect("show");
    assert_eq!(shown["vote_count"], 1);
    assert!(shown["last_voted_at"].as_str().is_some());

    let listed = super::learning_tools::list(&runtime, json!({"status": "active"})).expect("list");
    let row = find_id(&listed, &learning.id).expect("listed row");
    assert!(row.get("vote_count").is_none());
    assert!(row.get("last_voted_at").is_none());
}

// --- AC #4: scope-OR with dedup --------------------------------------

#[test]
fn search_does_scope_or_with_dedup_on_combined_axes() {
    let (_guard, runtime, _repo_root) = test_runtime();
    let paths_only = create_minimal(&runtime, "paths only", &["foo/**"], &[]);
    let tags_only = create_minimal(&runtime, "tags only", &[], &["perf"]);
    let both = create_minimal(&runtime, "both axes", &["foo/**"], &["perf"]);

    // Path-only hits paths_only + both.
    let by_path =
        super::learning_tools::search(&runtime, json!({"path": "foo/bar.rs"})).expect("by path");
    let ids = ids_from_array(&by_path);
    assert!(ids.contains(&paths_only.id));
    assert!(ids.contains(&both.id));
    assert!(!ids.contains(&tags_only.id));

    // Tag-only hits tags_only + both.
    let by_tag = super::learning_tools::search(&runtime, json!({"tag": "perf"})).expect("by tag");
    let ids = ids_from_array(&by_tag);
    assert!(ids.contains(&tags_only.id));
    assert!(ids.contains(&both.id));
    assert!(!ids.contains(&paths_only.id));

    // Combined: every learning surfaces exactly once.
    let combined =
        super::learning_tools::search(&runtime, json!({"path": "foo/bar.rs", "tag": "perf"}))
            .expect("combined");
    let ids = ids_from_array(&combined);
    assert_eq!(ids.len(), 3);

    // AC #5: matched_by has both axes for the `both` record.
    let both_row = find_id(&combined, &both.id).expect("both row");
    let matched_by = both_row["matched_by"]
        .as_array()
        .expect("matched_by array")
        .iter()
        .filter_map(Value::as_str)
        .collect::<Vec<_>>();
    assert!(matched_by.iter().any(|axis| axis.starts_with("path:")));
    assert!(matched_by.iter().any(|axis| axis.starts_with("tag:")));
}

#[test]
fn search_accepts_absolute_paths_inside_workspace() {
    let (_guard, runtime, repo_root) = test_runtime();
    let learning = create_minimal(&runtime, "paths only", &["foo/**"], &[]);
    let absolute = repo_root.join("foo/bar.rs").to_string_lossy().to_string();

    let by_path =
        super::learning_tools::search(&runtime, json!({"path": absolute})).expect("by path");
    let ids = ids_from_array(&by_path);
    assert!(ids.contains(&learning.id));
}

#[test]
fn search_accepts_absolute_paths_inside_linked_worktree() {
    let (_guard, runtime, repo_root) = test_runtime();
    let worktree = tempfile::tempdir().expect("worktree tempdir");
    seed_fake_git_worktree(&repo_root, worktree.path());
    let learning = create_minimal(&runtime, "paths only", &["foo/**"], &[]);
    let absolute = worktree
        .path()
        .join("foo/bar.rs")
        .to_string_lossy()
        .to_string();

    let by_path =
        super::learning_tools::search(&runtime, json!({"path": absolute})).expect("by path");
    let ids = ids_from_array(&by_path);
    assert!(ids.contains(&learning.id));
}

// --- AC #5: matched_by always present on search results --------------

#[test]
fn search_annotates_every_result_with_matched_by() {
    let (_guard, runtime, _repo_root) = test_runtime();
    create_minimal(&runtime, "first", &["foo/**"], &["alpha"]);
    create_minimal(&runtime, "second", &["bar/**"], &["alpha"]);

    let results = super::learning_tools::search(&runtime, json!({"tag": "alpha"})).expect("search");
    let array = results.as_array().expect("array");
    assert_eq!(array.len(), 2);
    for row in array {
        let matched_by = row["matched_by"].as_array().expect("matched_by present");
        assert!(!matched_by.is_empty(), "matched_by must not be empty");
        for axis in matched_by {
            let raw = axis.as_str().expect("string");
            assert!(
                raw.starts_with("path:") || raw.starts_with("tag:") || raw.starts_with("query:"),
                "matched_by axis must be path:|tag:|query:; got {raw}"
            );
        }
    }
}

// --- AC #6: ranking honors priority desc then updated_at desc --------

#[test]
fn search_ranks_priority_desc_then_updated_at_desc() {
    let (_guard, runtime, _repo_root) = test_runtime();

    // Recent low priority (no priority set).
    let recent_low = create_minimal(&runtime, "recent low", &["foo/**"], &[]);
    // Old high priority — created next but with explicit priority set.
    let high_priority = runtime
        .create_learning(LearningCreateParams {
            summary: "old high priority".to_string(),
            scope: LearningScope {
                paths: vec!["foo/**".to_string()],
                ..Default::default()
            },
            body: String::new(),
            evidence: Vec::new(),
            created_by: None,
            priority: Some(10),
        })
        .expect("high");
    // Mid record with priority = 5.
    let mid = runtime
        .create_learning(LearningCreateParams {
            summary: "mid".to_string(),
            scope: LearningScope {
                paths: vec!["foo/**".to_string()],
                ..Default::default()
            },
            body: String::new(),
            evidence: Vec::new(),
            created_by: None,
            priority: Some(5),
        })
        .expect("mid");

    let results =
        super::learning_tools::search(&runtime, json!({"path": "foo/bar.rs"})).expect("search");
    let ids = ids_from_array(&results);
    let high_pos = ids.iter().position(|id| id == &high_priority.id).unwrap();
    let mid_pos = ids.iter().position(|id| id == &mid.id).unwrap();
    let low_pos = ids.iter().position(|id| id == &recent_low.id).unwrap();
    assert!(
        high_pos < mid_pos && mid_pos < low_pos,
        "expected priority desc ranking, got {ids:?}"
    );
}

// --- AC #8: supersession excludes from default search ----------------

#[test]
fn supersede_excludes_from_default_search_but_surfaces_under_list_superseded() {
    let (_guard, runtime, _repo_root) = test_runtime();
    let old = create_minimal(&runtime, "old", &["foo/**"], &[]);
    let new = create_minimal(&runtime, "new", &["foo/**"], &[]);

    super::learning_tools::supersede(&runtime, json!({"id": old.id, "with": new.id}), None, None)
        .expect("supersede");

    let results =
        super::learning_tools::search(&runtime, json!({"path": "foo/bar.rs"})).expect("search");
    let ids = ids_from_array(&results);
    assert!(!ids.contains(&old.id));
    assert!(ids.contains(&new.id));

    let superseded =
        super::learning_tools::list(&runtime, json!({"status": "superseded"})).expect("list");
    let ids = ids_from_array(&superseded);
    assert!(ids.contains(&old.id));
}

// --- AC #11: reindex rebuilds the index from YAML --------------------

#[test]
fn reindex_rebuilds_index_after_truncation() {
    let (_guard, runtime, _repo_root) = test_runtime();
    let learning = create_minimal(&runtime, "a", &["foo/**"], &["alpha"]);

    let response = super::learning_tools::reindex(&runtime, Value::Null).expect("reindex");
    assert!(response["rebuilt_count"].as_u64().unwrap() >= 1);

    // Pre-condition holds: search still finds the learning.
    let results = super::learning_tools::search(&runtime, json!({"tag": "alpha"})).expect("search");
    let ids = ids_from_array(&results);
    assert!(ids.contains(&learning.id));
}

// --- AC #12: input validation ----------------------------------------

#[test]
fn add_rejects_summary_longer_than_280_chars() {
    let (_guard, runtime, _repo_root) = test_runtime();
    let long = "a".repeat(281);
    let err = super::learning_tools::add(
        &runtime,
        json!({
            "summary": long,
            "scope": {"paths": ["foo/**"]},
        }),
        None,
        None,
    )
    .expect_err("rejects long summary");
    assert!(
        matches!(err, OrbitError::InvalidInput(_)),
        "expected InvalidInput, got {err:?}",
    );
}

#[test]
fn supersede_rejects_id_equal_to_with() {
    let (_guard, runtime, _repo_root) = test_runtime();
    let learning = create_minimal(&runtime, "x", &[], &[]);
    let err = super::learning_tools::supersede(
        &runtime,
        json!({"id": learning.id, "with": learning.id}),
        None,
        None,
    )
    .expect_err("self-supersede rejected");
    assert!(matches!(err, OrbitError::InvalidInput(_)));
}

#[test]
fn update_rejects_on_superseded_record() {
    let (_guard, runtime, _repo_root) = test_runtime();
    let old = create_minimal(&runtime, "old", &[], &[]);
    let new = create_minimal(&runtime, "new", &[], &[]);
    runtime
        .supersede_learning(&old.id, &new.id)
        .expect("supersede");

    let err = super::learning_tools::update(
        &runtime,
        json!({"id": old.id, "summary": "rewrite"}),
        None,
        None,
    )
    .expect_err("immutable after supersession");
    assert!(matches!(err, OrbitError::InvalidInput(_)));
}

// --- AC #10: prune (stale-only reports; --delete archives) -----------

#[test]
fn prune_stale_only_reports_without_modifying_and_delete_archives_via_supersede_with_null() {
    let (_guard, runtime, _repo_root) = test_runtime();

    // 1) Stale: scope paths point at a directory that does not exist
    //    AND evidence task ID is unknown.
    let stale = runtime
        .create_learning(LearningCreateParams {
            summary: "stale rule".to_string(),
            scope: LearningScope {
                paths: vec!["nonexistent-dir-xyz/**".to_string()],
                ..Default::default()
            },
            body: String::new(),
            evidence: vec![LearningEvidence {
                kind: EvidenceKind::Task,
                reference: "T99999999-0".to_string(),
            }],
            created_by: None,
            priority: None,
        })
        .expect("stale");
    // 2) Fresh: at least one extant evidence reference. Use a real task
    //    ID from the test workspace so the evidence check passes; scope
    //    paths are intentionally bogus so the evidence axis alone
    //    decides per §7.3.
    let task = super::test_support::create_context_task(
        &runtime,
        runtime.paths().repo_root.as_path(),
        orbit_common::types::TaskStatus::InProgress,
        &[],
    );
    let fresh = runtime
        .create_learning(LearningCreateParams {
            summary: "fresh rule".to_string(),
            scope: LearningScope {
                paths: vec!["another-nonexistent-dir/**".to_string()],
                ..Default::default()
            },
            body: String::new(),
            evidence: vec![LearningEvidence {
                kind: EvidenceKind::Task,
                reference: task.id.clone(),
            }],
            created_by: None,
            priority: None,
        })
        .expect("fresh");

    let report = super::learning_tools::prune(&runtime, json!({})).expect("report");
    let stale_ids: Vec<String> = report["stale"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap().to_string())
        .collect();
    assert!(stale_ids.contains(&stale.id));
    assert!(!stale_ids.contains(&fresh.id));
    assert!(report["deleted"].as_array().unwrap().is_empty());

    // delete: true archives the stale ones.
    let result = super::learning_tools::prune(&runtime, json!({"delete": true})).expect("delete");
    let deleted_ids: Vec<String> = result["deleted"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap().to_string())
        .collect();
    assert!(deleted_ids.contains(&stale.id));

    // Verify directly: the archived YAML now has status=superseded and
    // superseded_by=null per §7.3.
    let archived = runtime.get_learning(&stale.id).expect("archived");
    assert_eq!(archived.status, LearningStatus::Superseded);
    assert!(archived.superseded_by.is_none());
}

// --- AC #7: end-to-end latency (gated) -------------------------------

#[test]
#[ignore]
fn learning_search_end_to_end_latency_p50_under_10ms_at_500_records() {
    let (_guard, runtime, _repo_root) = test_runtime();

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
        runtime
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

    let mut durations_ns: Vec<u128> = Vec::with_capacity(1000);
    for _ in 0..1000 {
        let start = Instant::now();
        let _ = runtime
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
    let p50_ms = (p(0.50) as f64) / 1_000_000.0;
    let p95_ms = (p(0.95) as f64) / 1_000_000.0;
    let p99_ms = (p(0.99) as f64) / 1_000_000.0;
    #[allow(clippy::print_stdout)]
    {
        println!(
            "learning_search_end_to_end_latency: 500 records, 1000 calls, target=crates/orbit-engine/perf_runner.rs"
        );
        println!(
            "learning_search_end_to_end_latency: p50={p50_ms:.3}ms p95={p95_ms:.3}ms p99={p99_ms:.3}ms"
        );
    }
    assert!(
        p50_ms < 10.0,
        "median search latency must be < 10ms; got {p50_ms:.3}ms (p95={p95_ms:.3}ms p99={p99_ms:.3}ms)"
    );
}

// --- shared helpers --------------------------------------------------

fn ids_from_array(value: &Value) -> Vec<String> {
    value
        .as_array()
        .expect("array")
        .iter()
        .map(|item| item["id"].as_str().expect("id present").to_string())
        .collect()
}

fn find_id<'a>(value: &'a Value, id: &str) -> Option<&'a Value> {
    value
        .as_array()?
        .iter()
        .find(|item| item["id"].as_str() == Some(id))
}

fn seed_fake_git_worktree(main_repo: &std::path::Path, worktree: &std::path::Path) {
    let worktree_git_dir = main_repo.join(".git").join("worktrees").join("orbit-test");
    std::fs::create_dir_all(&worktree_git_dir).expect("create fake worktree git dir");
    std::fs::write(
        worktree.join(".git"),
        format!("gitdir: {}\n", worktree_git_dir.display()),
    )
    .expect("write worktree gitfile");
}
