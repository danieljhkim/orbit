#![allow(missing_docs)]
// ORB-00262: integration coverage for the activity-scoped `proc.spawn`
// allowlist. Fixture setup uses unwrap/expect for readability.
#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use chrono::Utc;
use orbit_common::types::{FsProfile, OrbitError, PolicyDef};
use orbit_policy::PolicyEngine;
use orbit_tools::{ToolContext, ToolRegistry};
use serde_json::{Value, json};
use tempfile::tempdir;

fn registry() -> ToolRegistry {
    let mut registry = ToolRegistry::new();
    registry.register_builtins();
    registry
}

#[test]
fn disallowed_program_denied_when_activity_scoped() {
    let ctx = ToolContext {
        proc_allowed_programs: vec!["git".to_string()],
        proc_spawn_activity_scoped: true,
        ..Default::default()
    };
    let err = registry()
        .execute("proc.spawn", &ctx, json!({ "program": "sh" }))
        .expect_err("disallowed program must be denied");
    assert!(matches!(err, OrbitError::PolicyDenied(_)));
}

#[test]
fn empty_allowlist_denies_every_program_when_scoped() {
    let ctx = ToolContext {
        proc_allowed_programs: Vec::new(),
        proc_spawn_activity_scoped: true,
        ..Default::default()
    };
    let err = registry()
        .execute("proc.spawn", &ctx, json!({ "program": "git" }))
        .expect_err("empty scoped allowlist must deny");
    assert!(matches!(err, OrbitError::PolicyDenied(_)));
}

#[test]
fn allowed_program_runs_under_lockdown() {
    let ctx = ToolContext {
        proc_allowed_programs: vec!["echo".to_string(), "/bin/echo".to_string()],
        proc_spawn_activity_scoped: true,
        ..Default::default()
    };
    let value = registry()
        .execute(
            "proc.spawn",
            &ctx,
            json!({
                "program": "/bin/echo",
                "args": ["ok"],
                "timeout_ms": 5000,
            }),
        )
        .expect("allowed program should run");
    let stdout = value["stdout"].as_str().unwrap_or_default();
    assert!(
        stdout.contains("ok"),
        "expected `ok` in stdout, got: {stdout:?}"
    );
}

#[test]
fn legacy_unrestricted_path_preserved_when_not_scoped() {
    let ctx = ToolContext {
        // No allowlist, not activity-scoped — legacy v1/direct-CLI behavior.
        ..Default::default()
    };
    registry()
        .execute(
            "proc.spawn",
            &ctx,
            json!({
                "program": "/bin/echo",
                "args": ["legacy"],
                "timeout_ms": 5000,
            }),
        )
        .expect("legacy unrestricted path should still permit echo");
}

#[test]
fn restrictive_fs_profile_not_bypassed_via_proc_spawn() {
    let workspace = tempdir().expect("workspace tempdir");
    let workspace_root: PathBuf = workspace
        .path()
        .canonicalize()
        .expect("canonical workspace path");

    let ctx = ToolContext {
        workspace_root: Some(workspace_root.clone()),
        policy_engine: Some(Arc::new(
            PolicyEngine::from_def(&restricted_policy()).expect("policy"),
        )),
        fs_profile: Some("restricted".to_string()),
        proc_allowed_programs: Vec::new(),
        proc_spawn_activity_scoped: true,
        ..Default::default()
    };

    let err = registry()
        .execute(
            "proc.spawn",
            &ctx,
            json!({ "program": "sh", "args": ["-c", "echo escape"] }),
        )
        .expect_err("scoped empty allowlist must deny under restrictive fsProfile");
    assert!(matches!(err, OrbitError::PolicyDenied(_)));
}

#[test]
fn spawn_runs_inside_workspace_root() {
    let workspace = tempdir().expect("workspace tempdir");
    let workspace_root: PathBuf = workspace
        .path()
        .canonicalize()
        .expect("canonical workspace path");

    let ctx = ToolContext {
        workspace_root: Some(workspace_root.clone()),
        proc_allowed_programs: vec!["/bin/pwd".to_string(), "pwd".to_string()],
        proc_spawn_activity_scoped: true,
        ..Default::default()
    };

    let value: Value = registry()
        .execute(
            "proc.spawn",
            &ctx,
            json!({ "program": "/bin/pwd", "timeout_ms": 5000 }),
        )
        .expect("pwd should run");
    let stdout = value["stdout"].as_str().unwrap_or_default().trim();
    let observed: PathBuf = PathBuf::from(stdout)
        .canonicalize()
        .unwrap_or_else(|_| PathBuf::from(stdout));
    assert_eq!(
        observed, workspace_root,
        "expected pwd inside workspace_root ({workspace_root:?}), got {observed:?}"
    );
}

fn restricted_policy() -> PolicyDef {
    let mut fs_profiles = HashMap::new();
    fs_profiles.insert(
        "restricted".to_string(),
        FsProfile {
            read: vec!["./allowed/**".to_string()],
            modify: vec!["./allowed/**".to_string()],
        },
    );

    PolicyDef {
        name: "test".to_string(),
        description: None,
        deny_read: Vec::new(),
        deny_modify: Vec::new(),
        fs_profiles,
        created_at: Utc::now(),
        updated_at: Utc::now(),
    }
}
