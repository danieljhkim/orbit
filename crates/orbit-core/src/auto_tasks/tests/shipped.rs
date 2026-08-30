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
    let names: Vec<&str> = DEFAULT_AUTO_TASK_FILES
        .iter()
        .map(|(name, _)| *name)
        .collect();
    for required in [
        "ci-failure-remediation",
        "code-review-sweep",
        "friction-curation",
        "qa-sweep",
        "security-review",
    ] {
        assert!(
            names.contains(&required),
            "missing shipped default {required}"
        );
    }
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
fn release_prep_probe_stays_no_diff_and_keeps_canonical_task_non_dispatchable() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(".orbit/auto_tasks/release-prep.yaml");
    let yaml = std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("read {}: {error}", path.display()));
    let definition = parse_auto_task_yaml(&yaml).expect("parse release-prep");

    assert_eq!(definition.name, "release-prep");
    assert!(!definition.enabled, "definition must ship disabled");
    assert!(matches!(definition.dedupe, DedupePolicy::SkipIfOpen));
    assert_eq!(
        definition.template.status,
        orbit_types::task::TaskStatus::Backlog
    );
    assert!(
        definition
            .template
            .tags
            .iter()
            .any(|tag| tag == "no-diff-expected"),
        "probe must stay a successful no-diff outcome"
    );
    assert!(
        !definition
            .template
            .tags
            .iter()
            .any(|tag| tag == "release" || tag == "awaiting-release-approval"),
        "the probe itself is not the canonical release task"
    );

    let body = definition
        .template
        .description
        .to_lowercase()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    for required in [
        "no-diff-expected",
        "successful no-diff",
        "never enter a commit-required delivery tail",
        "status `proposed`",
        "awaiting-release-approval",
        "authorized bounded diff",
        "before any backlog or in-progress admission",
        "tag, publish, promotion, and merge",
        "do not dispatch",
    ] {
        assert!(
            body.contains(required),
            "release-prep template should retain '{required}'"
        );
    }

    let joined = definition
        .template
        .acceptance_criteria
        .iter()
        .map(|criterion| criterion.to_lowercase())
        .collect::<Vec<_>>()
        .join(" ");
    for required in [
        "successful no-diff outcome",
        "never enter a commit-required delivery tail",
        "awaiting-release-approval",
        "status proposed",
        "non-dispatchable",
        "authorized bounded diff",
        "before backlog or in-progress admission",
        "tag, publish, promotion, and merge remain unauthorized",
    ] {
        assert!(
            joined.contains(required),
            "release-prep criteria should retain '{required}'"
        );
    }
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
    assert!(!definition.enabled, "definition must ship disabled");
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
    assert_eq!(
        definition.schedule,
        AutoTaskSchedule::Cron {
            cron: "50 * * * *".to_string()
        },
        "qa-sweep must keep its documented hourly schedule"
    );
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
    assert_eq!(
        definition.template.task_type,
        orbit_types::task::TaskType::Chore
    );
    assert_eq!(
        definition.template.priority,
        orbit_types::task::TaskPriority::Medium
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
        "documented setup",
        "writable temporary",
        "configured task or issue surface",
        "skip duplicates",
        "failing test",
        "standard validation command",
        "must be filed as a durable issue",
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
    let yaml_lower = yaml.to_lowercase();
    for orbit_specific in [
        "orbit init",
        "workspace init",
        "--root",
        "~/.orbit",
        "orbit mcp",
        "orbit tool run",
        "filed as an orbit task",
        "filed as orbit tasks",
        "tag it `qa-sweep`",
    ] {
        assert!(
            !yaml_lower.contains(orbit_specific),
            "qa-sweep instructions must stay product-agnostic; found '{orbit_specific}'"
        );
    }
    assert!(
        definition
            .template
            .acceptance_criteria
            .iter()
            .any(|criterion| {
                let criterion = criterion.to_lowercase();
                criterion.contains("configured task or issue surface")
                    && criterion.contains("evidence")
                    && criterion.contains("reproduction")
            }),
        "qa-sweep acceptance criteria must require durable reporting on the workspace issue surface"
    );
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
                    && !criterion.contains("orbit task")
            }),
        "qa-sweep acceptance criteria must require filing breaking tests"
    );
}

/// Code review sweep carries its window cursor in execution summaries rather
/// than in scheduler state, so the template must keep saying so, and it must
/// stay generic across workspaces.
#[test]
fn code_review_sweep_default_is_portable_cursor_driven_and_inert() {
    let (_, yaml) = DEFAULT_AUTO_TASK_FILES
        .iter()
        .find(|(name, _)| *name == "code-review-sweep")
        .expect("code-review-sweep default");
    let definition = parse_auto_task_yaml(yaml).expect("parse code-review-sweep");

    assert_eq!(definition.name, "code-review-sweep");
    assert!(!definition.enabled, "definition must ship disabled");
    assert_eq!(
        definition.schedule,
        AutoTaskSchedule::Cron {
            cron: "40 */6 * * *".to_string()
        },
        "code-review-sweep must use a documented six-hourly schedule"
    );
    assert!(matches!(definition.dedupe, DedupePolicy::SkipIfOpen));
    assert_eq!(definition.template.crew.as_deref(), Some("system"));
    assert!(
        yaml.contains("\n  crew: system"),
        "default must name the portable system crew"
    );
    assert_eq!(
        definition.template.status,
        orbit_types::task::TaskStatus::Backlog
    );
    for required_tag in ["code-review-sweep", "no-diff-expected"] {
        assert!(
            definition
                .template
                .tags
                .iter()
                .any(|tag| tag == required_tag),
            "missing required tag {required_tag}"
        );
    }
    assert!(
        !yaml.contains("/home/") && !yaml.contains("/Users/"),
        "default must not contain a machine-specific path"
    );
    // The template ships to every workspace, so it must not name this
    // repository's branches or files.
    for repo_specific in ["agent-main", "ORB-", "CLAUDE.md", "make ci"] {
        assert!(
            !yaml.contains(repo_specific),
            "template must stay workspace-generic; found '{repo_specific}'"
        );
    }

    let body = definition.template.description.to_lowercase();
    for required in [
        "last-reviewed commit",
        "execution summary",
        "seeds the cursor",
        "verify every finding",
        "skip duplicates",
        "file:line",
        "no-op",
        "orbit tool run orbit.task.add",
        "orbit tool run orbit.task.list",
        "orbit tool run orbit.task.show",
        "orbit tool run orbit.search",
    ] {
        assert!(
            body.contains(required),
            "template should retain '{required}'"
        );
    }
    assert!(!body.contains("orbit task add"));
    assert!(!body.contains("orbit task list"));
    assert!(!body.contains("orbit task show"));
    assert!(
        definition
            .template
            .acceptance_criteria
            .iter()
            .any(|criterion| {
                let criterion = criterion.to_lowercase();
                criterion.contains("reviewed range")
                    && criterion.contains("last-reviewed commit")
                    && criterion.contains("execution summary")
            }),
        "code-review-sweep must require recording the window cursor"
    );
    assert!(
        definition
            .template
            .acceptance_criteria
            .iter()
            .any(|criterion| {
                let criterion = criterion.to_lowercase();
                criterion.contains("verified against live code")
                    && criterion.contains("non-duplicate")
                    && criterion.contains("file:line")
            }),
        "code-review-sweep must require verified, evidenced, non-duplicate findings"
    );
}

#[test]
fn security_review_default_is_portable_weekly_and_inert() {
    let (_, yaml) = DEFAULT_AUTO_TASK_FILES
        .iter()
        .find(|(name, _)| *name == "security-review")
        .expect("security-review default");
    let definition = parse_auto_task_yaml(yaml).expect("parse security-review");

    assert_eq!(definition.name, "security-review");
    assert!(!definition.enabled, "definition must ship disabled");
    assert_eq!(
        definition.schedule,
        AutoTaskSchedule::Cron {
            cron: "0 8 * * 1".to_string()
        },
        "security-review must use a documented weekly schedule"
    );
    assert!(matches!(definition.dedupe, DedupePolicy::SkipIfOpen));
    assert_eq!(definition.template.crew.as_deref(), Some("system"));
    assert!(
        yaml.contains("\n  crew: system"),
        "default must name the portable system crew"
    );
    assert_eq!(
        definition.template.status,
        orbit_types::task::TaskStatus::Backlog
    );
    assert!(
        definition
            .template
            .tags
            .iter()
            .any(|tag| tag == "security-review"),
        "minted tasks must carry the security-review tag"
    );
    assert!(
        !yaml.contains("/home/") && !yaml.contains("/Users/"),
        "default must not contain a machine-specific path"
    );

    let body = definition.template.description.to_lowercase();
    for required in [
        "application code",
        "dependencies",
        "secret handling",
        "configuration",
        "evidence",
        "skip duplicates",
        "severity",
        "impact",
        "narrative-only",
        "no findings",
        "no-op",
        "orbit tool run orbit.task.add",
        "orbit tool run orbit.search",
        "orbit tool run orbit.task.show",
        "orbit tool run orbit.task.list",
    ] {
        assert!(
            body.contains(required),
            "template should retain '{required}'"
        );
    }
    assert!(!body.contains("orbit task add"));
    assert!(!body.contains("orbit task list"));
    assert!(!body.contains("orbit task show"));
    assert!(
        definition
            .template
            .acceptance_criteria
            .iter()
            .any(|criterion| {
                let criterion = criterion.to_lowercase();
                criterion.contains("durable")
                    && criterion.contains("evidence")
                    && criterion.contains("severity")
                    && criterion.contains("impact")
                    && criterion.contains("narrative-only")
            }),
        "security-review acceptance criteria must require durable filed findings"
    );
    assert!(
        definition
            .template
            .acceptance_criteria
            .iter()
            .any(|criterion| criterion.to_lowercase().contains("no findings")
                && criterion.to_lowercase().contains("no-op")),
        "security-review acceptance criteria must treat a clean review as success"
    );
}

/// CI-failure remediation is the portable default. It keeps the evidence
/// contract while remaining disabled until an operator opts in.
#[test]
fn ci_failure_remediation_default_is_portable_hourly_and_inert() {
    let (_, yaml) = DEFAULT_AUTO_TASK_FILES
        .iter()
        .find(|(name, _)| *name == "ci-failure-remediation")
        .expect("ci-failure-remediation default");
    let definition = parse_auto_task_yaml(yaml).expect("parse ci-failure-remediation");

    assert_eq!(definition.name, "ci-failure-remediation");
    assert!(!definition.enabled, "definition must ship disabled");
    assert_eq!(
        definition.schedule,
        AutoTaskSchedule::Cron {
            cron: "15 * * * *".to_string()
        },
        "ci-failure-remediation must use a documented hourly schedule"
    );
    assert!(matches!(definition.dedupe, DedupePolicy::SkipIfOpen));
    assert_eq!(definition.template.crew.as_deref(), Some("system"));
    assert!(
        yaml.contains("\n  crew: system"),
        "default must name the portable system crew"
    );
    assert_eq!(
        definition.template.status,
        orbit_types::task::TaskStatus::Backlog
    );
    assert_eq!(
        definition.template.task_type,
        orbit_types::task::TaskType::Bug
    );
    for required_tag in ["ci-failure-remediation", "no-diff-expected"] {
        assert!(
            definition
                .template
                .tags
                .iter()
                .any(|tag| tag == required_tag),
            "missing required tag {required_tag}"
        );
    }
    assert!(
        !yaml.contains("/home/") && !yaml.contains("/Users/"),
        "default must not contain a machine-specific path"
    );
    // The template ships to every workspace, so it must not name this
    // repository's branches, gates, workflow steps, files, or run IDs.
    for repo_specific in [
        "agent-main",
        "make ci",
        "Create orbit task on CI failure",
        ".github/workflows",
        "30232696219",
        "orbit-store",
        "plugin_skill_symlinks",
        "ORB-",
        "CLAUDE.md",
    ] {
        assert!(
            !yaml.contains(repo_specific),
            "template must stay workspace-generic; found '{repo_specific}'"
        );
    }
    assert!(
        definition
            .description
            .to_lowercase()
            .contains("github-actions-shaped")
            || definition
                .description
                .to_lowercase()
                .contains("github-actions shaped"),
        "description must disclose the GitHub Actions shape to operators on other CI"
    );
    let description = definition.description.to_lowercase();
    assert!(
        description.contains("execution-lane precondition"),
        "description must state what the execution lane needs before an operator enables it"
    );
    assert!(
        description.contains("github cli") && description.contains("token"),
        "description must name both ways a lane can satisfy the precondition"
    );

    let body = definition.template.description.to_lowercase();
    for required in [
        "current head",
        "reported head sha",
        "checkout commit",
        "stale",
        "supersedes",
        "root cause",
        "weaken",
        "transient",
        "green",
        "rerun",
        "execution_summary",
        "no-diff",
        "pre-handoff",
        "orbit tool run orbit.search",
        "orbit tool run orbit.task.list",
        "orbit tool run orbit.task.show",
        // CI discovery is tool-mediated so it stays bounded, redacted, and
        // usable from an execution lane that holds no GitHub credentials of
        // its own.
        "orbit tool run github.auth.status",
        "orbit tool run github.run.list",
        "orbit tool run github.run.view",
        "orbit tool run github.run.logs",
        "orbit tool run github.pr.list",
        "capability_unavailable",
    ] {
        assert!(
            body.contains(required),
            "template should retain '{required}'"
        );
    }
    assert!(!body.contains("orbit task list"));
    assert!(!body.contains("orbit task show"));

    // The body must not reach around the tools. A bare vendor-CLI invocation
    // or a hand-rolled API call skips the output bound and the redaction, and
    // fails opaquely on a lane with no credentials.
    for forbidden in [
        "`gh ",
        "$(gh ",
        "gh run ",
        "gh pr ",
        "gh auth ",
        "gh api",
        "api.github.com",
        "curl ",
    ] {
        assert!(
            !definition.template.description.contains(forbidden),
            "template must route CI discovery through the github.* tools; found '{forbidden}'"
        );
    }

    // The silent failure this definition exists to avoid: an agent that could
    // not query CI at all, reporting a clean pipeline.
    assert!(
        body.contains("never report \"no current failures\""),
        "template must forbid reporting a clean result after a failed preflight"
    );
    assert!(
        definition
            .template
            .acceptance_criteria
            .iter()
            .any(|criterion| {
                let criterion = criterion.to_lowercase();
                criterion.contains("execution_summary")
                    && criterion.contains("green rerun")
                    && criterion.contains("root cause")
            }),
        "ci-failure-remediation must require the execution_summary mapping contract"
    );
    assert!(
        definition
            .template
            .acceptance_criteria
            .iter()
            .any(|criterion| {
                let criterion = criterion.to_lowercase();
                criterion.contains("no-current-failure") && criterion.contains("no-diff")
            }),
        "ci-failure-remediation must treat a successful no-diff outcome as success"
    );
    assert!(
        definition
            .template
            .acceptance_criteria
            .iter()
            .any(|criterion| {
                let criterion = criterion.to_lowercase();
                criterion.contains("github.auth.status")
                    && criterion.contains("capability-unavailable")
            }),
        "ci-failure-remediation must require the preflight and a distinguishable capability-unavailable outcome"
    );
}

/// This repository already has an enabled workspace-authored
/// `ci-failure-remediation` definition. Seeding the inert default must treat
/// that file as authored and leave it byte-for-byte, the way workspace init
/// preserves an operator-authored `security-review`.
#[test]
fn seed_does_not_clobber_repository_authored_ci_failure_remediation() {
    let authored_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(".orbit/auto_tasks/ci-failure-remediation.yaml");
    let authored = std::fs::read_to_string(&authored_path)
        .unwrap_or_else(|error| panic!("read {}: {error}", authored_path.display()));
    let authored_definition =
        parse_auto_task_yaml(&authored).expect("repository-authored ci-failure-remediation parses");
    assert!(
        authored_definition.enabled,
        "this repository's definition is workspace-authored and enabled"
    );
    assert_ne!(
        authored_definition.template.crew.as_deref(),
        Some("system"),
        "this repository's definition must differ from the inert default so preservation is observable"
    );

    let temp = tempfile::tempdir().expect("tempdir");
    let orbit_dir = temp.path().join(".orbit");
    let dest = crate::auto_tasks::definition_path(&orbit_dir, "ci-failure-remediation");
    std::fs::create_dir_all(dest.parent().expect("auto_tasks parent"))
        .expect("create auto_tasks dir");
    std::fs::write(&dest, &authored).expect("install repository-authored definition");

    crate::auto_tasks::seed_default_auto_tasks(&orbit_dir)
        .expect("seed defaults into a workspace that already has the authored file");

    let preserved = std::fs::read_to_string(&dest).expect("read after seed");
    assert_eq!(
        preserved, authored,
        "seed/re-init must treat the repository-authored definition as authored and leave it byte-for-byte"
    );
    let preserved_definition =
        parse_auto_task_yaml(&preserved).expect("preserved definition still parses");
    assert!(preserved_definition.enabled);
    assert_eq!(
        preserved_definition.template.crew.as_deref(),
        authored_definition.template.crew.as_deref()
    );
}
