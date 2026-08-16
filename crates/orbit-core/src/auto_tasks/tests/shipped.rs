//! Embedded default auto-task tests [ORB-10549]. Defaults must parse through
//! the same schema as workspace definitions and remain inert until explicitly
//! enabled or manually minted.

use std::path::PathBuf;

use orbit_common::protocol::yaml::parse_auto_task_yaml;
use orbit_types::workflow::{AutoTaskSchedule, DedupePolicy};

use crate::auto_tasks::DEFAULT_AUTO_TASK_FILES;

/// Every embedded default parses, uses its filename identity, and remains
/// disabled. An enabled default would turn workspace initialization into an
/// implicit scheduler opt-in, so make that regression deterministic here.
#[test]
fn shipped_defaults_all_parse_and_are_disabled() {
    assert!(
        !DEFAULT_AUTO_TASK_FILES.is_empty(),
        "expected at least one shipped auto-task definition"
    );
    for (stem, yaml) in DEFAULT_AUTO_TASK_FILES {
        let definition =
            parse_auto_task_yaml(yaml).unwrap_or_else(|error| panic!("parse {stem}: {error}"));
        assert_eq!(
            definition.name, *stem,
            "name must match file stem for {stem}"
        );
        assert!(
            !definition.enabled,
            "default auto-task {stem} must ship disabled"
        );
    }
}

/// Every repository-local definition remains covered in addition to the
/// embedded defaults. These files are workspace-authored and may intentionally
/// differ from the inert defaults.
#[test]
fn repository_definitions_all_parse() {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(".orbit/auto_tasks");
    let entries =
        std::fs::read_dir(&dir).unwrap_or_else(|error| panic!("read {}: {error}", dir.display()));
    let mut count = 0usize;
    for entry in entries {
        let path = entry.expect("directory entry").path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("yaml") {
            continue;
        }
        let yaml = std::fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("read {}: {error}", path.display()));
        let definition = parse_auto_task_yaml(&yaml)
            .unwrap_or_else(|error| panic!("parse {}: {error}", path.display()));
        let stem = path
            .file_stem()
            .and_then(|value| value.to_str())
            .expect("file stem");
        assert_eq!(
            definition.name, stem,
            "name must match file stem for {stem}"
        );
        count += 1;
    }
    assert!(
        count > 0,
        "expected at least one repository-local auto-task"
    );
}

#[test]
fn model_price_audit_is_weekly_report_only_and_routes_to_terra() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(".orbit/auto_tasks/model-price-audit.yaml");
    let yaml = std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("read {}: {error}", path.display()));
    let definition = parse_auto_task_yaml(&yaml).expect("parse model-price-audit");

    assert_eq!(definition.name, "model-price-audit");
    assert!(definition.enabled, "definition must be enabled");
    assert_eq!(
        definition.schedule,
        AutoTaskSchedule::Cron {
            cron: "0 6 * * 1".to_string()
        }
    );
    assert!(matches!(definition.dedupe, DedupePolicy::SkipIfOpen));
    assert_eq!(definition.template.crew.as_deref(), Some("terra"));
    assert_eq!(
        definition.template.status,
        orbit_types::task::TaskStatus::Backlog
    );
    for required_tag in ["model-price-audit", "pricing", "no-diff-expected"] {
        assert!(
            definition
                .template
                .tags
                .iter()
                .any(|tag| tag == required_tag),
            "missing required tag {required_tag}"
        );
    }

    let body = definition.template.description.to_lowercase();
    for required in [
        "invocationrecord",
        "authoritative",
        "source url",
        "retrieval timestamp",
        "at most one",
        "historical rows",
        "non-overlapping",
        "short-context",
        "fast/service-tier",
        "long-context",
        "dry-run",
        "human review",
        "orchestration-session cost",
    ] {
        assert!(
            body.contains(required),
            "template should retain '{required}'"
        );
    }
    assert!(body.contains("must not edit model_prices.yaml"));
}

/// Friction curation is the portable default. It keeps the curation safeguards
/// while remaining disabled until an operator opts in.
#[test]
fn friction_curation_default_is_portable_and_inert() {
    let (_, yaml) = DEFAULT_AUTO_TASK_FILES
        .iter()
        .find(|(name, _)| *name == "friction-curation")
        .expect("friction-curation default");
    let definition = parse_auto_task_yaml(yaml).expect("parse friction-curation");

    assert_eq!(definition.name, "friction-curation");
    assert!(!definition.enabled, "definition must ship disabled");
    assert!(
        matches!(definition.schedule, AutoTaskSchedule::Cron { .. }),
        "friction curation runs on a cron cadence"
    );
    assert!(matches!(definition.dedupe, DedupePolicy::SkipIfOpen));
    // [ORB-10877] `system` is a portable lane seeded for every detected family,
    // rather than a family-specific crew such as Luna or Sonnet.
    assert_eq!(definition.template.crew.as_deref(), Some("system"));
    assert!(
        yaml.contains("\n  crew: system"),
        "default must name the portable system crew"
    );
    assert!(
        !yaml.contains("/home/") && !yaml.contains("/Users/"),
        "default must not contain a machine-specific path"
    );

    let body = definition.template.description.to_lowercase();
    for required in [
        "rejected tasks",
        "terminal rejection",
        "administrative rejection",
        "exactly one",
        "fail open",
        "repeat pass",
    ] {
        assert!(
            body.contains(required),
            "template should retain '{required}'"
        );
    }

    assert!(body.contains("orbit tool run orbit.friction.list"));
    assert!(body.contains("orbit tool run orbit.friction.update"));
    assert!(body.contains("orbit tool run orbit.friction.resolve"));
    assert!(!body.contains("orbit friction list"));
    assert!(!body.contains("orbit friction update"));
    assert!(!body.contains("orbit friction resolve"));
}

#[test]
fn qa_sweep_default_preserves_hands_on_validation_contract() {
    let (_, yaml) = DEFAULT_AUTO_TASK_FILES
        .iter()
        .find(|(name, _)| *name == "qa-sweep")
        .expect("qa-sweep default");
    let definition = parse_auto_task_yaml(yaml).expect("parse qa-sweep");

    assert_eq!(definition.name, "qa-sweep");
    assert!(!definition.enabled);
    assert!(matches!(definition.schedule, AutoTaskSchedule::Cron { .. }));
    assert!(matches!(definition.dedupe, DedupePolicy::SkipIfOpen));
    // [ORB-10877] Same portable system-lane rule as friction-curation above.
    assert_eq!(definition.template.crew.as_deref(), Some("system"));
    assert!(
        yaml.contains("\n  crew: system"),
        "default must name the portable system crew"
    );
    assert_eq!(
        definition.template.status,
        orbit_types::task::TaskStatus::Backlog
    );
    assert!(definition.template.tags.iter().any(|tag| tag == "qa-sweep"));
    assert!(
        definition
            .template
            .tags
            .iter()
            .any(|tag| tag == "no-diff-expected")
    );
    assert!(!yaml.contains("/home/") && !yaml.contains("/Users/"));
    let body = definition.template.description.to_lowercase();
    for required in [
        "validate them hands-on",
        "exercise the affected",
        "skip duplicates",
        "failing test",
        "standard validation command",
        "must be filed as an orbit task",
        "environment-specific",
        "test-harness",
        "portability",
        "narrative-only",
        "validation impact",
        "production impact",
        "failing command",
        "exact error",
        "environment evidence",
        "scope assessment",
    ] {
        assert!(
            body.contains(required),
            "template should retain '{required}'"
        );
    }
    assert!(
        definition
            .template
            .acceptance_criteria
            .iter()
            .any(|criterion| {
                let criterion = criterion.to_lowercase();
                criterion.contains("failing test")
                    && criterion.contains("validation command")
                    && criterion.contains("validation impact")
                    && criterion.contains("production impact")
            }),
        "qa-sweep acceptance criteria must require filing breaking tests"
    );
}
