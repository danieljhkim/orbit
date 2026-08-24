#![allow(missing_docs)]
// ORB-00262: integration coverage for the activity-scoped `proc.spawn`
// allowlist. Fixture setup uses unwrap/expect for readability.
#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::sync::Arc;

use chrono::Utc;
use orbit_common::OrbitError;
use orbit_policy::PolicyEngine;
use orbit_tools::{ToolContext, ToolRegistry};
use orbit_types::policy::{FsProfile, PolicyDef};
use serde_json::{Value, json};
use tempfile::tempdir;

fn registry() -> ToolRegistry {
    let mut registry = ToolRegistry::new();
    registry.register_builtins();
    registry
}

fn unrestricted_activity_context(programs: Vec<String>) -> ToolContext {
    let workspace_root = std::env::current_dir()
        .expect("current directory")
        .canonicalize()
        .expect("canonical current directory");
    ToolContext {
        workspace_root: Some(workspace_root),
        policy_engine: Some(Arc::new(
            PolicyEngine::from_def(&policy_with_profile(
                "unrestricted",
                vec!["./**".to_string()],
            ))
            .expect("unrestricted policy"),
        )),
        fs_profile: Some("unrestricted".to_string()),
        proc_allowed_programs: programs,
        proc_spawn_activity_scoped: true,
        ..Default::default()
    }
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
fn missing_filesystem_policy_denies_an_allowed_scoped_program() {
    let ctx = ToolContext {
        proc_allowed_programs: vec!["/bin/echo".to_string()],
        proc_spawn_activity_scoped: true,
        ..Default::default()
    };
    let err = registry()
        .execute(
            "proc.spawn",
            &ctx,
            json!({ "program": "/bin/echo", "args": ["must-not-run"] }),
        )
        .expect_err("missing activity fsProfile must fail closed");
    assert!(matches!(err, OrbitError::PolicyDenied(_)));
}

#[test]
fn allowed_program_runs_under_lockdown() {
    let ctx = ToolContext {
        proc_spawn_environment: Some(vec![("PATH".to_string(), "/usr/bin:/bin".to_string())]),
        ..unrestricted_activity_context(vec!["echo".to_string(), "/bin/echo".to_string()])
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
fn ambient_credential_is_excluded_unless_policy_admits_it() {
    let ctx = ToolContext {
        proc_spawn_environment: Some(vec![("PATH".to_string(), "/usr/bin:/bin".to_string())]),
        ..unrestricted_activity_context(vec!["/usr/bin/env".to_string()])
    };
    let value = registry()
        .execute(
            "proc.spawn",
            &ctx,
            json!({ "program": "/usr/bin/env", "timeout_ms": 5000 }),
        )
        .expect("allowlisted env should run");
    assert!(
        !value["stdout"]
            .as_str()
            .unwrap_or_default()
            .contains("DATABASE_URL=")
    );

    let admitted = ToolContext {
        proc_spawn_environment: Some(vec![
            ("PATH".to_string(), "/usr/bin:/bin".to_string()),
            ("DATABASE_URL".to_string(), "test-sentinel".to_string()),
        ]),
        ..ctx
    };
    let value = registry()
        .execute(
            "proc.spawn",
            &admitted,
            json!({ "program": "/usr/bin/env", "timeout_ms": 5000 }),
        )
        .expect("explicitly admitted env should run");
    assert!(
        value["stdout"]
            .as_str()
            .unwrap_or_default()
            .contains("DATABASE_URL=test-sentinel")
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
        proc_allowed_programs: vec!["/bin/cat".to_string()],
        proc_spawn_activity_scoped: true,
        ..Default::default()
    };

    let denied = workspace_root.join("denied.txt");
    fs::write(&denied, "must stay private").expect("write denied fixture");

    let err = registry()
        .execute(
            "proc.spawn",
            &ctx,
            json!({ "program": "/bin/cat", "args": [denied], "timeout_ms": 5000 }),
        )
        .expect_err("allowed program must not read a denied path");
    assert!(matches!(err, OrbitError::PolicyDenied(_)));
}

#[test]
fn allowed_program_can_read_an_allowed_path() {
    let workspace = tempdir().expect("workspace tempdir");
    let workspace_root = workspace
        .path()
        .canonicalize()
        .expect("canonical workspace");
    let allowed_dir = workspace_root.join("allowed");
    fs::create_dir(&allowed_dir).expect("create allowed fixture dir");
    let allowed = allowed_dir.join("visible.txt");
    fs::write(&allowed, "visible").expect("write allowed fixture");
    let ctx = ToolContext {
        workspace_root: Some(workspace_root),
        policy_engine: Some(Arc::new(
            PolicyEngine::from_def(&restricted_policy()).expect("policy"),
        )),
        fs_profile: Some("restricted".to_string()),
        proc_allowed_programs: vec!["/bin/cat".to_string()],
        proc_spawn_activity_scoped: true,
        ..Default::default()
    };

    let value = registry()
        .execute(
            "proc.spawn",
            &ctx,
            json!({ "program": "/bin/cat", "args": [allowed], "timeout_ms": 5000 }),
        )
        .expect("allowed path should be readable through allowed program");
    assert_eq!(value["stdout"].as_str().unwrap_or_default(), "visible");
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
        policy_engine: Some(Arc::new(
            PolicyEngine::from_def(&policy_with_profile(
                "unrestricted",
                vec!["./**".to_string()],
            ))
            .expect("policy"),
        )),
        fs_profile: Some("unrestricted".to_string()),
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
    policy_with_profile("restricted", vec!["./allowed/**".to_string()])
}

fn policy_with_profile(name: &str, read: Vec<String>) -> PolicyDef {
    let mut fs_profiles = HashMap::new();
    fs_profiles.insert(
        name.to_string(),
        FsProfile {
            read: read.clone(),
            modify: read,
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
