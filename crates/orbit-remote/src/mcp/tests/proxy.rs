//! Tunnelled stdio proxy mode tests [ORB-10710].

use std::path::{Path, PathBuf};

use chrono::Utc;
use orbit_common::types::{
    McpCapability, Workspace, WorkspaceCheckout, WorkspaceCheckoutRole, WorkspaceRegistry,
    WorkspaceStatus,
};

use super::super::proxy::{
    DEFAULT_REMOTE_MCP_PORT, RemoteProxyArgs, local_checkout_evidence, owned_checkout_refusal,
    remote_listen_command, tunnel_spec,
};

fn args(ssh_host: &str, capability: Option<McpCapability>) -> RemoteProxyArgs {
    RemoteProxyArgs {
        ssh_host: ssh_host.to_string(),
        remote_port: DEFAULT_REMOTE_MCP_PORT,
        local_port: None,
        capability,
    }
}

fn workspace(id: &str) -> Workspace {
    Workspace {
        id: id.to_string(),
        name: id.to_string(),
        owner_machine_id: None,
        git_remote: None,
        ship_mode: None,
        base_branch: "agent-main".to_string(),
        status: WorkspaceStatus::Active,
        created_at: Utc::now(),
        updated_at: Utc::now(),
    }
}

fn checkout(id: &str, repo_root: PathBuf) -> WorkspaceCheckout {
    WorkspaceCheckout {
        workspace_id: id.to_string(),
        orbit_dir: repo_root.join(".orbit"),
        repo_root,
        role: Some(WorkspaceCheckoutRole::Owner),
        owner_machine_id: None,
        path_overrides: Vec::new(),
    }
}

/// Lay down what a `git clone` of a repository that versions its Orbit
/// workspace delivers: an `.orbit/` directory holding the committed partitions
/// and `config.toml`, and nothing machine-local. This is the read mirror
/// [ADR-0360] admits — the shape is identical to a working checkout's, which is
/// exactly why the filesystem cannot be the discriminator.
fn read_mirror(root: &Path) {
    let orbit_dir = root.join(".orbit");
    for partition in ["learnings", "auto_tasks", "resources", "routines"] {
        std::fs::create_dir_all(orbit_dir.join(partition)).expect("mirror partition");
    }
    std::fs::write(orbit_dir.join("config.toml"), "[workflow]\n").expect("mirror config");
}

// ── the checkout guard ────────────────────────────────────────────────────

#[test]
fn a_client_with_no_checkout_is_admitted() {
    let evidence = local_checkout_evidence(&WorkspaceRegistry::default(), None);
    assert!(
        evidence.is_none(),
        "a machine with an empty registry and no discoverable workspace is the client class \
         this mode exists for: {evidence:?}"
    );
}

#[test]
fn a_registered_checkout_that_exists_on_disk_refuses() {
    // The load-bearing case: a spoke registering this mode would otherwise
    // receive another machine's branch state as its own.
    let repo = tempfile::tempdir().expect("repo root");
    let registry = WorkspaceRegistry {
        workspaces: vec![workspace("ws_local")],
        checkouts: vec![checkout("ws_local", repo.path().to_path_buf())],
        ..Default::default()
    };

    let evidence = local_checkout_evidence(&registry, None).expect("a local checkout must refuse");
    assert!(
        evidence.contains("ws_local"),
        "the refusal must name which checkout it found: {evidence}"
    );
    assert!(
        evidence.contains(&repo.path().display().to_string()),
        "the refusal must name where it found it: {evidence}"
    );
}

#[test]
fn a_registered_checkout_whose_root_is_gone_is_stale_not_evidence() {
    // A registry row pointing at a deleted tree describes history, not a
    // checkout. Refusing on it would strand a genuinely checkoutless client.
    let registry = WorkspaceRegistry {
        workspaces: vec![workspace("ws_deleted")],
        checkouts: vec![checkout(
            "ws_deleted",
            Path::new("/nonexistent/orbit/checkout").to_path_buf(),
        )],
        ..Default::default()
    };

    assert!(local_checkout_evidence(&registry, None).is_none());
}

#[test]
fn a_checkoutless_catalog_entry_alone_is_not_evidence() {
    // A logical workspace with no machine-local checkout binding is exactly
    // what an off-box orchestrator's catalog looks like.
    let registry = WorkspaceRegistry {
        workspaces: vec![workspace("ws_remote_only")],
        ..Default::default()
    };

    assert!(local_checkout_evidence(&registry, None).is_none());
}

#[test]
fn a_read_mirror_clone_is_admitted() {
    // The client class this mode exists for [ORB-10761]. A repository that
    // versions its `.orbit/` delivers the whole directory through `git clone`,
    // so an unregistered one describes a read mirror, not ownership. Refusing
    // on it would lock every mirror-carrying orchestrator out of the mode.
    let mirror = tempfile::tempdir().expect("mirror root");
    read_mirror(mirror.path());

    let evidence = local_checkout_evidence(&WorkspaceRegistry::default(), Some(mirror.path()));
    assert!(
        evidence.is_none(),
        "a tracked `.orbit/` this machine's registry does not bind is not ownership: {evidence:?}"
    );
}

#[test]
fn the_same_shape_refuses_once_the_registry_binds_it() {
    // The other direction, over an identical filesystem: what changes the
    // verdict is the machine-local registry entry, nothing on disk.
    let repo = tempfile::tempdir().expect("repo root");
    read_mirror(repo.path());
    let registry = WorkspaceRegistry {
        workspaces: vec![workspace("ws_owned")],
        checkouts: vec![checkout("ws_owned", repo.path().to_path_buf())],
        ..Default::default()
    };

    let evidence = local_checkout_evidence(&registry, Some(repo.path()))
        .expect("an owned checkout must refuse");
    assert!(
        evidence.contains("ws_owned"),
        "the refusal must name the owning workspace: {evidence}"
    );
    assert!(
        evidence.contains(&repo.path().display().to_string()),
        "the refusal must name where it is checked out: {evidence}"
    );
}

#[test]
fn an_override_path_binding_the_cwd_is_ownership() {
    // A checkout can claim a directory outside its `repo_root` through an
    // explicit override; standing in one is still standing in a workspace this
    // machine coordinates.
    let elsewhere = tempfile::tempdir().expect("override root");
    let mut bound = checkout("ws_override", Path::new("/nonexistent/repo").to_path_buf());
    bound.path_overrides = vec![elsewhere.path().to_path_buf()];
    let registry = WorkspaceRegistry {
        workspaces: vec![workspace("ws_override")],
        checkouts: vec![bound],
        ..Default::default()
    };

    let evidence = local_checkout_evidence(&registry, Some(elsewhere.path()))
        .expect("a registered override path must refuse");
    assert!(
        evidence.contains("ws_override"),
        "the refusal must name the owning workspace: {evidence}"
    );
    assert!(
        evidence.contains(&elsewhere.path().display().to_string()),
        "the refusal must name the registered path that claims the cwd, not an absent \
         repository root: {evidence}"
    );
}

#[test]
fn the_refusal_names_the_evidence_and_the_local_broker() {
    // An operator who hits this needs both halves: what disqualified the
    // machine, and the surface to register in its place.
    let refusal =
        owned_checkout_refusal("my-box", "workspace 'ws_owned' is checked out at /srv/ws")
            .to_string();
    assert!(refusal.contains("ws_owned"), "{refusal}");
    assert!(refusal.contains("/srv/ws"), "{refusal}");
    assert!(refusal.contains("`orbit mcp serve`"), "{refusal}");
    assert!(refusal.contains("my-box"), "{refusal}");
}

// ── the remote listener command ───────────────────────────────────────────

#[test]
fn remote_command_binds_loopback() {
    let command = remote_listen_command(7879, None);
    assert_eq!(command, "orbit mcp serve --listen 127.0.0.1:7879");
}

#[test]
fn remote_command_passes_the_starting_capability() {
    let command = remote_listen_command(9100, Some(McpCapability::Operator));
    assert_eq!(
        command,
        "orbit mcp serve --listen 127.0.0.1:9100 --capabilities 'operator'"
    );
}

#[test]
fn remote_command_never_carries_hub_or_placement_flags() {
    // This is not a hub link and must not acquire hub-link responsibilities.
    for capability in [
        None,
        Some(McpCapability::Agent),
        Some(McpCapability::Operator),
        Some(McpCapability::Runner),
    ] {
        let command = remote_listen_command(7879, capability);
        assert!(!command.contains("--hub"), "{command}");
        assert!(!command.contains("--root"), "{command}");
    }
}

// ── tunnel wiring ─────────────────────────────────────────────────────────

#[test]
fn tunnel_spec_forwards_to_the_remote_listener_port() {
    let mut cfg = args("my-box", Some(McpCapability::Agent));
    cfg.remote_port = 9100;
    let spec = tunnel_spec(&cfg, 40000);

    assert_eq!(spec.ssh_host, "my-box");
    assert_eq!(spec.local_port, 40000);
    assert_eq!(spec.remote_port, 9100);
    assert_eq!(
        spec.remote_command,
        "orbit mcp serve --listen 127.0.0.1:9100 --capabilities 'agent'"
    );
    assert_eq!(spec.remote_description, "orbit mcp serve --listen");
}

#[test]
fn tunnel_spec_waits_longer_for_a_spawned_listener_than_for_an_attach_probe() {
    // Attaching to an already-running listener must not pay a remote-boot
    // timeout, and spawning one must not be declared dead before it binds.
    let spec = tunnel_spec(&args("my-box", None), 40000);
    assert!(spec.ready_timeout > spec.attach_timeout);
}

#[test]
fn the_default_remote_port_is_distinct_from_the_dashboard_port() {
    // Both are loopback listeners reached the same way; colliding on one port
    // would make `orbit web connect` and this mode fight over a forward.
    assert_ne!(DEFAULT_REMOTE_MCP_PORT, 7878);
}
