use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use chrono::Utc;
use orbit_common::types::{
    HUB_KNOWLEDGE_ALLOCATION_SCHEMA_VERSION, HubKnowledgeAllocationRequestV1, KnowledgeIdKind,
    McpCapability, McpTransport, ToolSessionContext, Workspace, WorkspaceCheckout,
    WorkspaceCheckoutRole, WorkspaceRegistry, WorkspaceStatus,
};
use rusqlite::Connection;

use crate::{HubKnowledgeSequenceService, scan_registered_knowledge_inventories};

fn write_identity(root: &Path, mode: &str) {
    fs::write(
        root.join("host.toml"),
        format!(
            "schema_version = 1\nmachine_id = \"hm_hub\"\nhost_id = \"hub\"\nmode = \"{mode}\"\n"
        ),
    )
    .expect("host identity");
}

fn configure_hub(root: &Path, machine_id: &str) {
    let service = crate::host_registry_service_at(root).expect("host registry service");
    service
        .register_hub_identity(
            &crate::HostIdentity {
                schema_version: crate::HOST_IDENTITY_SCHEMA_VERSION,
                machine_id: machine_id.to_string(),
                host_id: "configured-hub".to_string(),
                mode: crate::HostMode::Hub,
            },
            BTreeSet::new(),
        )
        .expect("configured hub identity");
}

fn allocation_context(workspace_id: &str, mcp_call_id: &str) -> ToolSessionContext {
    ToolSessionContext {
        workspace: None,
        workspace_id: Some(workspace_id.to_string()),
        caller_machine_id: Some("hm_hub".to_string()),
        caller_host_id: Some("hub".to_string()),
        process_machine_id: Some("hm_hub".to_string()),
        process_host_id: Some("hub".to_string()),
        transport: Some(McpTransport::Local),
        effective_capabilities: BTreeSet::from([McpCapability::Agent]),
        origin_session_id: Some("knowledge-test".to_string()),
        mcp_call_id: Some(mcp_call_id.to_string()),
        leased_run: None,
    }
}

fn workspace(root: &Path, id: &str) -> (Workspace, WorkspaceCheckout) {
    let repo_root = root.join(id);
    let orbit_dir = repo_root.join(".orbit");
    fs::create_dir_all(&orbit_dir).expect("orbit dir");
    (
        Workspace {
            id: id.to_string(),
            name: id.to_string(),
            owner_machine_id: Some("hm_hub".to_string()),
            git_remote: None,
            ship_mode: None,
            base_branch: "agent-main".to_string(),
            status: WorkspaceStatus::Active,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        },
        WorkspaceCheckout {
            workspace_id: id.to_string(),
            repo_root,
            orbit_dir,
            role: Some(WorkspaceCheckoutRole::Owner),
            owner_machine_id: None,
            path_overrides: Vec::new(),
        },
    )
}

fn save_registry(root: &Path, entries: Vec<(Workspace, WorkspaceCheckout)>) {
    let (workspaces, checkouts): (Vec<_>, Vec<_>) = entries.into_iter().unzip();
    crate::workspace_registry::save_registry_to(
        &WorkspaceRegistry {
            workspaces,
            checkouts,
            ..WorkspaceRegistry::default()
        },
        &crate::workspace_registry::registry_path_for(root),
    )
    .expect("registry");
}

fn write_adr(orbit_dir: &Path, state: &str, id: &str) {
    let directory = orbit_dir.join("adrs").join(state).join(id);
    fs::create_dir_all(&directory).expect("adr directory");
    fs::write(directory.join("adr.yaml"), format!("id: {id}\n")).expect("adr yaml");
}

fn write_learning(orbit_dir: &Path, id: &str, status: &str) {
    let directory = orbit_dir.join("learnings").join(id);
    fs::create_dir_all(&directory).expect("learning directory");
    fs::write(
        directory.join("learning.yaml"),
        format!("id: {id}\nstatus: {status}\n"),
    )
    .expect("learning yaml");
}

fn write_allocations(orbit_dir: &Path) {
    let database = orbit_dir.join("state").join("semantic.db");
    fs::create_dir_all(database.parent().expect("database parent")).expect("state dir");
    let connection = Connection::open(database).expect("semantic db");
    connection
        .execute_batch(
            "CREATE TABLE id_allocations (
                 kind TEXT NOT NULL,
                 id TEXT NOT NULL,
                 status TEXT NOT NULL,
                 worktree_root TEXT NOT NULL,
                 body_path TEXT
             );
             INSERT INTO id_allocations VALUES
                 ('adr', 'ADR-0007', 'reserved', '/outside/hub', '/arbitrary/ADR-9999/body.md'),
                 ('learning', 'L-0008', 'merged', '/stale/worktree', '../outside.yaml'),
                 ('adr', 'ADR-0009', 'abandoned', '/deleted/worktree', NULL);",
        )
        .expect("allocation rows");
}

fn snapshot_files(root: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    fn visit(root: &Path, directory: &Path, files: &mut BTreeMap<PathBuf, Vec<u8>>) {
        let mut entries = fs::read_dir(directory)
            .expect("snapshot directory")
            .map(|entry| entry.expect("snapshot entry"))
            .collect::<Vec<_>>();
        entries.sort_by_key(|entry| entry.path());
        for entry in entries {
            let path = entry.path();
            let kind = entry.file_type().expect("snapshot file type");
            if kind.is_dir() {
                visit(root, &path, files);
            } else if kind.is_file() {
                files.insert(
                    path.strip_prefix(root)
                        .expect("relative snapshot")
                        .to_path_buf(),
                    fs::read(path).expect("snapshot file"),
                );
            }
        }
    }
    let mut files = BTreeMap::new();
    visit(root, root, &mut files);
    files
}

#[test]
fn scanner_covers_every_file_lifecycle_and_allocation_status_without_following_row_paths() {
    let root = tempfile::tempdir().expect("root");
    write_identity(root.path(), "hub");
    let (workspace_alpha, checkout_alpha) = workspace(root.path(), "ws_alpha");
    let (workspace_beta, checkout_beta) = workspace(root.path(), "ws_beta");
    let orbit_dir = checkout_alpha.orbit_dir.clone();
    let beta_orbit_dir = checkout_beta.orbit_dir.clone();
    save_registry(
        root.path(),
        vec![
            (workspace_alpha, checkout_alpha),
            (workspace_beta, checkout_beta),
        ],
    );

    for (state, id) in [
        ("proposed", "ADR-0001"),
        ("accepted", "ADR-0002"),
        ("superseded", "ADR-0003"),
        ("deleted", "ADR-0004"),
    ] {
        write_adr(&orbit_dir, state, id);
    }
    write_learning(&orbit_dir, "L-0005", "active");
    write_learning(&orbit_dir, "L-0006", "superseded");
    write_allocations(&orbit_dir);
    write_adr(&beta_orbit_dir, "accepted", "ADR-0010");
    write_learning(&beta_orbit_dir, "L-0011", "active");

    let before = snapshot_files(root.path());
    let inventories = scan_registered_knowledge_inventories(root.path()).expect("scan");
    assert_eq!(
        snapshot_files(root.path()),
        before,
        "scanner mutated source bytes"
    );
    assert_eq!(inventories.len(), 2);
    let alpha = inventories
        .iter()
        .find(|inventory| inventory.workspace_id == "ws_alpha")
        .expect("alpha inventory");
    let actual = alpha
        .ids
        .iter()
        .map(|record| ((record.kind, record.id.clone()), record.evidence.clone()))
        .collect::<BTreeMap<_, _>>();
    for (kind, id, source) in [
        (KnowledgeIdKind::Adr, "ADR-0001", "adr-file:proposed"),
        (KnowledgeIdKind::Adr, "ADR-0002", "adr-file:accepted"),
        (KnowledgeIdKind::Adr, "ADR-0003", "adr-file:superseded"),
        (KnowledgeIdKind::Adr, "ADR-0004", "adr-file:deleted"),
        (KnowledgeIdKind::Learning, "L-0005", "learning-file:active"),
        (
            KnowledgeIdKind::Learning,
            "L-0006",
            "learning-file:superseded",
        ),
        (KnowledgeIdKind::Adr, "ADR-0007", "allocation:reserved"),
        (KnowledgeIdKind::Learning, "L-0008", "allocation:merged"),
        (KnowledgeIdKind::Adr, "ADR-0009", "allocation:abandoned"),
    ] {
        assert_eq!(
            actual.get(&(kind, id.to_string())),
            Some(&BTreeSet::from([source.to_string()])),
            "{kind:?} {id}"
        );
    }
    assert_eq!(actual.len(), 9);
    let beta = inventories
        .iter()
        .find(|inventory| inventory.workspace_id == "ws_beta")
        .expect("beta inventory");
    assert_eq!(
        beta.ids
            .iter()
            .map(|record| record.id.as_str())
            .collect::<BTreeSet<_>>(),
        BTreeSet::from(["ADR-0010", "L-0011"])
    );
}

#[test]
fn scanner_ignores_derived_global_indexes_and_accepts_missing_semantic_db_after_file_scan() {
    let root = tempfile::tempdir().expect("root");
    write_identity(root.path(), "hub");
    let (workspace, checkout) = workspace(root.path(), "ws_alpha");
    let orbit_dir = checkout.orbit_dir.clone();
    save_registry(root.path(), vec![(workspace, checkout)]);
    write_adr(&orbit_dir, "accepted", "ADR-0042");

    // A malformed global audit DB would have blocked the old derived-index
    // scanner. It is not an activation source and must remain irrelevant.
    fs::write(root.path().join("orbit.db"), b"stale derived cache").expect("stale cache");
    let inventories = scan_registered_knowledge_inventories(root.path()).expect("scan files");
    assert_eq!(inventories[0].ids.len(), 1);
    assert_eq!(inventories[0].ids[0].id, "ADR-0042");
}

#[test]
fn scanner_fails_on_present_but_unreadable_or_non_file_semantic_database() {
    for directory_instead_of_file in [false, true] {
        let root = tempfile::tempdir().expect("root");
        write_identity(root.path(), "hub");
        let (workspace, checkout) = workspace(root.path(), "ws_alpha");
        let database = checkout.orbit_dir.join("state").join("semantic.db");
        fs::create_dir_all(database.parent().expect("parent")).expect("state dir");
        if directory_instead_of_file {
            fs::create_dir(&database).expect("database directory");
        } else {
            fs::write(&database, b"not a sqlite database").expect("malformed database");
        }
        save_registry(root.path(), vec![(workspace, checkout)]);
        let error = scan_registered_knowledge_inventories(root.path())
            .expect_err("present invalid database must fail")
            .to_string();
        assert!(error.contains("semantic.db"), "{error}");
    }
}

#[test]
fn scanner_names_missing_hub_local_source_and_leaves_dormant_store_unchanged() {
    let root = tempfile::tempdir().expect("root");
    write_identity(root.path(), "hub");
    crate::workspace_registry::save_registry_to(
        &WorkspaceRegistry {
            workspaces: vec![Workspace {
                id: "ws_checkoutless".to_string(),
                name: "Checkoutless".to_string(),
                owner_machine_id: Some("hm_owner".to_string()),
                git_remote: None,
                ship_mode: None,
                base_branch: "agent-main".to_string(),
                status: WorkspaceStatus::Active,
                created_at: Utc::now(),
                updated_at: Utc::now(),
            }],
            ..WorkspaceRegistry::default()
        },
        &crate::workspace_registry::registry_path_for(root.path()),
    )
    .expect("checkoutless registry");
    let store = crate::remote_store_at(root.path()).expect("dormant store");
    let before = snapshot_files(root.path());

    let error = scan_registered_knowledge_inventories(root.path())
        .expect_err("missing local source must block activation scan")
        .to_string();
    assert!(error.contains("ws_checkoutless"), "{error}");
    assert!(error.contains("hub-local checkout"), "{error}");
    assert_eq!(
        snapshot_files(root.path()),
        before,
        "failed scan mutated bytes"
    );
    let state = store.knowledge_allocator_state().expect("allocator state");
    assert_eq!(state.activation_generation, 0);
    assert!(state.activated_at.is_none());
}

#[test]
fn scanner_rejects_stale_primary_orbit_dir_without_mutating_dormant_store() {
    let root = tempfile::tempdir().expect("root");
    write_identity(root.path(), "hub");
    let (mut workspace, checkout) = workspace(root.path(), "ws_stale");
    workspace.status = WorkspaceStatus::Invalid;
    fs::remove_dir(&checkout.orbit_dir).expect("remove empty orbit dir");
    let stale_path = checkout.orbit_dir.clone();
    save_registry(root.path(), vec![(workspace, checkout)]);
    let store = crate::remote_store_at(root.path()).expect("dormant store");
    let before = snapshot_files(root.path());

    let error = scan_registered_knowledge_inventories(root.path())
        .expect_err("stale primary binding must block activation scan")
        .to_string();
    for expected in [
        "ws_stale",
        stale_path.to_string_lossy().as_ref(),
        "orbit_dir",
        "stale",
        "unresolved migration source",
    ] {
        assert!(error.contains(expected), "missing '{expected}' in {error}");
    }
    assert_eq!(
        snapshot_files(root.path()),
        before,
        "failed scan mutated bytes"
    );
    let state = store.knowledge_allocator_state().expect("allocator state");
    assert_eq!(state.activation_generation, 0);
    assert!(state.activated_at.is_none());
}

#[test]
fn inactive_registered_workspace_contributes_global_max_but_cannot_allocate() {
    let root = tempfile::tempdir().expect("root");
    write_identity(root.path(), "hub");
    configure_hub(root.path(), "hm_hub");
    let (active, active_checkout) = workspace(root.path(), "ws_active");
    let (mut inactive, inactive_checkout) = workspace(root.path(), "ws_inactive");
    inactive.status = WorkspaceStatus::Invalid;
    write_adr(&inactive_checkout.orbit_dir, "accepted", "ADR-0099");
    save_registry(
        root.path(),
        vec![(active, active_checkout), (inactive, inactive_checkout)],
    );

    let inventories =
        scan_registered_knowledge_inventories(root.path()).expect("scan all registered");
    assert_eq!(inventories.len(), 2);
    let service = HubKnowledgeSequenceService::at(root.path()).expect("authoritative service");
    let state = service.activate(inventories).expect("activate");
    assert_eq!(state.adr_next_sequence, 100);
    let allocation = service
        .allocate(
            &HubKnowledgeAllocationRequestV1 {
                schema_version: HUB_KNOWLEDGE_ALLOCATION_SCHEMA_VERSION,
                workspace_id: "ws_active".to_string(),
                kind: KnowledgeIdKind::Adr,
                model: None,
            },
            &allocation_context("ws_active", "mcall-inactive-max"),
        )
        .expect("allocate above inactive max");
    assert_eq!(allocation.id, "ADR-0100");
    let error = service
        .allocate(
            &HubKnowledgeAllocationRequestV1 {
                schema_version: HUB_KNOWLEDGE_ALLOCATION_SCHEMA_VERSION,
                workspace_id: "ws_inactive".to_string(),
                kind: KnowledgeIdKind::Adr,
                model: None,
            },
            &allocation_context("ws_inactive", "mcall-inactive-denied"),
        )
        .expect_err("inactive workspace must not allocate")
        .to_string();
    assert!(error.contains("inactive"), "{error}");
}

#[test]
fn production_service_rejects_unstamped_and_shadow_hub_stores() {
    let unstamped = tempfile::tempdir().expect("unstamped root");
    write_identity(unstamped.path(), "hub");
    let error = match HubKnowledgeSequenceService::at(unstamped.path()) {
        Ok(_) => panic!("unstamped store must not construct allocation authority"),
        Err(error) => error.to_string(),
    };
    assert!(error.contains("no configured hub_machine_id"), "{error}");
    let state = crate::remote_store_at(unstamped.path())
        .expect("unstamped store")
        .knowledge_allocator_state()
        .expect("unstamped state");
    assert_eq!(state.activation_generation, 0);
    assert!(state.activated_at.is_none());

    let shadow = tempfile::tempdir().expect("shadow root");
    write_identity(shadow.path(), "hub");
    configure_hub(shadow.path(), "hm_other");
    let error = match HubKnowledgeSequenceService::at(shadow.path()) {
        Ok(_) => panic!("shadow store must not construct allocation authority"),
        Err(error) => error.to_string(),
    };
    assert!(error.contains("hm_hub"), "{error}");
    assert!(error.contains("hm_other"), "{error}");
    assert!(error.contains("shadow"), "{error}");
    let state = crate::remote_store_at(shadow.path())
        .expect("shadow store")
        .knowledge_allocator_state()
        .expect("shadow state");
    assert_eq!(state.activation_generation, 0);
    assert!(state.activated_at.is_none());
}

#[test]
fn production_service_rejects_standalone_before_opening_allocation_authority() {
    let root = tempfile::tempdir().expect("root");
    write_identity(root.path(), "standalone");
    let error = match HubKnowledgeSequenceService::at(root.path()) {
        Ok(_) => panic!("standalone must not construct hub allocation authority"),
        Err(error) => error.to_string(),
    };
    assert!(error.contains("hub-local"), "{error}");
    assert!(error.contains("standalone"), "{error}");
    assert!(!root.path().join("orbit.db").exists());
}
