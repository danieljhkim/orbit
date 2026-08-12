use std::collections::BTreeSet;

use chrono::Utc;
use orbit_common::types::{
    AuditEventStatus, McpCapability, McpToolPlacement, McpTransport, ToolSessionContext, Workspace,
    WorkspaceRegistry, WorkspaceStatus,
};
use orbit_core::runtime::HubCoordinationExecutor;
use orbit_mcp::McpHost;
use rusqlite::Connection;
use serde_json::{Value, json};
use tempfile::TempDir;

use super::super::config::load_trusted_mcp_config;
use super::super::owner::OwnerMcpHost;

/// Write a complete, current-schema host identity.
///
/// ORB-10727: this used to write `schema_version = 1` with a `mode`. Schema 2
/// has no machine-level mode to declare (ADR-0358), so the owner endpoint reads
/// only the stable `machine_id`.
fn write_identity(root: &TempDir, machine_id: &str) {
    write_identity_at(root.path(), machine_id, "test-host");
}

fn write_identity_at(root: &std::path::Path, machine_id: &str, host_id: &str) {
    std::fs::write(
        root.join("host.toml"),
        format!(
            "schema_version = 2\nmachine_id = \"{machine_id}\"\nhost_id = \"{host_id}\"\ntask_prefix = \"ORB\"\n"
        ),
    )
    .expect("host identity");
}

fn initialize_store(root: &TempDir) -> std::path::PathBuf {
    crate::registry_snapshot_at(root.path()).expect("initialize global store");
    orbit_core::config::resolved_audit_db_path(root.path(), root.path()).expect("global store path")
}

fn stamp_store(root: &TempDir, machine_id: &str) -> std::path::PathBuf {
    let path = initialize_store(root);
    let connection = Connection::open(&path).expect("global store");
    connection
        .execute(
            "UPDATE hub_registry_metadata
             SET hub_machine_id = ?1, registry_revision = registry_revision + 1
             WHERE id = 0",
            [machine_id],
        )
        .expect("stamp hub store");
    path
}

fn add_checkoutless_workspace(root: &TempDir, workspace_id: &str) {
    crate::workspace_registry::save_registry_to(
        &WorkspaceRegistry {
            workspaces: vec![Workspace {
                id: workspace_id.to_string(),
                name: "Checkoutless".to_string(),
                owner_machine_id: Some("hm_owner".to_string()),
                git_remote: None,
                ship_mode: None,
                base_branch: "agent-main".to_string(),
                status: WorkspaceStatus::Active,
                created_at: Utc::now(),
                updated_at: Utc::now(),
            }],
            ..Default::default()
        },
        &crate::workspace_registry::registry_path_for(root.path()),
    )
    .expect("workspace registry");
    HubCoordinationExecutor::register_workspace(root.path(), workspace_id, "checkoutless")
        .expect("coordination workspace");
}

fn context(capability: McpCapability, call_id: &str) -> ToolSessionContext {
    let mut context = ToolSessionContext::trusted_local(
        None,
        Some("hm_owner".to_string()),
        Some("test-host".to_string()),
    );
    context.effective_capabilities = BTreeSet::from([capability]);
    context.mcp_call_id = Some(call_id.to_string());
    context
}

fn remote_context(capability: McpCapability, call_id: &str) -> ToolSessionContext {
    ToolSessionContext {
        caller_machine_id: Some("hm_client".to_string()),
        caller_host_id: Some("spoke".to_string()),
        transport: Some(McpTransport::SshMcp),
        effective_capabilities: BTreeSet::from([capability]),
        origin_session_id: Some("remote-session".to_string()),
        mcp_call_id: Some(call_id.to_string()),
        ..ToolSessionContext::default()
    }
}

fn valid_config() -> &'static str {
    "[[owner]]\nmachine_id = \"hm_owner\"\ntransport = \"ssh\"\nhost = \"orbit-owner\"\nallowed_capabilities = [\"agent\", \"operator\"]\n"
}

#[test]
fn trusted_config_parses_zero_or_more_safe_owner_routes() {
    let root = tempfile::tempdir().expect("global root");

    // Zero routes is the valid default for a machine that owns what it uses.
    assert!(
        load_trusted_mcp_config(root.path())
            .expect("absent config")
            .routes()
            .next()
            .is_none()
    );

    std::fs::write(root.path().join("mcp.toml"), valid_config()).expect("mcp config");
    let config = load_trusted_mcp_config(root.path()).expect("trusted config");
    let routes = config.routes().collect::<Vec<_>>();
    assert_eq!(routes.len(), 1);
    assert_eq!(routes[0].machine_id, "hm_owner");
    assert_eq!(routes[0].host, "orbit-owner");
    assert_eq!(
        routes[0].allowed_capabilities,
        BTreeSet::from([McpCapability::Agent, McpCapability::Operator])
    );
}

/// Routes are per machine, not per workspace: a client holding replicas of
/// workspaces owned by several machines names each owner once, and lookup is by
/// the target `machine_id`.
#[test]
fn multiple_owner_entries_are_keyed_by_target_machine_id() {
    let root = tempfile::tempdir().expect("global root");
    std::fs::write(
        root.path().join("mcp.toml"),
        "[[owner]]\nmachine_id = \"hm_alpha\"\ntransport = \"ssh\"\nhost = \"alpha\"\nallowed_capabilities = [\"agent\"]\n\
         \n\
         [[owner]]\nmachine_id = \"hm_beta\"\ntransport = \"ssh\"\nhost = \"beta\"\nallowed_capabilities = [\"operator\"]\n",
    )
    .expect("mcp config");
    let config = load_trusted_mcp_config(root.path()).expect("trusted config");

    let alpha = config.owners.get("hm_alpha").expect("alpha route");
    assert_eq!(alpha.host, "alpha");
    assert_eq!(
        alpha.allowed_capabilities,
        BTreeSet::from([McpCapability::Agent])
    );

    let beta = config.owners.get("hm_beta").expect("beta route");
    assert_eq!(beta.host, "beta");
    assert_eq!(
        beta.allowed_capabilities,
        BTreeSet::from([McpCapability::Operator])
    );
    assert!(
        !beta.allowed_capabilities.contains(&McpCapability::Agent),
        "operator must not imply agent"
    );

    assert!(!config.owners.contains_key("hm_unlisted"));
    assert_eq!(
        config
            .routes()
            .map(|route| route.machine_id.as_str())
            .collect::<Vec<_>>(),
        ["hm_alpha", "hm_beta"],
        "routes enumerate in stable machine_id order"
    );
}

/// ORB-10727 decision: a legacy singular `[hub]` table fails closed with a
/// migration message rather than being auto-migrated. `[hub]` named a
/// machine-level coordination host; `[[owner]]` names the machine that owns a
/// workspace. The hub target need not own anything, so rewriting one into the
/// other could silently route coordination calls to a non-owner.
#[test]
fn legacy_hub_table_fails_closed_with_an_actionable_migration_message() {
    let root = tempfile::tempdir().expect("global root");
    std::fs::write(
        root.path().join("mcp.toml"),
        "[hub]\nmachine_id = \"hm_owner\"\ntransport = \"ssh\"\nhost = \"orbit-owner\"\nallowed_capabilities = [\"agent\"]\n",
    )
    .expect("mcp config");

    let error = load_trusted_mcp_config(root.path()).expect_err("legacy [hub] must fail closed");
    let message = error.to_string();
    assert!(
        message.contains("[hub]"),
        "names the withdrawn table: {message}"
    );
    assert!(
        message.contains("[[owner]]"),
        "names the replacement form: {message}"
    );
    assert!(
        message.contains("not migrated for you"),
        "states that nothing was guessed: {message}"
    );
    assert!(
        message.contains(&root.path().join("mcp.toml").display().to_string()),
        "names the file to edit: {message}"
    );
}

#[test]
fn trusted_config_fails_closed_on_unknown_duplicate_empty_and_unsupported_values() {
    let cases = [
        ("owner = \"elsewhere\"\n", "invalid type"),
        ("[owners.dk1]\nmachine_id = \"hm_owner\"\n", "unknown field"),
        (
            "[[owner]]\nmachine_id = \"hm_owner\"\ntransport = \"ssh\"\nhost = \"owner\"\nallowed_capabilities = [\"agent\"]\ncommand = \"orbit mcp serve --owner\"\n",
            "unknown field",
        ),
        (
            "[[owner]]\nmachine_id = \"hm_owner\"\ntransport = \"http\"\nhost = \"owner\"\nallowed_capabilities = [\"agent\"]\n",
            "unknown variant",
        ),
        (
            "[[owner]]\nmachine_id = \"not-a-machine\"\ntransport = \"ssh\"\nhost = \"owner\"\nallowed_capabilities = [\"agent\"]\n",
            "invalid owner machine_id",
        ),
        (
            "[[owner]]\nmachine_id = \"hm_owner\"\ntransport = \"ssh\"\nhost = \"owner\"\nallowed_capabilities = []\n",
            "must not be empty",
        ),
        (
            "[[owner]]\nmachine_id = \"hm_owner\"\ntransport = \"ssh\"\nhost = \"owner\"\nallowed_capabilities = [\"agent\", \"agent\"]\n",
            "repeats owner capability",
        ),
        (
            "[[owner]]\nmachine_id = \"hm_owner\"\ntransport = \"ssh\"\nhost = \"owner\"\nallowed_capabilities = [\"admin\"]\n",
            "unknown variant",
        ),
        (
            // `runner` still parses as a capability, so this is a bridge policy
            // refusal rather than a deserialization failure (ADR-0358).
            "[[owner]]\nmachine_id = \"hm_owner\"\ntransport = \"ssh\"\nhost = \"owner\"\nallowed_capabilities = [\"runner\"]\n",
            "the v1 bridge withdrew",
        ),
        (
            "[[owner]]\nmachine_id = \"hm_owner\"\ntransport = \"ssh\"\nhost = \"owner\"\n",
            "missing field",
        ),
        (
            "[[owner]]\nmachine_id = \"hm_owner\"\ntransport = \"ssh\"\nhost = \"a\"\nallowed_capabilities = [\"agent\"]\n\
             \n\
             [[owner]]\nmachine_id = \"hm_owner\"\ntransport = \"ssh\"\nhost = \"b\"\nallowed_capabilities = [\"agent\"]\n",
            "more than one [[owner]] entry",
        ),
    ];
    for (body, expected) in cases {
        let root = tempfile::tempdir().expect("global root");
        std::fs::write(root.path().join("mcp.toml"), body).expect("mcp config");
        let error = load_trusted_mcp_config(root.path()).expect_err("invalid config");
        assert!(
            error.to_string().contains(expected),
            "expected {expected:?}, got {error} for {body:?}"
        );
    }
}

#[test]
fn trusted_config_rejects_every_alias_that_could_change_ssh_argv_or_command() {
    for alias in [
        "-oProxyCommand=touch-pwned",
        "owner extra",
        "user@owner",
        "owner;command",
        "owner|command",
        "owner\ncommand",
        "/tmp/owner",
        "owner=target",
        "*.internal",
    ] {
        let root = tempfile::tempdir().expect("global root");
        let encoded = toml::Value::String(alias.to_string()).to_string();
        let body = format!(
            "[[owner]]\nmachine_id = \"hm_owner\"\ntransport = \"ssh\"\nhost = {encoded}\nallowed_capabilities = [\"agent\"]\n"
        );
        std::fs::write(root.path().join("mcp.toml"), body).expect("mcp config");
        let error = load_trusted_mcp_config(root.path()).expect_err("unsafe alias");
        assert!(
            error.to_string().contains("invalid owner host alias"),
            "unexpected error for {alias:?}: {error}"
        );
    }
}

#[test]
fn config_resolution_ignores_repo_and_other_root_decoys() {
    let root = tempfile::tempdir().expect("global root");
    let repo = tempfile::tempdir().expect("repo");
    let env_decoy = tempfile::tempdir().expect("environment decoy");
    std::fs::create_dir_all(repo.path().join(".orbit")).expect("repo orbit dir");
    std::fs::write(root.path().join("mcp.toml"), valid_config()).expect("global config");
    std::fs::write(
        repo.path().join(".orbit/mcp.toml"),
        "[[owner]]\nmachine_id = \"hm_repo\"\ntransport = \"ssh\"\nhost = \"repo\"\nallowed_capabilities = [\"agent\"]\n",
    )
    .expect("repo decoy");
    std::fs::write(
        env_decoy.path().join("mcp.toml"),
        "[[owner]]\nmachine_id = \"hm_env\"\ntransport = \"ssh\"\nhost = \"env\"\nallowed_capabilities = [\"operator\"]\n",
    )
    .expect("env decoy");

    // The loader has no cwd or environment input: only the explicit machine
    // global root can influence the result.
    let config = load_trusted_mcp_config(root.path()).expect("global config");
    assert_eq!(
        config
            .routes()
            .map(|route| route.machine_id.as_str())
            .collect::<Vec<_>>(),
        ["hm_owner"]
    );
}

/// ORB-10727 [ADR-0355]: the owner endpoint no longer requires `host.toml` mode
/// `hub`, because schema 2 has no machine-level coordination role to declare,
/// and no longer requires the store to be stamped, because the registration
/// that stamped it is withdrawn (ADR-0358). What survives from ORB-10268: the
/// startup machine identity must keep matching, and a store stamped by a
/// *different* machine is still refused as a shadow store.
#[test]
fn owner_host_admits_an_unstamped_store_but_refuses_a_changed_identity_or_shadow_stamp() {
    let root = tempfile::tempdir().expect("global root");
    write_identity(&root, "hm_owner");
    initialize_store(&root);
    let host = OwnerMcpHost::new(root.path().to_path_buf(), McpCapability::Agent)
        .expect("an unstamped store has nothing to contradict");
    host.list_mcp_tool_definitions()
        .expect("unstamped store serves");

    let shadow = tempfile::tempdir().expect("global root");
    write_identity(&shadow, "hm_owner");
    stamp_store(&shadow, "hm_other");
    let error = OwnerMcpHost::new(shadow.path().to_path_buf(), McpCapability::Agent)
        .expect_err("a store stamped by another machine is a shadow store");
    assert!(
        error.to_string().contains("shadow coordination store"),
        "unexpected error: {error}"
    );

    write_identity(&root, "hm_replaced");
    let error = host
        .list_mcp_tool_definitions()
        .expect_err("identity change must be refused");
    assert!(
        error.to_string().contains("owner MCP authority changed"),
        "unexpected error: {error}"
    );
}

#[test]
fn owner_listing_uses_one_canonical_placement_and_capability_predicate() {
    let root = tempfile::tempdir().expect("global root");
    write_identity(&root, "hm_owner");
    stamp_store(&root, "hm_owner");

    let agent = OwnerMcpHost::new(root.path().to_path_buf(), McpCapability::Agent).expect("agent");
    let agent_definitions = agent.list_mcp_tool_definitions().expect("agent tools");
    assert!(agent_definitions.iter().all(|definition| {
        definition.policy.placement() == McpToolPlacement::Owner
            && definition
                .policy
                .allowed_capabilities()
                .contains(&McpCapability::Agent)
    }));
    let agent_names = agent_definitions
        .iter()
        .map(|definition| definition.schema.name.as_str())
        .collect::<BTreeSet<_>>();
    assert!(agent_names.contains("orbit.task.add"));
    // `orbit.workspace.list` is `local-derived`, so the owner endpoint does not
    // advertise it at any capability: it answers from the caller's own registry.
    assert!(!agent_names.contains("orbit.workspace.list"));
    // Crew discovery is admitted for agent (unlike the operator-only registry
    // discovery tool), proving capability is by placement, not a hierarchy.
    assert!(agent_names.contains("orbit.crew.list"));
    // `orbit.adr.*` left the advertised surface with the ADR store (ORB-10726):
    // ADRs are git-committed entries in each feature's `4_decisions.md`, so the
    // owner endpoint has no ADR tool to place.
    assert!(
        !agent_names
            .iter()
            .any(|name| name.starts_with("orbit.adr."))
    );
    assert!(
        !agent_names
            .iter()
            .any(|name| name.starts_with("orbit.graph."))
    );

    let operator =
        OwnerMcpHost::new(root.path().to_path_buf(), McpCapability::Operator).expect("operator");
    let operator_names = operator
        .list_mcp_tool_definitions()
        .expect("operator tools")
        .into_iter()
        .map(|definition| definition.schema.name)
        .collect::<BTreeSet<_>>();
    assert!(
        !operator_names.contains("orbit.workspace.list"),
        "local-derived discovery is never owner-advertised"
    );
    assert!(operator_names.contains("orbit.crew.list"));
    assert!(operator_names.contains("orbit.friction.list"));
    assert!(operator_names.contains("orbit.friction.show"));
    assert!(operator_names.contains("orbit.friction.update"));
    assert!(!agent_names.contains("orbit.friction.list"));
    assert!(!agent_names.contains("orbit.friction.show"));
    // ORB-10727 [ADR-0358]: `runner` is withdrawn from the bridge, so no
    // canonical policy admits it and such a session would advertise nothing.
    let runner =
        OwnerMcpHost::new(root.path().to_path_buf(), McpCapability::Runner).expect("runner");
    assert!(
        runner
            .list_mcp_tool_definitions()
            .expect("runner tools")
            .is_empty(),
        "no v1 tool policy admits the withdrawn runner capability"
    );
}

/// ORB-10729 [mcp-bridge §8.1]: crew discovery and explicit task-crew
/// validation run where the workspace is owned, so they read this machine's own
/// layered crew config. No execution-profile row is published here — the
/// registration/poll protocol that carried publication is withdrawn
/// ([ADR-0358]) — and the endpoint answers anyway.
#[test]
fn owner_crew_discovery_and_task_validation_read_the_owner_local_crew_config() {
    let root = tempfile::tempdir().expect("global root");
    write_identity(&root, "hm_owner");
    std::fs::write(
        root.path().join("config.toml"),
        "[crews.sol]\nmodel = \"gpt-test\"\nprovider = \"codex\"\nbackend = \"cli\"\n\n[workflow]\ndefault_crew = \"sol\"\n",
    )
    .expect("owner crew config");
    add_checkoutless_workspace(&root, "ws_alpha");

    // The dormant projection table exists and is empty: nothing published a
    // profile, and crew validation must not need one.
    let store_path = initialize_store(&root);
    let profile_rows: i64 = Connection::open(&store_path)
        .expect("global store")
        .query_row(
            "SELECT COUNT(*) FROM workspace_execution_profiles",
            [],
            |row| row.get(0),
        )
        .expect("profile row count");
    assert_eq!(
        profile_rows, 0,
        "no v1 caller publishes an execution profile"
    );

    let host =
        OwnerMcpHost::new(root.path().to_path_buf(), McpCapability::Agent).expect("hub host");
    let workspace_context = |call_id: &str| {
        let mut context = context(McpCapability::Agent, call_id);
        context.workspace = Some("ws_alpha".to_string());
        context.workspace_id = Some("ws_alpha".to_string());
        context
    };

    // Crew discovery resolves the session workspace and projects this owner
    // machine's configured crews.
    let crews = host
        .call_tool(
            "orbit.crew.list",
            json!({"workspace": "ws_alpha"}),
            workspace_context("mcall-crew-list"),
        )
        .expect("crew list");
    assert_eq!(crews["workspace_id"], "ws_alpha");
    assert_eq!(crews["owner_machine_id"], "hm_owner");
    assert!(
        crews.get("profile").is_none(),
        "owner-local config carries no freshness/generation envelope: {crews}"
    );
    assert_eq!(crews["default_crew"], "sol");
    assert_eq!(crews["crews"][0]["name"], "sol");
    assert_eq!(crews["crews"][0]["model"], "gpt-test");
    let serialized = serde_json::to_string(&crews).expect("serialize crew list");
    for forbidden in ["config_digest", "ship_closure_digest", "\"root\""] {
        assert!(
            !serialized.contains(forbidden),
            "crew list leaked {forbidden}"
        );
    }

    // Valid padded execution/orchestration aliases are canonicalized from the
    // owner's configured crews before the coordination task lands.
    let created = host
        .call_tool(
            "orbit.task.add",
            json!({
                "workspace": "ws_alpha",
                "title": "Coordinate",
                "description": "Body",
                "crew": "  sol  ",
                "orchestrator": "  sol  ",
                "model": "codex"
            }),
            workspace_context("mcall-task-valid"),
        )
        .expect("valid crew task add");
    assert_eq!(created["crew"], "sol");
    assert_eq!(created["orchestrator"], "sol");
    let task_id = created["id"].as_str().expect("task id");

    let updated = host
        .call_tool(
            "orbit.task.update",
            json!({
                "workspace": "ws_alpha",
                "id": task_id,
                "orchestrator": "  sol  ",
                "model": "codex"
            }),
            workspace_context("mcall-task-update-valid"),
        )
        .expect("valid orchestrator task update");
    assert_eq!(updated["orchestrator"], "sol");

    let error = host
        .call_tool(
            "orbit.task.update",
            json!({
                "workspace": "ws_alpha",
                "id": task_id,
                "orchestrator": "ghost",
                "model": "codex"
            }),
            workspace_context("mcall-task-update-bad-orchestrator"),
        )
        .expect_err("unknown update orchestrator rejected");
    assert!(error.to_string().contains("ghost"), "unexpected: {error}");

    let cleared = host
        .call_tool(
            "orbit.task.update",
            json!({
                "workspace": "ws_alpha",
                "id": task_id,
                "orchestrator": "",
                "model": "codex"
            }),
            workspace_context("mcall-task-update-clear-orchestrator"),
        )
        .expect("clear does not require crew validation");
    assert_eq!(cleared["orchestrator"], Value::Null);

    // An unknown explicit crew is rejected before allocation.
    let error = host
        .call_tool(
            "orbit.task.add",
            json!({
                "workspace": "ws_alpha",
                "title": "Rejected",
                "description": "Body",
                "crew": "ghost",
                "model": "codex"
            }),
            workspace_context("mcall-task-bad"),
        )
        .expect_err("unknown crew rejected");
    assert!(error.to_string().contains("ghost"), "unexpected: {error}");

    let error = host
        .call_tool(
            "orbit.task.add",
            json!({
                "workspace": "ws_alpha",
                "title": "Rejected orchestrator",
                "description": "Body",
                "orchestrator": "ghost",
                "model": "codex"
            }),
            workspace_context("mcall-task-bad-orchestrator"),
        )
        .expect_err("unknown orchestrator rejected");
    assert!(error.to_string().contains("ghost"), "unexpected: {error}");

    // Omitting the crew is always accepted, even for coordination filing.
    host.call_tool(
        "orbit.task.add",
        json!({
            "workspace": "ws_alpha",
            "title": "No crew",
            "description": "Body",
            "model": "codex"
        }),
        workspace_context("mcall-task-nocrew"),
    )
    .expect("omitted crew task add");

    // The rejected task never entered hub coordination state: exactly the two
    // accepted tasks exist.
    let listed = host
        .call_tool(
            "orbit.task.list",
            json!({"workspace": "ws_alpha"}),
            workspace_context("mcall-task-list"),
        )
        .expect("task list");
    assert_eq!(
        listed.as_array().expect("task array").len(),
        2,
        "unknown-crew add must not mutate the task count"
    );
}

/// ORB-10727 [ADR-0358]: v1 has no registration handshake, so there is no
/// "unknown caller may only register" gate and no private method to guess.
/// SSH is the authenticator; the endpoint requires only that the connector
/// hand over a complete caller identity, and it advertises no custom method.
#[test]
fn owner_endpoint_negotiates_no_private_method_and_requires_only_caller_identity() {
    let root = tempfile::tempdir().expect("global root");
    write_identity(&root, "hm_owner");
    stamp_store(&root, "hm_owner");
    let host =
        OwnerMcpHost::new(root.path().to_path_buf(), McpCapability::Operator).expect("owner host");

    // An unregistered remote caller is admitted past the identity gate:
    // registration is withdrawn, so it fails on workspace resolution instead.
    let unregistered = host
        .call_tool(
            "orbit.crew.list",
            json!({"workspace": "ws_unknown"}),
            remote_context(McpCapability::Operator, "mcall-unregistered"),
        )
        .expect_err("unknown workspace");
    assert!(
        !unregistered.to_string().contains("not registered"),
        "caller registration must not be a gate: {unregistered}"
    );

    // An incomplete SSH-carried identity is still refused before dispatch.
    let mut anonymous = remote_context(McpCapability::Operator, "mcall-anonymous");
    anonymous.caller_machine_id = None;
    let denied = host
        .call_tool("orbit.crew.list", json!({"workspace": "ws_any"}), anonymous)
        .expect_err("SSH calls require an authenticated caller");
    assert!(
        denied
            .to_string()
            .contains("require authenticated caller machine_id"),
        "unexpected error: {denied}"
    );

    // No connector-private method survives, by any spelling.
    let before = crate::registry_snapshot_at(root.path()).expect("snapshot before");
    for method in [
        "orbit/private/register-spoke/v1",
        "orbit/private/allocate-knowledge-id/v1",
    ] {
        let missing = host
            .call_tool(
                method,
                json!({}),
                remote_context(McpCapability::Operator, "mcall-private"),
            )
            .expect_err("withdrawn private methods are not tools");
        assert!(
            missing.to_string().contains("not found"),
            "{method}: {missing}"
        );
    }
    let after = crate::registry_snapshot_at(root.path()).expect("snapshot after");
    assert_eq!(
        after.registry_revision, before.registry_revision,
        "a guessed private name creates no registry mutation"
    );

    // The endpoint's advertised instructions carry the owner contract only.
    let instructions = host.contract_instructions().to_string();
    assert!(instructions.starts_with("orbit-owner-contract-v1:"));
    assert!(
        !instructions.contains("register-spoke"),
        "no registration seam is advertised: {instructions}"
    );
}

#[test]
fn hub_checkoutless_dispatch_and_capability_denial_each_write_one_trusted_audit() {
    let root = tempfile::tempdir().expect("global root");
    write_identity(&root, "hm_owner");
    let database = stamp_store(&root, "hm_owner");
    add_checkoutless_workspace(&root, "ws_checkoutless");
    let host =
        OwnerMcpHost::new(root.path().to_path_buf(), McpCapability::Agent).expect("hub host");

    let task = host
        .call_tool(
            "orbit.task.add",
            json!({
                "workspace": "ws_checkoutless",
                "title": "Hub only",
                "description": "No checkout or connector",
                "model": "codex"
            }),
            context(McpCapability::Agent, "mcall-hub-add"),
        )
        .expect("checkoutless hub task");
    assert_eq!(task["title"], "Hub only");

    let operator_error = host
        .call_tool(
            "orbit_workspace_list",
            json!({}),
            context(McpCapability::Agent, "mcall-hub-capability-denied"),
        )
        .expect_err("operator-only tool denied");
    assert!(
        operator_error
            .to_string()
            .contains("fixed session is 'agent'")
    );

    let connection = Connection::open(database).expect("audit store");
    for (call_id, expected_status) in [
        ("mcall-hub-add", AuditEventStatus::Success),
        ("mcall-hub-capability-denied", AuditEventStatus::Denied),
    ] {
        let row = connection
            .query_row(
                "SELECT COUNT(*), status, mcp_call_id, process_machine_id
                 FROM audit_events WHERE mcp_call_id = ?1",
                [call_id],
                |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, Option<String>>(2)?,
                        row.get::<_, Option<String>>(3)?,
                    ))
                },
            )
            .expect("audit row");
        assert_eq!(row.0, 1, "one audit for {call_id}");
        assert_eq!(row.1, expected_status.to_string());
        assert_eq!(row.2.as_deref(), Some(call_id));
        assert_eq!(row.3.as_deref(), Some("hm_owner"));
    }
}

#[test]
fn ssh_hub_call_never_defaults_a_missing_caller_to_the_hub_identity() {
    let root = tempfile::tempdir().expect("global root");
    write_identity(&root, "hm_owner");
    let database = stamp_store(&root, "hm_owner");
    add_checkoutless_workspace(&root, "ws_checkoutless");
    let host =
        OwnerMcpHost::new(root.path().to_path_buf(), McpCapability::Agent).expect("hub host");
    let mut remote = context(McpCapability::Agent, "mcall-missing-caller");
    remote.transport = Some(McpTransport::SshMcp);
    remote.caller_machine_id = None;
    remote.caller_host_id = None;

    let error = host
        .call_tool(
            "orbit.task.list",
            json!({"workspace": "ws_checkoutless"}),
            remote,
        )
        .expect_err("unauthenticated remote caller must fail closed");
    assert!(error.to_string().contains("authenticated caller"));

    let connection = Connection::open(database).expect("audit store");
    let row = connection
        .query_row(
            "SELECT COUNT(*), status, caller_machine_id, process_machine_id, transport
             FROM audit_events WHERE mcp_call_id = 'mcall-missing-caller'",
            [],
            |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, Option<String>>(2)?,
                    row.get::<_, Option<String>>(3)?,
                    row.get::<_, Option<String>>(4)?,
                ))
            },
        )
        .expect("denial audit");
    assert_eq!(row.0, 1);
    assert_eq!(row.1, AuditEventStatus::Denied.to_string());
    assert_eq!(row.2, None, "remote caller must not be forged as the hub");
    assert_eq!(row.3.as_deref(), Some("hm_owner"));
    assert_eq!(row.4.as_deref(), Some("ssh-mcp"));
}

#[test]
fn owner_rechecks_store_stamp_before_listing_and_dispatch_without_task_mutation() {
    let root = tempfile::tempdir().expect("global root");
    write_identity(&root, "hm_owner");
    let database = stamp_store(&root, "hm_owner");
    add_checkoutless_workspace(&root, "ws_checkoutless");
    let host =
        OwnerMcpHost::new(root.path().to_path_buf(), McpCapability::Agent).expect("owner host");

    Connection::open(&database)
        .expect("store")
        .execute(
            "UPDATE hub_registry_metadata SET hub_machine_id = 'hm_other' WHERE id = 0",
            [],
        )
        .expect("drift stamp");
    assert!(host.list_mcp_tool_definitions().is_err());
    let error = host
        .call_tool(
            "orbit.task.add",
            json!({
                "workspace": "ws_checkoutless",
                "title": "Must not exist",
                "description": "authority drift",
                "model": "codex"
            }),
            context(McpCapability::Agent, "mcall-shadow-denied"),
        )
        .expect_err("shadow store denied");
    assert!(error.to_string().contains("shadow coordination store"));

    let registry = orbit_store_query_task_count(root.path(), "ws_checkoutless");
    assert_eq!(registry, 0, "authority denial created no task");
}

fn orbit_store_query_task_count(root: &std::path::Path, workspace_id: &str) -> i64 {
    let path = orbit_store_task_registry_path(root);
    let connection = Connection::open(path).expect("task registry");
    connection
        .query_row(
            "SELECT COUNT(*) FROM task_bundle_index WHERE workspace_id = ?1",
            [workspace_id],
            |row| row.get(0),
        )
        .unwrap_or(0)
}

fn orbit_store_task_registry_path(root: &std::path::Path) -> std::path::PathBuf {
    root.join("tasks/index.sqlite")
}
