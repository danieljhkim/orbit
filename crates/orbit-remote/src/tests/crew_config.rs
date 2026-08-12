//! Owner-local crew discovery and validation [ORB-10729].
//!
//! Every case here runs with an empty `workspace_execution_profiles` table:
//! v1 publishes no execution profile ([ADR-0358]), so crew answers must come
//! from the owner machine's own config or not at all.

use std::path::Path;
use std::process::Command;

use chrono::Utc;
use orbit_common::types::{Workspace, WorkspaceCheckout, WorkspaceRegistry, WorkspaceStatus};
use orbit_store::sqlite::task_registry::{WorkspaceConfig, write_workspace_config};

use crate::OwnerLocalCrews;

const GLOBAL_CONFIG: &str = "\
[crews.sol]
model = \"gpt-global\"
provider = \"codex\"
backend = \"cli\"

[crews.luna]
model = \"claude-test\"
provider = \"claude\"
backend = \"cli\"

[workflow]
default_crew = \"sol\"
";

fn write_identity(global_root: &Path) {
    std::fs::write(
        global_root.join("host.toml"),
        "schema_version = 2\nmachine_id = \"hm_owner\"\nhost_id = \"owner\"\ntask_prefix = \"ORB\"\n",
    )
    .expect("host identity");
}

fn workspace(owner_machine_id: Option<&str>) -> Workspace {
    Workspace {
        id: "ws_alpha".to_string(),
        name: "Alpha".to_string(),
        owner_machine_id: owner_machine_id.map(ToOwned::to_owned),
        git_remote: None,
        ship_mode: None,
        base_branch: "agent-main".to_string(),
        status: WorkspaceStatus::Active,
        created_at: Utc::now(),
        updated_at: Utc::now(),
    }
}

/// A global root with an owner identity, a global crew config, and one
/// registered owner checkout carrying `workspace_config`.
fn owner_fixture(root: &Path, workspace_config: Option<&str>) -> (Workspace, WorkspaceCheckout) {
    let global_root = root.join("global");
    std::fs::create_dir_all(&global_root).expect("global root");
    write_identity(&global_root);
    std::fs::write(global_root.join("config.toml"), GLOBAL_CONFIG).expect("global crew config");

    let repo_root = root.join("alpha");
    let initialized = Command::new("git")
        .args(["init", "--quiet"])
        .current_dir(root)
        .arg(&repo_root)
        .status()
        .expect("initialize workspace repository");
    assert!(initialized.success(), "git init failed");
    let orbit_dir = repo_root.join(".orbit");
    std::fs::create_dir_all(&orbit_dir).expect("orbit directory");
    write_workspace_config(
        &orbit_dir,
        &WorkspaceConfig {
            schema_version: 1,
            workspace_id: "ws_alpha".to_string(),
        },
    )
    .expect("workspace identity");
    if let Some(config) = workspace_config {
        std::fs::write(orbit_dir.join("config.toml"), config).expect("workspace crew config");
    }

    let workspace = workspace(Some("hm_owner"));
    let checkout = WorkspaceCheckout::owner("ws_alpha".to_string(), repo_root, orbit_dir);
    crate::workspace_registry::save_registry_to(
        &WorkspaceRegistry {
            workspaces: vec![workspace.clone()],
            checkouts: vec![checkout.clone()],
            ..Default::default()
        },
        &crate::workspace_registry::registry_path_for(&global_root),
    )
    .expect("workspace registry");
    (workspace, checkout)
}

/// The coordination store this machine would publish a profile into, and the
/// number of rows actually in the dormant projection table.
fn published_profile_rows(global_root: &Path) -> i64 {
    crate::registry_snapshot_at(global_root).expect("initialize global store");
    let path = orbit_core::config::resolved_audit_db_path(global_root, global_root)
        .expect("global store path");
    rusqlite::Connection::open(path)
        .expect("global store")
        .query_row(
            "SELECT COUNT(*) FROM workspace_execution_profiles",
            [],
            |row| row.get(0),
        )
        .expect("profile row count")
}

#[test]
fn crew_discovery_layers_workspace_config_over_global_without_any_published_profile() {
    let root = tempfile::tempdir().expect("test root");
    // The workspace overrides one field of one crew; layering must keep the
    // rest of that crew and the rest of the registry.
    owner_fixture(
        root.path(),
        Some("[crews.sol]\nmodel = \"gpt-workspace\"\n"),
    );
    let global_root = root.path().join("global");
    assert_eq!(
        published_profile_rows(&global_root),
        0,
        "the fixture must publish no execution profile"
    );

    let discovery = OwnerLocalCrews::new(global_root)
        .crew_discovery("ws_alpha")
        .expect("crew discovery");

    assert_eq!(discovery.workspace_id, "ws_alpha");
    assert_eq!(discovery.owner_machine_id.as_deref(), Some("hm_owner"));
    assert_eq!(discovery.default_crew.as_deref(), Some("sol"));
    assert_eq!(
        discovery
            .crews
            .iter()
            .map(|crew| crew.name.as_str())
            .collect::<Vec<_>>(),
        ["luna", "sol"],
        "crews are returned sorted by name"
    );
    let sol = discovery
        .crews
        .iter()
        .find(|crew| crew.name == "sol")
        .expect("sol crew");
    assert_eq!(sol.model, "gpt-workspace", "workspace override wins");
    assert_eq!(sol.provider, "codex", "unoverridden crew fields survive");
    assert_eq!(sol.backend, "cli");

    // The sanitized projection carries no digest, path, or profile payload.
    let serialized = serde_json::to_string(&discovery).expect("serialize discovery");
    for forbidden in [
        "config_digest",
        "ship_closure_digest",
        "freshness",
        "generation",
        "\"root\"",
    ] {
        assert!(
            !serialized.contains(forbidden),
            "crew discovery leaked {forbidden}: {serialized}"
        );
    }
}

#[test]
fn checkoutless_workspace_reads_the_machine_global_crew_config() {
    let root = tempfile::tempdir().expect("test root");
    let global_root = root.path().join("global");
    std::fs::create_dir_all(&global_root).expect("global root");
    write_identity(&global_root);
    std::fs::write(global_root.join("config.toml"), GLOBAL_CONFIG).expect("global crew config");
    crate::workspace_registry::save_registry_to(
        &WorkspaceRegistry {
            workspaces: vec![workspace(Some("hm_owner"))],
            ..Default::default()
        },
        &crate::workspace_registry::registry_path_for(&global_root),
    )
    .expect("workspace registry");

    let discovery = OwnerLocalCrews::new(global_root)
        .crew_discovery("ws_alpha")
        .expect("crew discovery");

    assert_eq!(discovery.default_crew.as_deref(), Some("sol"));
    let sol = discovery
        .crews
        .iter()
        .find(|crew| crew.name == "sol")
        .expect("sol crew");
    assert_eq!(
        sol.model, "gpt-global",
        "a workspace with no checkout has only the global crew config"
    );
}

#[test]
fn task_crew_validation_canonicalizes_a_known_alias_and_names_the_owner_on_an_unknown_one() {
    let root = tempfile::tempdir().expect("test root");
    owner_fixture(root.path(), None);
    let global_root = root.path().join("global");
    let crews = OwnerLocalCrews::new(global_root.clone());

    assert_eq!(
        crews
            .validate_task_crew("ws_alpha", "  sol  ")
            .expect("padded known crew resolves"),
        "sol"
    );

    let error = crews
        .validate_task_crew("ws_alpha", "ghost")
        .expect_err("unknown crew is refused")
        .to_string();
    assert!(error.contains("ghost"), "{error}");
    assert!(
        error.contains("ws_alpha") && error.contains("hm_owner"),
        "the refusal must name the workspace and its owning machine: {error}"
    );
    assert!(
        error.contains("luna") && error.contains("sol"),
        "the refusal must stay actionable by listing the configured crews: {error}"
    );
    assert_eq!(
        published_profile_rows(&global_root),
        0,
        "validation must not depend on a published profile"
    );
}

/// Workflow preflight resolves the crew it will dispatch from the same owner-
/// local config, through the workspace runtime, with no projection row present.
#[test]
fn workflow_preflight_resolves_crew_from_owner_local_config() {
    let root = tempfile::tempdir().expect("test root");
    let (workspace, checkout) = owner_fixture(
        root.path(),
        Some("[crews.sol]\nmodel = \"gpt-workspace\"\n"),
    );
    let global_root = root.path().join("global");
    assert_eq!(published_profile_rows(&global_root), 0);

    let runtime = crate::runtime::RemoteRuntimeFactory::open_registered_checkout(
        &global_root,
        &workspace,
        &checkout,
    )
    .expect("workspace runtime");

    runtime
        .validate_crew_name(Some("luna"))
        .expect("a configured crew validates");
    let error = runtime
        .validate_crew_name(Some("ghost"))
        .expect_err("an unconfigured crew is refused")
        .to_string();
    assert!(error.contains("ghost"), "{error}");

    // The task's own crew wins over the configured default, and both come from
    // the layered config the owner machine holds.
    let resolved = runtime
        .resolve_crew_for_task(None, Some("luna"))
        .expect("task crew resolves");
    assert_eq!(resolved.name, "luna");
    let default = runtime
        .resolve_crew_for_task(None, None)
        .expect("default crew resolves");
    assert_eq!(default.name, "sol");
    assert_eq!(default.assignment.model, "gpt-workspace");
}
