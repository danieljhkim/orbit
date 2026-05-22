//! Sibling tests for `mod.rs` (i.e. the runtime module root; migrated per ORB-00246 / docs/design-patterns/test_layout.md).

use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::OrbitRuntime;

use std::ffi::OsString;
use std::sync::Mutex;

use tempfile::tempdir;

use crate::command::activity::DEFAULT_ACTIVITY_FILES;

static ENV_LOCK: Mutex<()> = Mutex::new(());

fn test_runtime() -> (tempfile::TempDir, OrbitRuntime, PathBuf, PathBuf) {
    let root = tempdir().expect("create tempdir");
    let global_root = root.path().join("global");
    let repo_root = root.path().join("repo");
    let workspace_root = repo_root.join(".orbit");
    std::fs::create_dir_all(&global_root).expect("create global root");
    std::fs::create_dir_all(&workspace_root).expect("create workspace root");
    let runtime =
        OrbitRuntime::from_roots(&global_root, &workspace_root).expect("build test runtime");
    (root, runtime, global_root, workspace_root)
}

#[test]
fn runtime_init_migrates_legacy_learning_ids_and_records_audit_once() {
    let root = tempdir().expect("create tempdir");
    let global_root = root.path().join("global");
    let repo_root = root.path().join("repo");
    let workspace_root = repo_root.join(".orbit");
    let legacy_dir = workspace_root.join("learnings/L20260517-1");
    std::fs::create_dir_all(&global_root).expect("create global root");
    std::fs::create_dir_all(&legacy_dir).expect("create legacy learning dir");
    std::fs::write(
        legacy_dir.join("learning.yaml"),
        "schema_version: 1\nid: L20260517-1\nstatus: active\nscope:\n  paths: []\n  tags: []\nsummary: Legacy learning\nbody: ''\nevidence: []\ncreated_at: 2026-05-17T00:00:00Z\nupdated_at: 2026-05-17T00:00:00Z\n",
    )
    .expect("legacy learning yaml");

    let runtime = OrbitRuntime::from_roots(&global_root, &workspace_root)
        .expect("build runtime with migration");

    assert!(
        workspace_root
            .join("learnings/L-0001/learning.yaml")
            .is_file()
    );
    assert!(!workspace_root.join("learnings/L20260517-1").exists());
    let events = runtime
        .list_audit_events_with_kind(
            None,
            None,
            Some("LearningIdFormatMigration".to_string()),
            None,
            None,
            10,
        )
        .expect("migration audit events");
    assert_eq!(events.len(), 1);
    let payload: Value =
        serde_json::from_str(events[0].arguments_json.as_deref().expect("arguments json"))
            .expect("migration payload");
    assert_eq!(payload["kind"].as_str(), Some("LearningIdFormatMigration"));
    assert_eq!(
        payload["rename_map"]["L20260517-1"].as_str(),
        Some("L-0001")
    );

    let runtime = OrbitRuntime::from_roots(&global_root, &workspace_root)
        .expect("rebuild runtime with no-op migration");
    let events = runtime
        .list_audit_events_with_kind(
            None,
            None,
            Some("LearningIdFormatMigration".to_string()),
            None,
            None,
            10,
        )
        .expect("migration audit events");
    assert_eq!(events.len(), 1);
}

#[test]
fn orbit_root_env_selects_workspace_but_not_global_root() {
    let _guard = ENV_LOCK.lock().expect("lock env");
    let home = tempdir().expect("home tempdir");
    let repo = tempdir().expect("repo tempdir");
    let workspace_root = repo.path().join(".orbit");
    seed_initialized_workspace_root(&workspace_root);
    let _home = EnvVarGuard::set("HOME", home.path().as_os_str().to_os_string());
    let _orbit_root = EnvVarGuard::set("ORBIT_ROOT", workspace_root.as_os_str().to_os_string());

    let resolved_roots =
        OrbitRuntime::resolve_roots_for_cwd(repo.path(), None).expect("resolve roots");

    assert_eq!(resolved_roots.global_root, home.path().join(".orbit"));
    assert_eq!(resolved_roots.shared_root, workspace_root);
    assert_eq!(resolved_roots.local_root, workspace_root);
}

fn seed_initialized_workspace_root(path: &Path) {
    std::fs::create_dir_all(path.join("resources")).expect("create resources dir");
    std::fs::create_dir_all(path.join("tasks")).expect("create tasks dir");
    std::fs::create_dir_all(path.join("state")).expect("create state dir");
}

struct EnvVarGuard {
    key: &'static str,
    previous: Option<OsString>,
}

impl EnvVarGuard {
    fn set(key: &'static str, value: OsString) -> Self {
        let previous = std::env::var_os(key);
        unsafe {
            std::env::set_var(key, value);
        }
        Self { key, previous }
    }
}

impl Drop for EnvVarGuard {
    fn drop(&mut self) {
        match &self.previous {
            Some(value) => unsafe {
                std::env::set_var(self.key, value);
            },
            None => unsafe {
                std::env::remove_var(self.key);
            },
        }
    }
}

fn write_activity(path: &Path, name: &str, description: &str) {
    let yaml = format!(
        r#"schemaVersion: 2
kind: Activity
metadata:
  name: {name}
spec:
  type: deterministic
  description: {description}
  action: test_action
  config: {{}}
"#
    );
    std::fs::create_dir_all(path.parent().expect("activity path has parent"))
        .expect("create activity dir");
    std::fs::write(path, yaml).expect("write activity yaml");
}

fn write_agent_loop_activity(path: &Path, name: &str, tools: &[&str]) {
    let tools_yaml = tools
        .iter()
        .map(|tool| format!("    - {tool}\n"))
        .collect::<String>();
    let yaml = format!(
        r#"schemaVersion: 2
kind: Activity
metadata:
  name: {name}
spec:
  type: agent_loop
  description: Test agent loop.
  instruction: Test.
  tools:
{tools_yaml}"#
    );
    std::fs::create_dir_all(path.parent().expect("activity path has parent"))
        .expect("create activity dir");
    std::fs::write(path, yaml).expect("write activity yaml");
}

#[test]
fn workspace_activity_overrides_global_default_in_catalog() {
    let (_root, runtime, global_root, workspace_root) = test_runtime();
    write_activity(
        &global_root.join("resources/activities/pr_open.yaml"),
        "pr_open",
        "global description",
    );
    write_activity(
        &workspace_root.join("resources/activities/pr_open.yaml"),
        "pr_open",
        "workspace description",
    );

    let catalog = runtime.v2_activity_catalog().expect("activity catalog");
    let activity = catalog.get("pr_open").expect("pr_open activity");
    assert_eq!(activity.description, "workspace description");
}

#[test]
fn duplicate_activities_within_one_catalog_directory_remain_invalid() {
    let (_root, runtime, _global_root, workspace_root) = test_runtime();
    let activities_dir = workspace_root.join("resources/activities");
    write_activity(
        &activities_dir.join("first.yaml"),
        "duplicate_activity",
        "first description",
    );
    write_activity(
        &activities_dir.join("nested/second.yaml"),
        "duplicate_activity",
        "second description",
    );

    let err = runtime
        .v2_activity_catalog()
        .expect_err("duplicate activity name should fail");
    assert!(err.to_string().contains("duplicate activity name"), "{err}");
}

#[test]
fn activity_catalog_accepts_registered_task_wildcard() {
    let (_root, runtime, _global_root, workspace_root) = test_runtime();
    write_agent_loop_activity(
        &workspace_root.join("resources/activities/task_tools.yaml"),
        "task_tools",
        &["orbit.task.*"],
    );

    let catalog = runtime.v2_activity_catalog().expect("activity catalog");

    assert!(catalog.get("task_tools").is_some());
}

#[test]
fn activity_catalog_rejects_unknown_concrete_tool() {
    let (_root, runtime, _global_root, workspace_root) = test_runtime();
    write_agent_loop_activity(
        &workspace_root.join("resources/activities/unknown_tool.yaml"),
        "unknown_tool",
        &["orbit.task.nope"],
    );

    let err = runtime
        .v2_activity_catalog()
        .expect_err("unknown concrete tool should fail");
    let message = err.to_string();

    assert!(message.contains("unknown_tool"), "{message}");
    assert!(message.contains("orbit.task.nope"), "{message}");
    assert!(message.contains("unknown tool name"), "{message}");
}

#[test]
fn activity_catalog_accepts_intentionally_empty_audit_wildcard() {
    let (_root, runtime, _global_root, workspace_root) = test_runtime();
    write_agent_loop_activity(
        &workspace_root.join("resources/activities/audit_tools.yaml"),
        "audit_tools",
        &["orbit.audit.*"],
    );

    let catalog = runtime.v2_activity_catalog().expect("activity catalog");

    assert!(catalog.get("audit_tools").is_some());
}

#[test]
fn get_job_rejects_retired_v1_lookup() {
    let (_root, runtime, _global_root, _workspace_root) = test_runtime();
    let err = runtime
        .get_job("legacy_job")
        .expect_err("v1 job lookup should be fenced");

    let message = err.to_string();
    assert!(message.contains("v1 job lookup is retired"), "{message}");
    assert!(message.contains("orbit job run"), "{message}");
}

#[test]
fn default_activity_catalog_allowlists_resolve_registered_tools() {
    let (_root, runtime, global_root, _workspace_root) = test_runtime();
    let activities_dir = global_root.join("resources/activities");
    for (name, yaml) in DEFAULT_ACTIVITY_FILES {
        let path = activities_dir.join(format!("{name}.yaml"));
        std::fs::create_dir_all(path.parent().expect("activity path has parent"))
            .expect("create activity dir");
        std::fs::write(path, yaml).expect("write activity yaml");
    }

    let catalog = runtime.v2_activity_catalog().expect("activity catalog");

    assert_eq!(catalog.len(), DEFAULT_ACTIVITY_FILES.len());
}
