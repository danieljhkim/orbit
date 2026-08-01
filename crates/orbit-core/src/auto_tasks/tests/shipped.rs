//! Embedded default auto-task tests [ORB-10549]. Defaults must parse through
//! the same schema as workspace definitions and remain inert until explicitly
//! enabled or manually minted.

use orbit_common::types::{AutoTaskSchedule, DedupePolicy, parse_auto_task_yaml};

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
    assert_eq!(definition.template.crew.as_deref(), Some("luna"));
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
}
