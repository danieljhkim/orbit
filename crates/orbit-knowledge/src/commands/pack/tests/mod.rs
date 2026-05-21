#![allow(missing_docs)]

use std::fs;

use serde_json::Value;
use tempfile::{TempDir, tempdir};

use crate::commands::{GraphCommandContext, TaskGraphScope};
use crate::graph::object_store::RefName;
use crate::pipeline::context::BuildConfig;

use super::{PackInput, run};

const TEST_REF: &str = "pack-test";

struct PackFixture {
    repo: TempDir,
    _knowledge: TempDir,
    context: GraphCommandContext,
}

#[test]
fn unresolved_file_outside_indexed_roots_reports_reason_and_hint() {
    let fixture = PackFixture::new();
    fs::create_dir_all(fixture.repo.path().join(".claude")).expect("create hidden dir");
    fs::write(
        fixture.repo.path().join(".claude/settings.json"),
        "{\"permissions\":{}}\n",
    )
    .expect("write hidden config");

    let entry = pack_single_entry(&fixture.context, "file:.claude/settings.json");

    assert_eq!(entry["kind"], "unresolved");
    assert_eq!(entry["reason"], "outside_indexed_roots");
    assert_ne!(
        entry["hint"],
        "Run `orbit graph build` for an explicit refresh, or pass `refresh: true` when an inline refresh is acceptable."
    );
}

#[test]
fn unresolved_file_missing_on_disk_reports_not_found() {
    let fixture = PackFixture::new();

    let entry = pack_single_entry(&fixture.context, "file:src/missing.rs");

    assert_eq!(entry["kind"], "unresolved");
    assert_eq!(entry["reason"], "not_found");
}

#[test]
fn unresolved_file_present_under_indexed_root_reports_stale_snapshot() {
    let fixture = PackFixture::new();
    fs::write(fixture.repo.path().join("src/new.rs"), "pub fn new() {}\n")
        .expect("write new file after graph build");

    let entry = pack_single_entry(&fixture.context, "file:src/new.rs");

    assert_eq!(entry["kind"], "unresolved");
    assert_eq!(entry["reason"], "stale_snapshot");
    assert!(
        entry["hint"]
            .as_str()
            .expect("stale snapshot hint")
            .contains("orbit graph build --refresh")
    );
}

impl PackFixture {
    fn new() -> Self {
        let repo = tempdir().expect("repo tempdir");
        fs::create_dir_all(repo.path().join("src")).expect("create source dir");
        fs::write(repo.path().join("src/lib.rs"), "pub fn indexed() {}\n")
            .expect("write indexed source file");

        let knowledge = tempdir().expect("knowledge tempdir");
        crate::pipeline::run_build(BuildConfig {
            repo_path: repo.path().to_path_buf(),
            output_dir: knowledge.path().to_path_buf(),
            incremental: false,
            ref_name: Some(RefName::new(TEST_REF).expect("valid ref name")),
        })
        .expect("build graph fixture");

        let context = GraphCommandContext {
            knowledge_dir: knowledge.path().to_path_buf(),
            workspace_root: Some(repo.path().to_path_buf()),
            explicit_ref: Some(TEST_REF.to_string()),
            explicit_knowledge_dir: false,
            task_scope: TaskGraphScope::default(),
        };

        Self {
            repo,
            _knowledge: knowledge,
            context,
        }
    }
}

fn pack_single_entry(context: &GraphCommandContext, selector: &str) -> Value {
    let result = run(PackInput {
        context: context.clone(),
        selectors: vec![selector.to_string()],
        hydrate_leaf_source: false,
        refresh: false,
        selector_timeout_ms: 15_000,
    })
    .expect("pack selector");

    result
        .pack
        .get("entries")
        .and_then(Value::as_array)
        .and_then(|entries| entries.first())
        .cloned()
        .expect("single pack entry")
}
