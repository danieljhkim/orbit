#![allow(missing_docs)]
// ORB-00013: Tests use unwrap/expect to keep fixture setup readable.
#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::collections::HashMap;
use std::fs;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use chrono::Utc;
use orbit_common::types::{FsProfile, OrbitError, PolicyDef};
use orbit_common::utility::logging::init_default_subscriber;
use orbit_policy::PolicyEngine;
use orbit_tools::{FsAuditLogger, FsCallEvent, FsCallEventKind, ToolContext, ToolRegistry};
use serde_json::{Value, json};
use tempfile::tempdir;

#[test]
fn policy_denials_emit_redacted_jsonl_tracing_events() {
    let home = tempdir().expect("create temp home");
    let workspace = tempdir().expect("create workspace");
    let log_path = home.path().join(".orbit/state/logs/orbit.jsonl");

    // SAFETY: this single-test integration binary mutates the process
    // environment before installing Orbit's subscriber or starting its writer.
    unsafe {
        std::env::set_var("HOME", home.path());
        std::env::set_var("RUST_LOG", "warn");
    }
    init_default_subscriber("warn");

    let audit = Arc::new(RecordingFsAudit::default());
    let fs_ctx = ToolContext {
        workspace_root: Some(workspace.path().to_path_buf()),
        policy_engine: Some(Arc::new(
            PolicyEngine::from_def(&restricted_policy()).expect("valid policy"),
        )),
        fs_profile: Some("restricted".to_string()),
        fs_audit: Some(audit.clone()),
        ..Default::default()
    };
    let denied_path = workspace
        .path()
        .join("secrets")
        .join("Authorization: Bearer abc123.txt");

    let mut registry = ToolRegistry::new();
    registry.register_builtins();

    let err = registry
        .execute(
            "fs.read",
            &fs_ctx,
            json!({
                "path": denied_path.display().to_string(),
            }),
        )
        .expect_err("fs read should be denied");
    assert!(matches!(err, OrbitError::PolicyDenied(_)));

    let proc_ctx = ToolContext {
        proc_allowed_programs: vec!["echo".to_string()],
        ..Default::default()
    };
    let err = registry
        .execute("proc.spawn", &proc_ctx, json!({ "program": "sh" }))
        .expect_err("proc spawn should be denied");
    assert!(matches!(err, OrbitError::PolicyDenied(_)));

    let audit_events = audit.events();
    assert_eq!(audit_events.len(), 1);
    assert_eq!(audit_events[0].kind, FsCallEventKind::Denied);
    assert!(audit_events[0].path.contains("abc123"));

    let events = wait_for_policy_events(&log_path, 2);
    let fs_event = events
        .iter()
        .find(|event| event["fields"]["tool"] == "fs.read")
        .expect("fs deny tracing event");
    assert_eq!(fs_event["fields"]["profile"], "restricted");
    assert_eq!(fs_event["fields"]["matched_rule"], "<no matching rule>");
    let redacted_path = fs_event["fields"]["path"].as_str().expect("fs path field");
    assert!(redacted_path.contains("[REDACTED_AUTH]"));
    assert!(!redacted_path.contains("abc123"));

    let proc_event = events
        .iter()
        .find(|event| event["fields"]["tool"] == "proc.spawn")
        .expect("proc deny tracing event");
    assert_eq!(proc_event["fields"]["path"], "sh");
    assert_eq!(proc_event["fields"]["profile"], "proc.allowed_programs");
    assert_eq!(proc_event["fields"]["matched_rule"], "echo");
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

fn wait_for_policy_events(log_path: &std::path::Path, expected: usize) -> Vec<Value> {
    let deadline = Instant::now() + Duration::from_secs(2);
    while Instant::now() < deadline {
        let events = fs::read_to_string(log_path)
            .ok()
            .map(|raw| {
                raw.lines()
                    .filter_map(|line| serde_json::from_str::<Value>(line).ok())
                    .filter(|event| event["target"] == "orbit.policy.deny")
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        if events.len() >= expected {
            return events;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    panic!("timed out waiting for {expected} policy deny events at {log_path:?}");
}

#[derive(Default)]
struct RecordingFsAudit {
    events: Mutex<Vec<FsCallEvent>>,
}

impl RecordingFsAudit {
    fn events(&self) -> Vec<FsCallEvent> {
        self.events.lock().expect("events lock").clone()
    }
}

impl FsAuditLogger for RecordingFsAudit {
    fn emit(&self, event: FsCallEvent) -> Result<(), OrbitError> {
        self.events.lock().expect("events lock").push(event);
        Ok(())
    }
}
