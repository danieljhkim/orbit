//! `[qa]` config validation tests [ORB-10039, reworked ORB-10146]: the section
//! is fail-closed — a typo'd priority/status, a bad worker URL, or a leftover
//! legacy `[[qa.workspace.check]]` table must be a config error, never a sweep
//! that silently validates nothing.

use std::time::Duration;

use orbit_common::types::{OrbitError, TaskPriority, TaskStatus};

use crate::config::RuntimeConfig;
use crate::qa::QaSweepConfig;
use crate::qa::config::DEFAULT_WORKER_BASE_URL;

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
base_url = "http://127.0.0.1:9099/"

[[qa.workspace]]
name = "polaris"
branch = "agent-main"
crew = "opus"
timeout_minutes = 90
max_commits = 10

[[qa.workspace]]
name = "bridge"
"#;

#[test]
fn full_section_parses_with_overrides_and_defaults() {
    let qa = qa_from_toml(FULL).expect("valid config");
    assert_eq!(qa.default_priority, TaskPriority::High);
    assert_eq!(qa.task_status, TaskStatus::Proposed);
    // Trailing slash is trimmed.
    assert_eq!(qa.worker_base_url, "http://127.0.0.1:9099");
    assert_eq!(qa.workspaces.len(), 2);

    let polaris = qa.workspace("polaris").expect("polaris entry");
    assert_eq!(polaris.branch.as_deref(), Some("agent-main"));
    assert_eq!(polaris.crew.as_deref(), Some("opus"));
    assert_eq!(polaris.timeout, Duration::from_secs(90 * 60));
    assert_eq!(polaris.max_commits, Some(10));

    // Bridge: branch/crew default to None, timeout to the 120-minute default.
    let bridge = qa.workspace("bridge").expect("bridge entry");
    assert_eq!(bridge.branch, None);
    assert_eq!(bridge.crew, None);
    assert_eq!(bridge.timeout, Duration::from_secs(120 * 60));
    assert_eq!(bridge.max_commits, None);
}

#[test]
fn absent_section_resolves_to_empty_defaults() {
    let qa = qa_from_toml("[workflow]\nbase_branch = \"agent-main\"\n").expect("valid config");
    assert_eq!(qa, QaSweepConfig::default());
    assert!(qa.workspaces.is_empty());
    assert_eq!(qa.default_priority, TaskPriority::Medium);
    assert_eq!(qa.worker_base_url, DEFAULT_WORKER_BASE_URL);
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
fn empty_base_url_is_rejected() {
    assert_invalid("[qa]\nbase_url = \"  \"\n", "qa.base_url");
}

#[test]
fn legacy_check_config_fails_with_a_migration_error() {
    assert_invalid(
        "[[qa.workspace]]\nname = \"polaris\"\n\
         [[qa.workspace.check]]\nname = \"lint\"\ncommand = \"make lint\"\n",
        "removed inline shell checks",
    );
}

#[test]
fn empty_crew_is_rejected() {
    assert_invalid(
        "[[qa.workspace]]\nname = \"polaris\"\ncrew = \"  \"\n",
        "crew must not be empty",
    );
}

#[test]
fn duplicate_workspaces_are_rejected() {
    assert_invalid(
        "[[qa.workspace]]\nname = \"polaris\"\n[[qa.workspace]]\nname = \"polaris\"\n",
        "more than once",
    );
}

#[test]
fn zero_timeout_is_rejected() {
    assert_invalid(
        "[[qa.workspace]]\nname = \"polaris\"\ntimeout_minutes = 0\n",
        "timeout_minutes",
    );
}

#[test]
fn zero_max_commits_is_rejected() {
    assert_invalid(
        "[[qa.workspace]]\nname = \"polaris\"\nmax_commits = 0\n",
        "max_commits",
    );
}

#[test]
fn workspace_without_name_is_rejected() {
    assert_invalid("[[qa.workspace]]\nbranch = \"agent-main\"\n", "name");
}
