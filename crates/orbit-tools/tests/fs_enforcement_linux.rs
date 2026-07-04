#![allow(missing_docs)]
// [ORB-10009] Fixture setup uses unwrap/expect for readability.
#![allow(clippy::expect_used, clippy::unwrap_used)]
#![cfg(unix)]

//! [ORB-10009] Runtime filesystem-enforcement integration tests.
//!
//! On Linux there is **no kernel sandbox** (the macOS leg wraps children in
//! `sandbox-exec`; see `orbit-exec::macos_sandbox`). The only enforcement
//! layer is this tool-layer seam: `check_workspace_boundary` +
//! `PolicyEngine::check_resolved` consulted by every fs builtin before the
//! filesystem is touched. These tests drive real tool executions through
//! `ToolRegistry` against a real temp workspace and assert observable
//! effects — content returned, files surviving denied deletes — not just
//! policy verdicts. The symlink cases are the runtime-enforcement side of
//! the [ORB-00418] canonicalization fix.
//!
//! Everything here is plain userspace (tempdirs + symlinks), so no kernel
//! feature gating is needed; the suite is expected to pass on any unix
//! runner, unprivileged.

use std::collections::HashMap;
use std::os::unix::fs::symlink;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use chrono::Utc;
use orbit_common::types::{FsProfile, OrbitError, PolicyDef};
use orbit_policy::PolicyEngine;
use orbit_tools::{FsAuditLogger, FsCallEvent, FsCallEventKind, ToolContext, ToolRegistry};
use serde_json::json;
use tempfile::{TempDir, tempdir};

/// Policy under test:
/// - profile `restricted`: read/modify only inside `allowed/`
/// - profile `broad`: read/modify everywhere, minus the global denies
/// - global denyRead/denyModify: `denied/**`
fn policy() -> PolicyDef {
    let mut fs_profiles = HashMap::new();
    fs_profiles.insert(
        "restricted".to_string(),
        FsProfile {
            read: vec!["allowed/**".to_string()],
            modify: vec!["allowed/**".to_string()],
        },
    );
    fs_profiles.insert(
        "broad".to_string(),
        FsProfile {
            read: vec!["./**".to_string()],
            modify: vec!["./**".to_string()],
        },
    );
    PolicyDef {
        name: "enforcement".to_string(),
        description: None,
        deny_read: vec!["denied/**".to_string()],
        deny_modify: vec!["denied/**".to_string()],
        fs_profiles,
        created_at: Utc::now(),
        updated_at: Utc::now(),
    }
}

struct Fixture {
    _workspace: TempDir,
    root: PathBuf,
    registry: ToolRegistry,
    audit: Arc<RecordingFsAudit>,
}

impl Fixture {
    fn new() -> Self {
        let workspace = tempdir().expect("workspace tempdir");
        let root = workspace.path().canonicalize().expect("canonical root");
        std::fs::create_dir(root.join("allowed")).expect("allowed dir");
        std::fs::create_dir(root.join("denied")).expect("denied dir");
        std::fs::create_dir(root.join("outside_profile")).expect("outside_profile dir");
        std::fs::write(root.join("allowed/ok.txt"), "inside-allowed").expect("ok.txt");
        std::fs::write(root.join("denied/secret.txt"), "top-secret").expect("secret.txt");
        std::fs::write(root.join("outside_profile/data.txt"), "unlisted").expect("data.txt");

        let mut registry = ToolRegistry::new();
        registry.register_builtins();
        Self {
            _workspace: workspace,
            root,
            registry,
            audit: Arc::new(RecordingFsAudit::default()),
        }
    }

    fn ctx(&self, profile: &str) -> ToolContext {
        ToolContext {
            workspace_root: Some(self.root.clone()),
            policy_engine: Some(Arc::new(
                PolicyEngine::from_def(&policy()).expect("valid policy"),
            )),
            fs_profile: Some(profile.to_string()),
            fs_audit: Some(self.audit.clone()),
            ..Default::default()
        }
    }

    fn read(&self, profile: &str, path: &Path) -> Result<serde_json::Value, OrbitError> {
        self.registry.execute(
            "fs.read",
            &self.ctx(profile),
            json!({ "path": path.display().to_string() }),
        )
    }

    fn delete(&self, profile: &str, path: &Path) -> Result<serde_json::Value, OrbitError> {
        self.registry.execute(
            "fs.delete",
            &self.ctx(profile),
            json!({ "path": path.display().to_string() }),
        )
    }
}

// --- Allow inside / deny outside the profile paths ---

#[test]
fn read_inside_allowed_profile_paths_returns_content() {
    let fx = Fixture::new();
    let value = fx
        .read("restricted", &fx.root.join("allowed/ok.txt"))
        .expect("allowed read must succeed");
    assert_eq!(value["content"], "inside-allowed");

    let events = fx.audit.events();
    assert!(
        events
            .iter()
            .any(|event| event.kind == FsCallEventKind::Request && event.allowed),
        "allowed read must emit an allowed request audit event: {events:?}"
    );
}

#[test]
fn read_outside_allowed_profile_paths_is_denied() {
    let fx = Fixture::new();
    let err = fx
        .read("restricted", &fx.root.join("outside_profile/data.txt"))
        .expect_err("read outside profile paths must be denied");
    assert!(matches!(err, OrbitError::PolicyDenied(_)), "{err:?}");

    let events = fx.audit.events();
    assert!(
        events
            .iter()
            .any(|event| event.kind == FsCallEventKind::Denied),
        "denied read must emit a denied audit event: {events:?}"
    );
}

#[test]
fn delete_outside_allowed_profile_paths_is_denied_and_file_survives() {
    let fx = Fixture::new();
    let target = fx.root.join("outside_profile/data.txt");
    let err = fx
        .delete("restricted", &target)
        .expect_err("delete outside profile paths must be denied");
    assert!(matches!(err, OrbitError::PolicyDenied(_)), "{err:?}");
    assert!(
        target.exists(),
        "denied delete must leave the file untouched"
    );
}

#[test]
fn delete_inside_allowed_profile_paths_removes_file() {
    let fx = Fixture::new();
    let target = fx.root.join("allowed/ok.txt");
    fx.delete("restricted", &target)
        .expect("allowed delete must succeed");
    assert!(!target.exists(), "allowed delete must remove the file");
}

#[test]
fn global_deny_beats_broad_profile_allow_at_runtime() {
    let fx = Fixture::new();
    let err = fx
        .read("broad", &fx.root.join("denied/secret.txt"))
        .expect_err("global denyRead must beat the broad profile allow");
    assert!(matches!(err, OrbitError::PolicyDenied(_)), "{err:?}");
}

// --- Symlink escapes (runtime side of [ORB-00418]) ---

#[test]
fn read_through_symlink_into_denied_subtree_is_denied() {
    let fx = Fixture::new();
    // Link lives inside the broad-allowed tree but targets the denied one.
    symlink(fx.root.join("denied"), fx.root.join("allowed/link")).expect("symlink");

    let err = fx
        .read("broad", &fx.root.join("allowed/link/secret.txt"))
        .expect_err("read through a symlink into a denied subtree must be denied");
    assert!(matches!(err, OrbitError::PolicyDenied(_)), "{err:?}");
    let message = format!("{err}");
    assert!(
        message.contains("denied"),
        "denial should attribute the real (resolved) location: {message}"
    );
}

#[test]
fn read_through_symlink_escaping_workspace_is_denied() {
    let outside = tempdir().expect("outside tempdir");
    std::fs::write(outside.path().join("passwd"), "root:x:0:0").expect("outside file");

    let fx = Fixture::new();
    symlink(outside.path(), fx.root.join("allowed/escape")).expect("symlink");

    let err = fx
        .read("broad", &fx.root.join("allowed/escape/passwd"))
        .expect_err("symlink escaping the workspace must be denied");
    assert!(matches!(err, OrbitError::PolicyDenied(_)), "{err:?}");
    assert!(
        format!("{err}").contains("outside workspace"),
        "boundary check should attribute the escape: {err}"
    );
}

#[test]
fn delete_through_dangling_symlink_targeting_denied_subtree_is_denied() {
    let fx = Fixture::new();
    // Dangling link: target does not exist, but an unlink/O_CREAT through it
    // operates on the *target* location — must be evaluated there.
    symlink(
        fx.root.join("denied/planted.txt"),
        fx.root.join("allowed/dangling"),
    )
    .expect("symlink");

    let err = fx
        .delete("broad", &fx.root.join("allowed/dangling"))
        .expect_err("modify through a dangling symlink must be evaluated at the target");
    assert!(matches!(err, OrbitError::PolicyDenied(_)), "{err:?}");
}

#[test]
fn workspace_sibling_with_shared_name_prefix_is_still_outside() {
    // Prefix-collision trap: `<root>-evil` shares a string prefix with the
    // workspace root; the boundary check must compare path components, not
    // string prefixes.
    let parent = tempdir().expect("parent tempdir");
    let root = parent.path().join("ws");
    let evil = parent.path().join("ws-evil");
    std::fs::create_dir_all(root.join("allowed")).expect("workspace dirs");
    std::fs::create_dir_all(&evil).expect("evil sibling");
    std::fs::write(evil.join("loot.txt"), "loot").expect("loot");
    symlink(evil.join("loot.txt"), root.join("allowed/link")).expect("symlink");

    let mut registry = ToolRegistry::new();
    registry.register_builtins();
    let ctx = ToolContext {
        workspace_root: Some(root.canonicalize().expect("canonical root")),
        policy_engine: Some(Arc::new(
            PolicyEngine::from_def(&policy()).expect("valid policy"),
        )),
        fs_profile: Some("broad".to_string()),
        ..Default::default()
    };
    let err = registry
        .execute(
            "fs.read",
            &ctx,
            json!({ "path": root.join("allowed/link").display().to_string() }),
        )
        .expect_err("sibling directory sharing a name prefix must stay outside the workspace");
    assert!(matches!(err, OrbitError::PolicyDenied(_)), "{err:?}");
}

// --- Audit recorder ---

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
