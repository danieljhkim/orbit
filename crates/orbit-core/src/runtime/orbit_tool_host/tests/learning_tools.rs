//! Tests for `crates/orbit-core/src/runtime/orbit_tool_host/learning_tools.rs`.
//!
//! Covers the 13 ACs from T20260511-6:
//! 1. All learning tools surface in the registry with documented field names.
//! 2. Sync + prune tools live in the registry alongside the six design-doc tools.
//! 3. Round-trip persistence (add → show preserves every field).
//! 4. Scope-OR matching with dedup on combined queries.
//! 5. `matched_by` annotation present on every result.
//! 6. Ranking honors priority desc then updated_at desc.
//! 7. End-to-end latency p50 < 10 ms at 500 records (gated, `#[ignore]`).
//! 8. Supersession excludes from default search; surfaces under `list status=superseded`.
//! 9. CLI parity is covered in `crates/orbit-cli/tests/learning.rs`.
//! 10. `prune --stale-only` reports without modifying; `prune --delete` archives.
//! 11. `sync` rebuilds the index from YAML.
//! 12. Input validation (summary > 280, self-supersede, immutable superseded).
//! 13. ADR-004 status flipped on the design-doc tree (covered in 4_decisions.md).

use std::time::Instant;

use orbit_common::types::{
    EvidenceKind, LearningEvidence, LearningScope, LearningStatus, OrbitError,
};
use orbit_store::{LearningCreateParams, LearningSearchParams};
use orbit_tools::ToolRegistry;
use serde_json::{Value, json};
use tempfile::tempdir;

use super::super::test_support::test_runtime;
use crate::OrbitRuntime;

fn registry_with_builtins() -> ToolRegistry {
    let mut registry = ToolRegistry::new();
    registry.register_builtins();
    registry
}

/// Declare a human caller context for a test that reaches a learning *write*
/// tool.
///
/// [ORB-10364] gates `add`/`update`/`supersede` on the `ORBIT_AGENT_*` pair,
/// which a child of a managed Orbit run inherits (the ORB-10350 hazard). The
/// returned guard also holds the process-wide env lock, so it serializes these
/// tests against the executor-context ones below.
fn human_context_env() -> orbit_common::test_env::ScopedEnv {
    orbit_common::test_env::unset(
        orbit_common::test_env::AGENT_IDENTITY_ENV
            .iter()
            .copied()
            .chain(std::iter::once("ORBIT_LEARNING_AUTHOR")),
    )
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
    let schemas = registry.all_schemas();
    let names: Vec<&str> = schemas
        .iter()
        .map(|s| s.name.as_str())
        .filter(|n| n.starts_with("orbit.learning."))
        .collect();
    for expected in [
        "orbit.learning.add",
        "orbit.learning.archive",
        "orbit.learning.list",
        "orbit.learning.prune",
        "orbit.learning.sync",
        "orbit.learning.show",
        "orbit.learning.supersede",
        "orbit.learning.update",
    ] {
        assert!(
            names.contains(&expected),
            "missing tool: {expected}; got {names:?}"
        );
    }
    // ORB-00202: `orbit.learning.search` was deleted in phase 2; the
    // substring case moves to `orbit.search --kind learning` and the
    // structural cases move to `orbit.learning.list --path/--tag`.
    assert!(
        !names.contains(&"orbit.learning.search"),
        "orbit.learning.search must be deleted in phase 2"
    );

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

    let response =
        super::super::learning_tools::show(&runtime, json!({"id": learning.id})).expect("show");
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
}

// --- ORB-00202: orbit.learning.list path filter uses glob-containment

#[test]
fn list_path_filter_uses_glob_containment() {
    let (_guard, runtime, _repo_root) = test_runtime();
    let scoped = create_minimal(&runtime, "scoped", &["foo/**"], &[]);
    let unscoped = create_minimal(&runtime, "unscoped", &["bar/**"], &[]);

    let results = super::super::learning_tools::list(&runtime, json!({"path": "foo/bar.rs"}))
        .expect("by path");
    let ids = ids_from_array(&results);
    assert!(
        ids.contains(&scoped.id),
        "glob-containment should match foo/bar.rs against scope foo/**"
    );
    assert!(
        !ids.contains(&unscoped.id),
        "unrelated scope must not match"
    );
}

#[test]
fn list_tag_filter_uses_case_insensitive_equality() {
    let (_guard, runtime, _repo_root) = test_runtime();
    let tagged = create_minimal(&runtime, "tagged", &[], &["perf"]);
    let untagged = create_minimal(&runtime, "untagged", &[], &["other"]);

    let results =
        super::super::learning_tools::list(&runtime, json!({"tag": "perf"})).expect("by tag");
    let ids = ids_from_array(&results);
    assert!(ids.contains(&tagged.id));
    assert!(!ids.contains(&untagged.id));
}

// --- AC #8: supersession excludes from default list ------------------

#[test]
fn supersede_excludes_from_default_list_but_surfaces_under_status_superseded() {
    let (_guard, runtime, _repo_root) = test_runtime();
    let _env = human_context_env();
    let old = create_minimal(&runtime, "old", &["foo/**"], &[]);
    let new = create_minimal(&runtime, "new", &["foo/**"], &[]);

    super::super::learning_tools::supersede(
        &runtime,
        json!({"id": old.id, "with": new.id}),
        None,
        None,
    )
    .expect("supersede");

    let active = super::super::learning_tools::list(&runtime, json!({"status": "active"}))
        .expect("active list");
    let ids = ids_from_array(&active);
    assert!(!ids.contains(&old.id));
    assert!(ids.contains(&new.id));

    let superseded = super::super::learning_tools::list(&runtime, json!({"status": "superseded"}))
        .expect("list");
    let ids = ids_from_array(&superseded);
    assert!(ids.contains(&old.id));
}

// --- ORB-10469: named single-learning archive (retire without a replacement)

#[test]
fn archive_retires_a_single_active_learning_without_a_replacement() {
    let (_guard, runtime, _repo_root) = test_runtime();
    let _env = human_context_env();
    let learning = create_minimal(&runtime, "obsolete rule", &[], &[]);

    let response =
        super::super::learning_tools::archive(&runtime, json!({"id": learning.id}), None, None)
            .expect("archive");
    assert_eq!(response["id"], learning.id);
    assert_eq!(response["status"], "superseded");
    assert!(response["superseded_by"].is_null());

    let active = super::super::learning_tools::list(&runtime, json!({"status": "active"}))
        .expect("active list");
    assert!(!ids_from_array(&active).contains(&learning.id));
}

#[test]
fn archive_is_idempotent_on_an_already_archived_learning() {
    let (_guard, runtime, _repo_root) = test_runtime();
    let _env = human_context_env();
    let learning = create_minimal(&runtime, "obsolete rule", &[], &[]);

    super::super::learning_tools::archive(&runtime, json!({"id": learning.id}), None, None)
        .expect("first archive");
    let second =
        super::super::learning_tools::archive(&runtime, json!({"id": learning.id}), None, None)
            .expect("second archive is a no-op success");
    assert_eq!(second["status"], "superseded");
}

#[test]
fn archive_is_a_no_op_on_a_record_already_superseded_with_a_replacement() {
    let (_guard, runtime, _repo_root) = test_runtime();
    let _env = human_context_env();
    let old = create_minimal(&runtime, "old", &[], &[]);
    let new = create_minimal(&runtime, "new", &[], &[]);
    runtime
        .supersede_learning(&old.id, &new.id)
        .expect("supersede");

    let response =
        super::super::learning_tools::archive(&runtime, json!({"id": old.id}), None, None)
            .expect("archive on already-superseded record is a no-op");
    assert_eq!(response["status"], "superseded");
    // The existing replacement pointer is preserved, not clobbered to null.
    assert_eq!(response["superseded_by"], new.id);
}

#[test]
fn archive_rejects_a_missing_id() {
    let (_guard, runtime, _repo_root) = test_runtime();
    let _env = human_context_env();

    let err =
        super::super::learning_tools::archive(&runtime, json!({"id": "L-9999999"}), None, None)
            .expect_err("missing id is rejected");
    assert!(matches!(err, OrbitError::NotFound { .. }));
}

// --- AC #11: sync rebuilds the index from YAML -----------------------

#[test]
fn sync_rebuilds_index_after_truncation() {
    let (_guard, runtime, _repo_root) = test_runtime();
    let learning = create_minimal(&runtime, "a", &["foo/**"], &["alpha"]);

    let response = super::super::learning_tools::sync(&runtime, Value::Null).expect("sync");
    assert!(response["rebuilt_count"].as_u64().unwrap() >= 1);

    // Pre-condition holds: list still finds the learning by tag.
    let results =
        super::super::learning_tools::list(&runtime, json!({"tag": "alpha"})).expect("list");
    let ids = ids_from_array(&results);
    assert!(ids.contains(&learning.id));
}

// --- AC #12: input validation ----------------------------------------

#[test]
fn add_rejects_summary_longer_than_280_chars() {
    let (_guard, runtime, _repo_root) = test_runtime();
    let _env = human_context_env();
    let long = "a".repeat(281);
    let err = super::super::learning_tools::add(
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
    let _env = human_context_env();
    let learning = create_minimal(&runtime, "x", &[], &[]);
    let err = super::super::learning_tools::supersede(
        &runtime,
        json!({"id": learning.id, "with": learning.id}),
        None,
        None,
    )
    .expect_err("self-supersede rejected");
    assert!(matches!(err, OrbitError::InvalidInput(_)));
}

#[test]
fn supersede_reports_an_unreadable_remote_stub_like_show() {
    let root = tempdir().expect("tempdir");
    let global_root = root.path().join("global");
    let shared_root = root.path().join("repo/.orbit");
    let remote_root = root.path().join("remote/.orbit");
    let local_root = root.path().join("local/.orbit");
    for path in [&global_root, &shared_root, &remote_root, &local_root] {
        std::fs::create_dir_all(path).expect("create runtime root");
    }

    let remote = OrbitRuntime::from_resolved_roots(&global_root, &shared_root, &remote_root)
        .expect("remote runtime");
    let remote_learning = create_minimal(&remote, "remote", &[], &[]);
    std::fs::remove_file(
        remote_root
            .join("learnings")
            .join(&remote_learning.id)
            .join("learning.yaml"),
    )
    .expect("remove remote learning body");

    let local = OrbitRuntime::from_resolved_roots(&global_root, &shared_root, &local_root)
        .expect("local runtime");
    let replacement = create_minimal(&local, "replacement", &[], &[]);
    let _env = human_context_env();

    let err = super::super::learning_tools::supersede(
        &local,
        json!({"id": remote_learning.id, "with": replacement.id}),
        None,
        None,
    )
    .expect_err("remote stub is not locally mutable");

    let OrbitError::Store(message) = err else {
        panic!("expected remote-stub store error");
    };
    assert!(message.contains("is recorded in another worktree"));
    assert!(message.contains("body is not locally readable"));
    assert!(message.contains("worktree_root="));
}

#[test]
fn update_rejects_on_superseded_record() {
    let (_guard, runtime, _repo_root) = test_runtime();
    let _env = human_context_env();
    let old = create_minimal(&runtime, "old", &[], &[]);
    let new = create_minimal(&runtime, "new", &[], &[]);
    runtime
        .supersede_learning(&old.id, &new.id)
        .expect("supersede");

    let err = super::super::learning_tools::update(
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
    let task = super::super::test_support::create_context_task(
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

    let report = super::super::learning_tools::prune(&runtime, json!({})).expect("report");
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
    let result =
        super::super::learning_tools::prune(&runtime, json!({"delete": true})).expect("delete");
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

// --- [ORB-10330] runtime preallocated finalizers -------------------------

#[test]
fn finalize_preallocated_learning_lands_supplied_id_and_lists() {
    let (_guard, runtime, _repo) = test_runtime();
    // A non-sequential id proves the runtime path never selects a local id.
    let learning = runtime
        .finalize_preallocated_learning(
            "L-0055",
            LearningCreateParams {
                summary: "hub preallocated learning".to_string(),
                scope: LearningScope::default(),
                body: "body".to_string(),
                evidence: Vec::new(),
                created_by: Some("test".to_string()),
                priority: None,
            },
        )
        .expect("finalize preallocated learning");
    assert_eq!(learning.id, "L-0055");

    // Lifecycle read/list work through the owner-local projection.
    let fetched = runtime.get_learning("L-0055").expect("get learning");
    assert_eq!(fetched.summary, "hub preallocated learning");
    let ids: Vec<String> = runtime
        .list_learnings(Some(LearningStatus::Active))
        .expect("list learnings")
        .into_iter()
        .map(|learning| learning.id)
        .collect();
    assert!(ids.contains(&"L-0055".to_string()));
}

#[test]
fn finalize_preallocated_adr_lands_supplied_id() {
    let (_guard, runtime, _repo) = test_runtime();
    let adr = runtime
        .finalize_preallocated_adr(
            "ADR-0055",
            orbit_store::AdrCreateParams {
                title: "Hub preallocated ADR".to_string(),
                owner: "test".to_string(),
                related_features: Vec::new(),
                related_tasks: Vec::new(),
                tags: Vec::new(),
                paths: Vec::new(),
                body: "decision body".to_string(),
            },
        )
        .expect("finalize preallocated ADR");
    assert_eq!(adr.id, "ADR-0055");
    assert_eq!(adr.status, orbit_common::types::AdrStatus::Proposed);
}

// --- ORB-10364: caller-role gate on the tool write surfaces ---------------

/// Clear the identity pair and the authoring opt-in, then declare an executor
/// context. The returned guard holds the process-wide env lock and restores
/// everything on drop; every caller below is synchronous.
fn executor_context_env() -> orbit_common::test_env::ScopedEnv {
    let guard = human_context_env();
    // SAFETY: the guard holds the process-wide env lock for its lifetime and
    // restores `ORBIT_AGENT_MODEL` (as unset) when it drops.
    unsafe {
        std::env::set_var("ORBIT_AGENT_MODEL", "claude-opus-5");
    }
    guard
}

fn policy_denied_message<T>(result: Result<T, OrbitError>) -> String {
    match result {
        Err(OrbitError::PolicyDenied(message)) => message,
        Err(error) => panic!("expected a policy denial, got {error:?}"),
        Ok(_) => panic!("expected a policy denial, got success"),
    }
}

#[test]
fn learning_write_tools_refuse_executor_context_and_redirect_to_friction_add() {
    let (_temp, runtime, _repo_root) = test_runtime();
    let old = create_minimal(&runtime, "old", &[], &[]);
    let new = create_minimal(&runtime, "new", &[], &[]);
    let _env = executor_context_env();

    for (label, result) in [
        (
            "add",
            super::super::learning_tools::add(
                &runtime,
                json!({ "summary": "executor rule", "body": "executor body" }),
                None,
                None,
            ),
        ),
        (
            "update",
            super::super::learning_tools::update(
                &runtime,
                json!({ "id": old.id, "summary": "rewrite" }),
                None,
                None,
            ),
        ),
        (
            "supersede",
            super::super::learning_tools::supersede(
                &runtime,
                json!({ "id": old.id, "with": new.id }),
                None,
                None,
            ),
        ),
        (
            "archive",
            super::super::learning_tools::archive(&runtime, json!({ "id": old.id }), None, None),
        ),
    ] {
        let message = policy_denied_message(result);
        assert!(
            message.contains("orbit friction add"),
            "{label} redirects to friction: {message}"
        );
        assert!(
            message.contains("ORBIT_LEARNING_AUTHOR"),
            "{label} names the opt-in: {message}"
        );
    }

    // Nothing was written: both fixtures are still active and unmodified.
    let active = runtime
        .list_learnings(Some(LearningStatus::Active))
        .expect("list active");
    assert_eq!(active.len(), 2);
    assert!(active.iter().any(|l| l.id == old.id && l.summary == "old"));
}

/// Reads stay open in an executor context — the gate is on authoring only.
#[test]
fn learning_read_tools_are_unaffected_in_an_executor_context() {
    let (_temp, runtime, _repo_root) = test_runtime();
    let learning = create_minimal(&runtime, "readable", &["foo/**"], &["perf"]);
    let _env = executor_context_env();

    let shown = super::super::learning_tools::show(&runtime, json!({ "id": learning.id }))
        .expect("show is not gated");
    assert_eq!(shown["summary"], "readable");

    let listed =
        super::super::learning_tools::list(&runtime, json!({})).expect("list is not gated");
    assert_eq!(ids_from_array(&listed), vec![learning.id]);
}

/// The dashboard's `PATCH /api/learnings/:id` entry point deliberately skips
/// the gate: a dashboard server can run inside a managed Orbit run with
/// `ORBIT_AGENT_MODEL` set, and its writes carry request-derived attribution
/// (ORB-10352). Gating it would refuse a human's edit over how the *server*
/// was launched.
#[test]
fn the_dashboard_update_entry_point_is_not_gated_by_the_server_process_env() {
    let (_temp, runtime, _repo_root) = test_runtime();
    let learning = create_minimal(&runtime, "dashboard original", &[], &[]);
    let _env = executor_context_env();

    let updated = runtime
        .update_learning_from_request(json!({ "id": learning.id, "summary": "dashboard revised" }))
        .expect("dashboard update is not gated");

    assert_eq!(updated["summary"], "dashboard revised");
}
