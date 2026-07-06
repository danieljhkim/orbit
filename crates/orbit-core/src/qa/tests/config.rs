//! `[qa]` config validation tests [ORB-10039]: the section is fail-closed —
//! a typo'd priority/status or an empty check list must be a config error,
//! never a sweep that silently validates nothing.

use std::time::Duration;

use orbit_common::types::{OrbitError, TaskPriority, TaskStatus};

use crate::config::RuntimeConfig;
use crate::qa::QaSweepConfig;

/// Parse a config.toml document through the exact production pipeline and
/// return its resolved `[qa]` section.
fn qa_from_toml(raw: &str) -> Result<QaSweepConfig, OrbitError> {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("config.toml");
    std::fs::write(&path, raw).expect("write config");
    let config = RuntimeConfig::load_layered(dir.path(), dir.path())?;
    Ok(config.qa_sweep().clone())
}

const FULL: &str = r#"
[qa]
default_priority = "high"
task_status = "proposed"

[[qa.workspace]]
name = "polaris"
branch = "agent-main"

[[qa.workspace.check]]
name = "frontmatter"
command = "python3 scripts/check_frontmatter.py"

[[qa.workspace.check]]
name = "lint"
command = "make lint"
mute = true
priority = "critical"
timeout_minutes = 5

[[qa.workspace]]
name = "bridge"

[[qa.workspace.check]]
name = "tests"
command = "pytest -q"
"#;

#[test]
fn full_section_parses_with_overrides_and_defaults() {
    let qa = qa_from_toml(FULL).expect("valid config");
    assert_eq!(qa.default_priority, TaskPriority::High);
    assert_eq!(qa.task_status, TaskStatus::Proposed);
    assert_eq!(qa.workspaces.len(), 2);

    let polaris = qa.workspace("polaris").expect("polaris entry");
    assert_eq!(polaris.branch.as_deref(), Some("agent-main"));
    assert_eq!(polaris.checks.len(), 2);
    let frontmatter = &polaris.checks[0];
    assert!(!frontmatter.mute);
    assert_eq!(frontmatter.priority, None);
    assert_eq!(frontmatter.timeout, Duration::from_secs(30 * 60));
    let lint = &polaris.checks[1];
    assert!(lint.mute);
    assert_eq!(lint.priority, Some(TaskPriority::Critical));
    assert_eq!(lint.timeout, Duration::from_secs(5 * 60));

    // Branch defaults to None (resolved from the registry at sweep time).
    assert_eq!(qa.workspace("bridge").expect("bridge entry").branch, None);
}

#[test]
fn absent_section_resolves_to_empty_defaults() {
    let qa = qa_from_toml("[workflow]\nbase_branch = \"agent-main\"\n").expect("valid config");
    assert_eq!(qa, QaSweepConfig::default());
    assert!(qa.workspaces.is_empty());
    assert_eq!(qa.default_priority, TaskPriority::Medium);
    // Backlog by default: design D4 wants the loop to close without a human
    // courier (ship-sweep can dispatch the fix).
    assert_eq!(qa.task_status, TaskStatus::Backlog);
}

fn assert_invalid(raw: &str, needle: &str) {
    match qa_from_toml(raw) {
        Err(OrbitError::InvalidInput(message)) => assert!(
            message.contains(needle),
            "expected error containing '{needle}', got: {message}"
        ),
        other => panic!("expected InvalidInput containing '{needle}', got {other:?}"),
    }
}

#[test]
fn invalid_priority_is_rejected() {
    assert_invalid(
        "[qa]\ndefault_priority = \"urgent\"\n",
        "qa.default_priority",
    );
}

#[test]
fn invalid_task_status_is_rejected() {
    assert_invalid("[qa]\ntask_status = \"done\"\n", "qa.task_status");
}

#[test]
fn workspace_without_checks_is_rejected() {
    assert_invalid(
        "[[qa.workspace]]\nname = \"polaris\"\n",
        "at least one [[qa.workspace.check]]",
    );
}

#[test]
fn check_without_command_is_rejected() {
    assert_invalid(
        "[[qa.workspace]]\nname = \"polaris\"\n[[qa.workspace.check]]\nname = \"lint\"\n",
        "command must be set",
    );
}

#[test]
fn duplicate_check_names_are_rejected() {
    assert_invalid(
        "[[qa.workspace]]\nname = \"polaris\"\n\
         [[qa.workspace.check]]\nname = \"lint\"\ncommand = \"a\"\n\
         [[qa.workspace.check]]\nname = \"lint\"\ncommand = \"b\"\n",
        "more than once",
    );
}

#[test]
fn duplicate_workspaces_are_rejected() {
    assert_invalid(
        "[[qa.workspace]]\nname = \"polaris\"\n\
         [[qa.workspace.check]]\nname = \"lint\"\ncommand = \"a\"\n\
         [[qa.workspace]]\nname = \"polaris\"\n\
         [[qa.workspace.check]]\nname = \"lint\"\ncommand = \"a\"\n",
        "more than once",
    );
}

#[test]
fn zero_timeout_is_rejected() {
    assert_invalid(
        "[[qa.workspace]]\nname = \"polaris\"\n\
         [[qa.workspace.check]]\nname = \"lint\"\ncommand = \"a\"\ntimeout_minutes = 0\n",
        "timeout_minutes",
    );
}
