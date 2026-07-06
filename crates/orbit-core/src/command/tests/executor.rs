use chrono::Utc;
use orbit_common::types::{ExecutorDef, ExecutorSandboxKind, ExecutorType};

use crate::command::executor::{migrated_default_executor, parse_default_executor};

fn base_def(name: &str, executor_type: ExecutorType) -> ExecutorDef {
    let now = Utc::now();
    ExecutorDef {
        name: name.to_string(),
        executor_type,
        command: Some("noop".to_string()),
        args: Vec::new(),
        stdout_format: None,
        model_pair_override: None,
        model_flag: None,
        timeout_seconds: None,
        env: Default::default(),
        sandbox: None,
        allow_fallback: false,
        created_at: now,
        updated_at: now,
    }
}

const CLAUDE_YAML: &str = r#"schemaVersion: 2
kind: Executor
metadata:
  name: claude
spec:
  executor_type: direct_agent
  command: claude
  sandbox: macos-sandbox-exec
"#;

/// On macOS the sandbox declaration matches the host and must survive parsing —
/// otherwise ORB-10047's fix would silently disarm the sandbox everywhere.
#[cfg(target_os = "macos")]
#[test]
fn parse_default_executor_preserves_sandbox_on_matching_platform() {
    let def = parse_default_executor("claude", CLAUDE_YAML).expect("parse");
    assert_eq!(def.sandbox, Some(ExecutorSandboxKind::MacosSandboxExec));
}

/// On Linux the shipped `macos-sandbox-exec` declaration cannot be enforced;
/// keeping it would make every crew dispatch fail closed at dispatch time.
/// See ORB-10047.
#[cfg(not(target_os = "macos"))]
#[test]
fn parse_default_executor_scrubs_sandbox_on_mismatched_platform() {
    let def = parse_default_executor("claude", CLAUDE_YAML).expect("parse");
    assert_eq!(def.sandbox, None);
}

/// Re-seeding must scrub a platform-mismatched sandbox left over from a
/// prior install, so an upgrade on Linux clears a persisted
/// `macos-sandbox-exec` even though the executor type is unchanged.
#[cfg(not(target_os = "macos"))]
#[test]
fn migrated_default_executor_scrubs_platform_mismatched_sandbox_on_non_matching_host() {
    let mut existing = base_def("claude", ExecutorType::DirectAgent);
    existing.sandbox = Some(ExecutorSandboxKind::MacosSandboxExec);
    let seeded = base_def("claude", ExecutorType::DirectAgent);

    let migrated =
        migrated_default_executor(&existing, &seeded).expect("scrub should produce a migrated def");
    assert_eq!(migrated.sandbox, None);
    assert_eq!(migrated.executor_type, ExecutorType::DirectAgent);
}

/// On macOS the persisted sandbox is host-compatible; re-seeding must not
/// touch it and must not force a rewrite.
#[cfg(target_os = "macos")]
#[test]
fn migrated_default_executor_preserves_sandbox_on_matching_host() {
    let mut existing = base_def("claude", ExecutorType::DirectAgent);
    existing.sandbox = Some(ExecutorSandboxKind::MacosSandboxExec);
    let seeded = base_def("claude", ExecutorType::DirectAgent);

    assert!(migrated_default_executor(&existing, &seeded).is_none());
}

/// The pre-ORB-10047 `AgentCli → DirectAgent` migration path must still fire
/// so upgrades from older on-disk executor defs continue to move forward.
#[test]
fn migrated_default_executor_still_migrates_agent_cli_to_direct_agent() {
    let existing = base_def("claude", ExecutorType::AgentCli);
    let seeded = base_def("claude", ExecutorType::DirectAgent);

    let migrated = migrated_default_executor(&existing, &seeded).expect("type migration");
    assert_eq!(migrated.executor_type, ExecutorType::DirectAgent);
    assert_eq!(migrated.sandbox, None);
}

#[test]
fn migrated_default_executor_returns_none_when_nothing_needs_migrating() {
    let existing = base_def("claude", ExecutorType::DirectAgent);
    let seeded = base_def("claude", ExecutorType::DirectAgent);
    assert!(migrated_default_executor(&existing, &seeded).is_none());
}
