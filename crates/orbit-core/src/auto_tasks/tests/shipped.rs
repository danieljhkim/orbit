//! Shipped-definition tests [ORB-10318]: every git-tracked auto-task
//! definition under the repo's `.orbit/auto_tasks/` must parse and validate
//! fail-closed, and the artifact-deprecation review definition must stay
//! report-only. This is the "mechanism covered by tests" guard for the
//! definitions that travel with the repo — a malformed definition would
//! otherwise only surface at scheduler-fire time.

use std::path::PathBuf;

use orbit_common::types::{AutoTaskSchedule, parse_auto_task_yaml};

/// The repo's git-tracked auto-task definition directory, resolved relative to
/// this crate's manifest (`crates/orbit-core` → repo root → `.orbit/auto_tasks`).
fn shipped_auto_tasks_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(".orbit/auto_tasks")
}

/// Every shipped `.yaml` definition parses and validates. Fail-closed parsing
/// means any malformed record is an error here, not a silent skip at fire time.
#[test]
fn shipped_definitions_all_parse() {
    let dir = shipped_auto_tasks_dir();
    let entries =
        std::fs::read_dir(&dir).unwrap_or_else(|error| panic!("read {}: {error}", dir.display()));
    let mut count = 0usize;
    for entry in entries {
        let path = entry.expect("dir entry").path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("yaml") {
            continue;
        }
        let yaml = std::fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("read {}: {error}", path.display()));
        let definition = parse_auto_task_yaml(&yaml)
            .unwrap_or_else(|error| panic!("parse {}: {error}", path.display()));
        // The file stem is the definition's identity.
        let stem = path.file_stem().and_then(|s| s.to_str()).expect("stem");
        assert_eq!(
            definition.name, stem,
            "name must match file stem for {stem}"
        );
        count += 1;
    }
    assert!(
        count > 0,
        "expected at least one shipped auto-task definition"
    );
}

/// The artifact-deprecation review definition ships enabled, on a cron cadence,
/// and stays report-only: it carries the `no-diff-expected` and
/// `artifact-deprecation` tags and its template never asks to mutate learnings.
#[test]
fn artifact_deprecation_review_is_report_only() {
    let path = shipped_auto_tasks_dir().join("artifact-deprecation-review.yaml");
    let yaml = std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("read {}: {error}", path.display()));
    let definition = parse_auto_task_yaml(&yaml).expect("parse artifact-deprecation-review");

    assert_eq!(definition.name, "artifact-deprecation-review");
    assert!(definition.enabled, "definition must ship enabled");
    assert!(
        matches!(definition.schedule, AutoTaskSchedule::Cron { .. }),
        "deprecation review runs on a cron cadence"
    );

    let tags = &definition.template.tags;
    assert!(
        tags.iter().any(|t| t == "no-diff-expected"),
        "report-only run must be tagged no-diff-expected"
    );
    assert!(
        tags.iter().any(|t| t == "artifact-deprecation"),
        "definition must be tagged artifact-deprecation"
    );

    // Report-only: the prompt must not direct the agent to mutate learnings.
    let body = definition.template.description.to_lowercase();
    assert!(
        body.contains("report-only") || body.contains("report only"),
        "template must state the run is report-only"
    );
    for forbidden in ["execution_summary", "learning stats", "fail open"] {
        assert!(
            body.contains(forbidden),
            "template should reference '{forbidden}'"
        );
    }
}
