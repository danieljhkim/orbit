use std::fs;
use std::path::{Path, PathBuf};

use chrono::{TimeZone, Utc};
use orbit_common::types::{
    OrbitError, WORKSPACE_REGISTRY_SCHEMA_VERSION, Workspace, WorkspaceCheckout,
    WorkspaceCheckoutRole, WorkspaceRegistry, WorkspaceStatus,
};
use serde_json::{Value, json};
use tempfile::tempdir;

use super::{
    assign_checkout_role, find_checkout_by_path, find_workspace, find_workspace_by_path,
    load_registry_from, load_registry_from_with_writer, save_registry_to,
};

fn timestamp() -> chrono::DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 7, 18, 1, 2, 3)
        .single()
        .expect("fixed timestamp")
}

fn logical_workspace(id: &str, owner_machine_id: Option<&str>) -> Workspace {
    Workspace {
        id: id.to_string(),
        name: id.trim_start_matches("ws_").to_string(),
        owner_machine_id: owner_machine_id.map(str::to_string),
        git_remote: Some("git@example.test:orbit/repo.git".to_string()),
        ship_mode: Some("pr".to_string()),
        base_branch: "agent-main".to_string(),
        status: WorkspaceStatus::Active,
        created_at: timestamp(),
        updated_at: timestamp(),
    }
}

fn write_host_identity(root: &Path, mode: &str, machine_id: &str) {
    fs::write(
        root.join("host.toml"),
        format!(
            "schema_version = 1\nmachine_id = \"{machine_id}\"\nhost_id = \"test-host\"\nmode = \"{mode}\"\n"
        ),
    )
    .expect("write host identity");
}

fn write_json(path: &Path, value: &Value) -> Vec<u8> {
    let bytes = serde_json::to_vec_pretty(value).expect("serialize fixture");
    fs::write(path, &bytes).expect("write fixture");
    bytes
}

#[test]
fn legacy_registry_migrates_to_path_free_catalog_and_is_byte_stable() {
    let root = tempdir().expect("tempdir");
    let path = root.path().join("workspaces.json");
    let repo_root = root.path().join("repo");
    let orbit_dir = repo_root.join(".orbit");
    let override_path = root.path().join("linked-worktree");
    write_json(
        &path,
        &json!({
            "workspaces": [{
                "id": "ws_orbit",
                "name": "orbit",
                "root": repo_root,
                "orbit_dir": orbit_dir,
                "git_remote": "git@example.test:orbit/orbit.git",
                "ship_mode": "pr",
                "base_branch": "agent-main",
                "status": "active",
                "created_at": "2026-07-18T01:02:03Z",
                "updated_at": "2026-07-18T01:02:03Z"
            }],
            "path_overrides": {
                (override_path.to_string_lossy().to_string()): "ws_orbit",
                (root.path().join("dangling").to_string_lossy().to_string()): "ws_missing"
            }
        }),
    );

    let migrated = load_registry_from(&path).expect("migrate registry");
    assert_eq!(migrated.schema_version, WORKSPACE_REGISTRY_SCHEMA_VERSION);
    assert_eq!(migrated.workspaces.len(), 1);
    assert_eq!(migrated.checkouts.len(), 1);
    let workspace = &migrated.workspaces[0];
    assert_eq!(workspace.id, "ws_orbit");
    assert_eq!(workspace.name, "orbit");
    assert_eq!(
        workspace.git_remote.as_deref(),
        Some("git@example.test:orbit/orbit.git")
    );
    assert_eq!(workspace.ship_mode.as_deref(), Some("pr"));
    assert_eq!(workspace.base_branch, "agent-main");
    assert_eq!(workspace.created_at, timestamp());
    assert_eq!(workspace.updated_at, timestamp());
    let checkout = &migrated.checkouts[0];
    assert_eq!(checkout.workspace_id, "ws_orbit");
    assert_eq!(checkout.role, Some(WorkspaceCheckoutRole::Owner));
    assert_eq!(checkout.path_overrides, vec![override_path]);

    let persisted: Value =
        serde_json::from_slice(&fs::read(&path).expect("read migrated registry"))
            .expect("parse migrated registry");
    assert!(persisted["workspaces"][0].get("root").is_none());
    assert!(persisted["workspaces"][0].get("orbit_dir").is_none());
    assert!(persisted.get("path_overrides").is_none());
    assert_eq!(persisted["checkouts"][0]["role"], "owner");

    let first_bytes = fs::read(&path).expect("read first migration");
    let second = load_registry_from(&path).expect("load migrated registry again");
    assert_eq!(second, migrated);
    assert_eq!(fs::read(&path).expect("read second migration"), first_bytes);
}

#[test]
fn standalone_missing_role_canonicalizes_to_local_owner() {
    let root = tempdir().expect("tempdir");
    let path = root.path().join("workspaces.json");
    write_json(
        &path,
        &json!({
            "schema_version": 1,
            "workspaces": [logical_workspace("ws_orbit", None)],
            "checkouts": [{
                "workspace_id": "ws_orbit",
                "repo_root": "/repos/orbit",
                "orbit_dir": "/repos/orbit/.orbit"
            }]
        }),
    );

    let registry = load_registry_from(&path).expect("load standalone registry");
    assert_eq!(
        registry.checkouts[0].role,
        Some(WorkspaceCheckoutRole::Owner)
    );
    let persisted: Value =
        serde_json::from_slice(&fs::read(&path).expect("read registry")).expect("parse registry");
    assert_eq!(persisted["checkouts"][0]["role"], "owner");
}

#[test]
fn multi_host_legacy_registry_rejects_missing_role_without_rewriting() {
    for mode in ["hub", "spoke"] {
        let root = tempdir().expect("tempdir");
        write_host_identity(root.path(), mode, "hm_local");
        let path = root.path().join("workspaces.json");
        let original = write_json(
            &path,
            &json!({
                "workspaces": [{
                    "id": "ws_legacy",
                    "name": "legacy",
                    "root": "/repos/legacy",
                    "orbit_dir": "/repos/legacy/.orbit",
                    "git_remote": null,
                    "base_branch": "main",
                    "status": "active",
                    "created_at": "2026-07-18T01:02:03Z",
                    "updated_at": "2026-07-18T01:02:03Z"
                }],
                "path_overrides": {}
            }),
        );

        let error = load_registry_from(&path).expect_err("legacy role must not be inferred");
        let message = error.to_string();
        assert!(message.contains("ws_legacy"), "{message}");
        assert!(
            message.contains("missing a local checkout role"),
            "{message}"
        );
        assert_eq!(fs::read(&path).expect("read unchanged registry"), original);
    }
}

#[test]
fn multi_host_modes_reject_missing_unknown_and_contradictory_roles_by_workspace_id() {
    let cases = [
        (
            "hub",
            json!({
                "workspace_id": "ws_missing_role",
                "repo_root": "/repos/missing",
                "orbit_dir": "/repos/missing/.orbit"
            }),
            "missing a local checkout role",
        ),
        (
            "spoke",
            json!({
                "workspace_id": "ws_unknown_role",
                "repo_root": "/repos/unknown",
                "orbit_dir": "/repos/unknown/.orbit",
                "role": "secondary"
            }),
            "unknown checkout role 'secondary'",
        ),
        (
            "hub",
            json!({
                "workspace_id": "ws_contradictory",
                "repo_root": "/repos/contradictory",
                "orbit_dir": "/repos/contradictory/.orbit",
                "role": "owner"
            }),
            "logical owner is machine 'hm_remote'",
        ),
        (
            "spoke",
            json!({
                "workspace_id": "ws_replica_without_owner",
                "repo_root": "/repos/replica",
                "orbit_dir": "/repos/replica/.orbit",
                "role": "replica"
            }),
            "replica role without owner_machine_id",
        ),
    ];

    for (mode, checkout, expected) in cases {
        let root = tempdir().expect("tempdir");
        write_host_identity(root.path(), mode, "hm_local");
        let path = root.path().join("workspaces.json");
        let workspace_id = checkout["workspace_id"].as_str().expect("workspace id");
        let owner = match workspace_id {
            "ws_contradictory" | "ws_replica_without_owner" => Some("hm_remote"),
            _ => Some("hm_local"),
        };
        let original = write_json(
            &path,
            &json!({
                "schema_version": 1,
                "workspaces": [logical_workspace(workspace_id, owner)],
                "checkouts": [checkout]
            }),
        );

        let error = load_registry_from(&path).expect_err("invalid role must fail closed");
        let message = error.to_string();
        assert!(message.contains(workspace_id), "{message}");
        assert!(message.contains(expected), "{message}");
        assert_eq!(fs::read(&path).expect("read unchanged registry"), original);
    }
}

#[test]
fn valid_spoke_replica_requires_and_preserves_owner_machine() {
    let root = tempdir().expect("tempdir");
    write_host_identity(root.path(), "spoke", "hm_local");
    let path = root.path().join("workspaces.json");
    write_json(
        &path,
        &json!({
            "schema_version": 1,
            "workspaces": [logical_workspace("ws_orbit", Some("hm_owner"))],
            "checkouts": [{
                "workspace_id": "ws_orbit",
                "repo_root": "/repos/orbit",
                "orbit_dir": "/repos/orbit/.orbit",
                "role": "replica",
                "owner_machine_id": "hm_owner"
            }]
        }),
    );

    let registry = load_registry_from(&path).expect("valid replica registry");
    assert_eq!(
        registry.checkouts[0].role,
        Some(WorkspaceCheckoutRole::Replica)
    );
    assert_eq!(
        registry.checkouts[0].owner_machine_id.as_deref(),
        Some("hm_owner")
    );
}

#[test]
fn identity_lookup_is_path_independent_and_path_lookup_is_checkout_only() {
    let mut registry = WorkspaceRegistry {
        workspaces: vec![
            logical_workspace("ws_outer", None),
            logical_workspace("ws_inner", None),
            logical_workspace("ws_remote", Some("hm_remote")),
        ],
        checkouts: vec![
            WorkspaceCheckout::owner(
                "ws_outer".to_string(),
                PathBuf::from("/repos"),
                PathBuf::from("/repos/.orbit"),
            ),
            WorkspaceCheckout::owner(
                "ws_inner".to_string(),
                PathBuf::from("/different/inner"),
                PathBuf::from("/different/inner/.orbit"),
            ),
        ],
        ..Default::default()
    };
    registry.checkouts[1]
        .path_overrides
        .push(PathBuf::from("/repos/inner"));

    assert_eq!(
        find_workspace(&registry, "ws_remote").map(|workspace| workspace.id.as_str()),
        Some("ws_remote")
    );
    assert_eq!(
        find_workspace(&registry, "remote").map(|workspace| workspace.id.as_str()),
        Some("ws_remote")
    );
    assert_eq!(
        find_workspace_by_path(&registry, Path::new("/repos/inner/src"))
            .map(|workspace| workspace.id.as_str()),
        Some("ws_inner")
    );
    assert_eq!(
        find_checkout_by_path(&registry, Path::new("/repos/inner/src"))
            .map(|checkout| checkout.workspace_id.as_str()),
        Some("ws_inner")
    );
    assert!(find_workspace_by_path(&registry, Path::new("/remote/ws_remote")).is_none());
}

#[test]
fn malformed_and_future_registries_fail_without_rewriting() {
    let root = tempdir().expect("tempdir");
    for (name, bytes, expected) in [
        ("malformed", b"{ not json".to_vec(), "malformed JSON"),
        (
            "future",
            serde_json::to_vec_pretty(&json!({
                "schema_version": WORKSPACE_REGISTRY_SCHEMA_VERSION + 1,
                "workspaces": [],
                "checkouts": []
            }))
            .expect("serialize future fixture"),
            "unsupported schema_version",
        ),
    ] {
        let path = root.path().join(format!("{name}.json"));
        fs::write(&path, &bytes).expect("write invalid fixture");
        let error = load_registry_from(&path).expect_err("invalid registry must fail");
        assert!(error.to_string().contains(expected), "{error}");
        assert_eq!(fs::read(&path).expect("read unchanged fixture"), bytes);
    }
}

#[test]
fn injected_migration_write_failure_preserves_readable_legacy_registry() {
    let root = tempdir().expect("tempdir");
    let path = root.path().join("workspaces.json");
    let original = write_json(
        &path,
        &json!({
            "workspaces": [{
                "id": "ws_orbit",
                "name": "orbit",
                "root": "/repos/orbit",
                "orbit_dir": "/repos/orbit/.orbit",
                "git_remote": null,
                "base_branch": "main",
                "status": "active",
                "created_at": "2026-07-18T01:02:03Z",
                "updated_at": "2026-07-18T01:02:03Z"
            }],
            "path_overrides": {}
        }),
    );

    let error = load_registry_from_with_writer(&path, |_, _| {
        Err(OrbitError::Io("injected write failure".to_string()))
    })
    .expect_err("write failure must surface");
    assert!(error.to_string().contains("injected write failure"));
    assert_eq!(
        fs::read(&path).expect("read preserved legacy file"),
        original
    );

    let recovered = load_registry_from(&path).expect("legacy source remains migratable");
    assert_eq!(recovered.workspaces[0].id, "ws_orbit");
    assert_eq!(recovered.checkouts[0].workspace_id, "ws_orbit");
}

#[test]
fn assign_checkout_role_is_idempotent_and_rejects_owner_of_another_machine_byte_valid() {
    let root = tempdir().expect("tempdir");
    write_host_identity(root.path(), "spoke", "hm_local");
    let path = root.path().join("workspaces.json");
    write_json(
        &path,
        &json!({
            "schema_version": 1,
            "workspaces": [logical_workspace("ws_orbit", Some("hm_owner"))],
            "checkouts": [{
                "workspace_id": "ws_orbit",
                "repo_root": "/repos/orbit",
                "orbit_dir": "/repos/orbit/.orbit",
                "role": "replica",
                "owner_machine_id": "hm_owner"
            }]
        }),
    );

    // Re-declaring the same replica role is idempotent and persists cleanly.
    let mut registry = load_registry_from(&path).expect("load replica");
    assign_checkout_role(
        &mut registry,
        "ws_orbit",
        WorkspaceCheckoutRole::Replica,
        Some("hm_owner"),
    )
    .expect("replica role");
    save_registry_to(&registry, &path).expect("save replica");

    // Declaring owner role on this non-owner machine is a contradiction that
    // fails at save time and leaves the previous file byte-valid.
    let before = fs::read(&path).expect("read before");
    let mut contradictory = load_registry_from(&path).expect("reload");
    assign_checkout_role(
        &mut contradictory,
        "ws_orbit",
        WorkspaceCheckoutRole::Owner,
        None,
    )
    .expect("in-memory mutation is permitted");
    let error = save_registry_to(&contradictory, &path)
        .expect_err("owner role on a non-owner machine must fail")
        .to_string();
    assert!(error.contains("owner"), "unexpected: {error}");
    assert_eq!(
        fs::read(&path).expect("read after"),
        before,
        "rejected save must leave the registry file byte-identical"
    );
}
