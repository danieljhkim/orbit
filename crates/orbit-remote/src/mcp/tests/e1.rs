use std::collections::BTreeSet;

use chrono::Utc;
use orbit_common::types::{
    AuditEventStatus, HostRegistration, McpCapability, McpToolPlacement, McpTransport,
    SPOKE_REGISTRATION_METHOD_V1, SPOKE_REGISTRATION_SCHEMA_VERSION, SpokeRegistrationRequestV1,
    SpokeRegistrationStageV1, ToolSessionContext, Workspace, WorkspacePresenceDeclaration,
    WorkspaceRegistry, WorkspaceStatus,
};
use orbit_core::runtime::HubCoordinationExecutor;
use orbit_mcp::McpHost;
use rusqlite::Connection;
use serde_json::json;
use tempfile::TempDir;

use super::super::config::{HubTransport, load_trusted_mcp_config};
use super::super::hub::HubMcpHost;

fn write_identity(root: &TempDir, mode: &str, machine_id: &str) {
    std::fs::write(
        root.path().join("host.toml"),
        format!(
            "schema_version = 1\nmachine_id = \"{machine_id}\"\nhost_id = \"test-host\"\nmode = \"{mode}\"\n"
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
                owner_machine_id: Some("hm_hub".to_string()),
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
        Some("hm_hub".to_string()),
        Some("test-host".to_string()),
    );
    context.effective_capabilities = BTreeSet::from([capability]);
    context.mcp_call_id = Some(call_id.to_string());
    context
}

fn remote_context(capability: McpCapability, call_id: &str) -> ToolSessionContext {
    ToolSessionContext {
        caller_machine_id: Some("hm_spoke".to_string()),
        caller_host_id: Some("spoke".to_string()),
        transport: Some(McpTransport::SshMcp),
        effective_capabilities: BTreeSet::from([capability]),
        origin_session_id: Some("remote-session".to_string()),
        mcp_call_id: Some(call_id.to_string()),
        ..ToolSessionContext::default()
    }
}

fn spoke_registration(machine_id: &str, host_id: &str) -> SpokeRegistrationRequestV1 {
    SpokeRegistrationRequestV1 {
        schema_version: SPOKE_REGISTRATION_SCHEMA_VERSION,
        identity: HostRegistration {
            machine_id: machine_id.to_string(),
            host_id: host_id.to_string(),
            labels: BTreeSet::new(),
        },
        presence: Vec::new(),
        profiles: Vec::new(),
    }
}

fn valid_config() -> &'static str {
    "[hub]\nmachine_id = \"hm_hub\"\ntransport = \"ssh\"\nhost = \"orbit-hub\"\nallowed_capabilities = [\"agent\", \"operator\", \"runner\"]\n"
}

#[test]
fn trusted_config_parses_one_singular_safe_hub() {
    let root = tempfile::tempdir().expect("global root");
    std::fs::write(root.path().join("mcp.toml"), valid_config()).expect("mcp config");
    let config = load_trusted_mcp_config(root.path()).expect("trusted config");
    let hub = config.hub.expect("hub");
    assert_eq!(hub.machine_id, "hm_hub");
    assert_eq!(hub.transport, HubTransport::Ssh);
    assert_eq!(hub.host, "orbit-hub");
    assert_eq!(
        hub.allowed_capabilities,
        BTreeSet::from([
            McpCapability::Agent,
            McpCapability::Operator,
            McpCapability::Runner,
        ])
    );
}

#[test]
fn trusted_config_fails_closed_on_unknown_duplicate_empty_and_unsupported_values() {
    let cases = [
        ("owner = \"elsewhere\"\n", "unknown field"),
        ("[hubs.dk1]\nmachine_id = \"hm_hub\"\n", "unknown field"),
        (
            "[hub]\nmachine_id = \"hm_hub\"\ntransport = \"ssh\"\nhost = \"hub\"\nallowed_capabilities = [\"agent\"]\ncommand = \"orbit mcp serve --hub\"\n",
            "unknown field",
        ),
        (
            "[hub]\nmachine_id = \"hm_hub\"\ntransport = \"http\"\nhost = \"hub\"\nallowed_capabilities = [\"agent\"]\n",
            "unknown variant",
        ),
        (
            "[hub]\nmachine_id = \"not-a-machine\"\ntransport = \"ssh\"\nhost = \"hub\"\nallowed_capabilities = [\"agent\"]\n",
            "invalid hub machine_id",
        ),
        (
            "[hub]\nmachine_id = \"hm_hub\"\ntransport = \"ssh\"\nhost = \"hub\"\nallowed_capabilities = []\n",
            "must not be empty",
        ),
        (
            "[hub]\nmachine_id = \"hm_hub\"\ntransport = \"ssh\"\nhost = \"hub\"\nallowed_capabilities = [\"agent\", \"agent\"]\n",
            "repeats hub capability",
        ),
        (
            "[hub]\nmachine_id = \"hm_hub\"\ntransport = \"ssh\"\nhost = \"hub\"\nallowed_capabilities = [\"admin\"]\n",
            "unknown variant",
        ),
        (
            "[hub]\nmachine_id = \"hm_hub\"\ntransport = \"ssh\"\nhost = \"hub\"\n",
            "missing field",
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
        "hub extra",
        "user@hub",
        "hub;command",
        "hub|command",
        "hub\ncommand",
        "/tmp/hub",
        "hub=target",
        "*.internal",
    ] {
        let root = tempfile::tempdir().expect("global root");
        let encoded = toml::Value::String(alias.to_string()).to_string();
        let body = format!(
            "[hub]\nmachine_id = \"hm_hub\"\ntransport = \"ssh\"\nhost = {encoded}\nallowed_capabilities = [\"agent\"]\n"
        );
        std::fs::write(root.path().join("mcp.toml"), body).expect("mcp config");
        let error = load_trusted_mcp_config(root.path()).expect_err("unsafe alias");
        assert!(
            error.to_string().contains("invalid hub host alias"),
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
        "[hub]\nmachine_id = \"hm_repo\"\ntransport = \"ssh\"\nhost = \"repo\"\nallowed_capabilities = [\"runner\"]\n",
    )
    .expect("repo decoy");
    std::fs::write(
        env_decoy.path().join("mcp.toml"),
        "[hub]\nmachine_id = \"hm_env\"\ntransport = \"ssh\"\nhost = \"env\"\nallowed_capabilities = [\"operator\"]\n",
    )
    .expect("env decoy");

    // The loader has no cwd or environment input: only the explicit machine
    // global root can influence the result.
    let config = load_trusted_mcp_config(root.path()).expect("global config");
    assert_eq!(config.hub.expect("hub").machine_id, "hm_hub");
}

#[test]
fn spoke_route_requires_config_and_exact_non_hierarchical_membership() {
    use crate::{HOST_IDENTITY_SCHEMA_VERSION, HostIdentity, HostMode};

    let identity = HostIdentity {
        schema_version: HOST_IDENTITY_SCHEMA_VERSION,
        machine_id: "hm_spoke".to_string(),
        host_id: "spoke".to_string(),
        mode: HostMode::Spoke,
    };
    let missing = super::super::config::TrustedMcpConfig::default();
    assert!(missing.spoke_route(&identity, None).is_err());

    let root = tempfile::tempdir().expect("global root");
    std::fs::write(
        root.path().join("mcp.toml"),
        "[hub]\nmachine_id = \"hm_hub\"\ntransport = \"ssh\"\nhost = \"hub\"\nallowed_capabilities = [\"operator\"]\n",
    )
    .expect("mcp config");
    let config = load_trusted_mcp_config(root.path()).expect("config");
    assert!(
        config.spoke_route(&identity, None).is_err(),
        "agent default"
    );
    assert!(
        config
            .spoke_route(&identity, Some(McpCapability::Runner))
            .is_err(),
        "operator must not imply runner"
    );
    let (_, effective) = config
        .spoke_route(&identity, Some(McpCapability::Operator))
        .expect("operator grant");
    assert_eq!(effective, McpCapability::Operator);
}

#[test]
fn hub_host_rejects_non_hub_modes_and_unstamped_or_shadow_stores() {
    for mode in ["standalone", "spoke"] {
        let root = tempfile::tempdir().expect("global root");
        write_identity(&root, mode, "hm_hub");
        let error = HubMcpHost::new(root.path().to_path_buf(), McpCapability::Agent)
            .expect_err("invalid mode");
        assert!(error.to_string().contains("requires host.toml mode 'hub'"));
    }

    let unstamped = tempfile::tempdir().expect("global root");
    write_identity(&unstamped, "hub", "hm_hub");
    initialize_store(&unstamped);
    let error = HubMcpHost::new(unstamped.path().to_path_buf(), McpCapability::Agent)
        .expect_err("unstamped store");
    assert!(error.to_string().contains("no configured hub_machine_id"));

    let shadow = tempfile::tempdir().expect("global root");
    write_identity(&shadow, "hub", "hm_hub");
    stamp_store(&shadow, "hm_other");
    let error = HubMcpHost::new(shadow.path().to_path_buf(), McpCapability::Agent)
        .expect_err("shadow store");
    assert!(error.to_string().contains("shadow coordination store"));
}

#[test]
fn hub_listing_uses_one_canonical_placement_and_capability_predicate() {
    let root = tempfile::tempdir().expect("global root");
    write_identity(&root, "hub", "hm_hub");
    stamp_store(&root, "hm_hub");

    let agent = HubMcpHost::new(root.path().to_path_buf(), McpCapability::Agent).expect("agent");
    let agent_definitions = agent.list_mcp_tool_definitions().expect("agent tools");
    assert!(agent_definitions.iter().all(|definition| {
        matches!(
            definition.policy.placement(),
            McpToolPlacement::Hub | McpToolPlacement::Owner | McpToolPlacement::Composite
        ) && definition
            .policy
            .allowed_capabilities()
            .contains(&McpCapability::Agent)
    }));
    let agent_names = agent_definitions
        .iter()
        .map(|definition| definition.schema.name.as_str())
        .collect::<BTreeSet<_>>();
    assert!(agent_names.contains("orbit.task.add"));
    assert!(!agent_names.contains("orbit.host.list"));
    // Crew discovery is admitted for agent (unlike the operator-only registry
    // discovery tools), proving capability is by placement, not a hierarchy.
    assert!(agent_names.contains("orbit.crew.list"));
    assert!(
        !agent_names
            .iter()
            .any(|name| name.starts_with("orbit.graph."))
    );

    let operator =
        HubMcpHost::new(root.path().to_path_buf(), McpCapability::Operator).expect("operator");
    let operator_names = operator
        .list_mcp_tool_definitions()
        .expect("operator tools")
        .into_iter()
        .map(|definition| definition.schema.name)
        .collect::<BTreeSet<_>>();
    assert!(operator_names.contains("orbit.host.list"));
    assert!(operator_names.contains("orbit.crew.list"));
    assert!(operator_names.contains("orbit.friction.list"));
    assert!(operator_names.contains("orbit.friction.show"));
    assert!(operator_names.contains("orbit.friction.update"));
    assert!(!agent_names.contains("orbit.friction.list"));
    assert!(!agent_names.contains("orbit.friction.show"));
    assert!(
        !operator_names.contains(SPOKE_REGISTRATION_METHOD_V1),
        "private registration must never enter tools/list definitions"
    );

    let runner = HubMcpHost::new(root.path().to_path_buf(), McpCapability::Runner).expect("runner");
    assert!(
        runner
            .list_mcp_tool_definitions()
            .expect("runner tools")
            .is_empty(),
        "D1 currently declares no runner-capability hub tools"
    );
}

#[test]
fn hub_crew_discovery_and_task_validation_read_the_owner_execution_profile() {
    use orbit_common::types::{ExecutionProfileCrewV1, ExecutionProfileShipV1, ExecutionProfileV1};

    use crate::host_identity::{HOST_IDENTITY_SCHEMA_VERSION, HostIdentity, HostMode};
    use crate::host_registry::HostRegistryService;

    let root = tempfile::tempdir().expect("global root");
    write_identity(&root, "hub", "hm_hub");
    // Register + stamp the hub, bind the workspace owner, and publish an owner
    // execution profile through the same coordination store the hub reads.
    let service = HostRegistryService::new(crate::remote_store_at(root.path()).expect("store"));
    service
        .register_hub_identity(
            &HostIdentity {
                schema_version: HOST_IDENTITY_SCHEMA_VERSION,
                machine_id: "hm_hub".to_string(),
                host_id: "test-host".to_string(),
                mode: HostMode::Hub,
            },
            BTreeSet::new(),
        )
        .expect("register hub");
    add_checkoutless_workspace(&root, "ws_alpha");
    let registry = crate::workspace_registry::load_registry_from(
        &crate::workspace_registry::registry_path_for(root.path()),
    )
    .expect("registry");
    service
        .bind_workspace_owner(&registry, "ws_alpha", "hm_hub")
        .expect("bind owner");
    let observed = Utc::now();
    let mut profile = ExecutionProfileV1 {
        schema_version: 1,
        workspace_id: "ws_alpha".to_string(),
        owner_machine_id: "hm_hub".to_string(),
        observed_at: observed,
        config_digest: String::new(),
        default_crew: "sol".to_string(),
        crews: vec![ExecutionProfileCrewV1 {
            name: "sol".to_string(),
            provider: "codex".to_string(),
            model: "gpt-test".to_string(),
            backend: "cli".to_string(),
            description: None,
            tags: Vec::new(),
        }],
        ship: ExecutionProfileShipV1 {
            mode: "pr".to_string(),
            base_branch: "agent-main".to_string(),
            ship_closure_digest: "a".repeat(64),
        },
    };
    profile.config_digest = profile.compute_config_digest().expect("config digest");
    service
        .publish_execution_profile("hm_hub", 0, &profile)
        .expect("publish owner profile");

    let host = HubMcpHost::new(root.path().to_path_buf(), McpCapability::Agent).expect("hub host");
    let workspace_context = |call_id: &str| {
        let mut context = context(McpCapability::Agent, call_id);
        context.workspace = Some("ws_alpha".to_string());
        context.workspace_id = Some("ws_alpha".to_string());
        context
    };

    // Crew discovery resolves the session workspace and projects the owner
    // profile, not the hub's unrelated local configuration.
    let crews = host
        .call_tool(
            "orbit.crew.list",
            json!({"workspace": "ws_alpha"}),
            workspace_context("mcall-crew-list"),
        )
        .expect("crew list");
    assert_eq!(crews["workspace_id"], "ws_alpha");
    assert_eq!(crews["owner_machine_id"], "hm_hub");
    assert_eq!(crews["profile"]["freshness"], "current");
    assert_eq!(crews["profile"]["generation"], 1);
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

    // A valid explicit crew is accepted and the coordination task lands.
    let created = host
        .call_tool(
            "orbit.task.add",
            json!({
                "workspace": "ws_alpha",
                "title": "Coordinate",
                "description": "Body",
                "crew": "sol",
                "model": "codex"
            }),
            workspace_context("mcall-task-valid"),
        )
        .expect("valid crew task add");
    assert_eq!(created["crew"], "sol");

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

#[test]
fn unknown_remote_caller_can_only_register_and_retirement_invalidates_open_peer() {
    let root = tempfile::tempdir().expect("global root");
    write_identity(&root, "hub", "hm_hub");
    stamp_store(&root, "hm_hub");
    let host =
        HubMcpHost::new(root.path().to_path_buf(), McpCapability::Operator).expect("hub host");

    let unknown = host
        .call_tool(
            "orbit.host.list",
            json!({}),
            remote_context(McpCapability::Operator, "mcall-before-register"),
        )
        .expect_err("unknown caller denied before discovery");
    assert!(unknown.to_string().contains("not registered"));

    let mismatch = host
        .private_register_spoke(
            spoke_registration("hm_other", "other"),
            remote_context(McpCapability::Operator, "mcall-mismatch"),
        )
        .expect("typed result");
    assert!(!mismatch.complete);
    assert!(mismatch.last_committed_stage.is_none());

    let registered = host
        .private_register_spoke(
            spoke_registration("hm_spoke", "spoke"),
            remote_context(McpCapability::Operator, "mcall-register"),
        )
        .expect("typed result");
    assert!(registered.complete);
    assert_eq!(
        registered
            .snapshot
            .as_ref()
            .expect("snapshot")
            .hosts
            .iter()
            .filter(|entry| entry.machine_id == "hm_spoke")
            .count(),
        1
    );
    let before_hidden_call =
        crate::registry_snapshot_at(root.path()).expect("snapshot before guessed ordinary call");
    let hidden = host
        .call_tool(
            SPOKE_REGISTRATION_METHOD_V1,
            serde_json::to_value(spoke_registration("hm_spoke", "spoke"))
                .expect("registration JSON"),
            remote_context(McpCapability::Operator, "mcall-guessed-registration"),
        )
        .expect_err("ordinary tools/call cannot invoke the private method");
    assert!(hidden.to_string().contains("not found"));
    let after_hidden_call =
        crate::registry_snapshot_at(root.path()).expect("snapshot after guessed ordinary call");
    assert_eq!(
        after_hidden_call.registry_revision, before_hidden_call.registry_revision,
        "guessed private name created no registry mutation"
    );
    host.call_tool(
        "orbit.host.list",
        json!({}),
        remote_context(McpCapability::Operator, "mcall-after-register"),
    )
    .expect("registered active caller admitted");

    crate::host_registry_service_at(root.path())
        .expect("service")
        .retire("hm_spoke")
        .expect("retire spoke");
    let retired = host
        .call_tool(
            "orbit.host.list",
            json!({}),
            remote_context(McpCapability::Operator, "mcall-after-retire"),
        )
        .expect_err("retirement invalidates already-open peer");
    assert!(retired.to_string().contains("retired"));
}

#[test]
fn registration_reports_registry_commit_before_projection_failure_and_can_repair() {
    let root = tempfile::tempdir().expect("global root");
    write_identity(&root, "hub", "hm_hub");
    stamp_store(&root, "hm_hub");
    add_checkoutless_workspace(&root, "ws_checkoutless");
    let host = HubMcpHost::new(root.path().to_path_buf(), McpCapability::Agent).expect("hub host");

    let mut invalid = spoke_registration("hm_spoke", "spoke");
    invalid.presence.push(WorkspacePresenceDeclaration {
        workspace_id: "ws_unknown".to_string(),
        root: root.path().join("spoke-checkout"),
        last_verified: Utc::now(),
    });
    let partial = host
        .private_register_spoke(
            invalid,
            remote_context(McpCapability::Agent, "mcall-register-partial"),
        )
        .expect("typed partial result");
    assert!(!partial.complete);
    assert_eq!(
        partial.last_committed_stage,
        Some(SpokeRegistrationStageV1::Registry)
    );
    assert_eq!(
        partial.host.as_ref().map(|host| host.machine_id.as_str()),
        Some("hm_spoke")
    );
    assert!(partial.snapshot.is_none());

    let snapshot =
        crate::registry_snapshot_at(root.path()).expect("registry after projection failure");
    assert!(
        snapshot
            .hosts
            .iter()
            .any(|host| host.machine_id == "hm_spoke"),
        "the committed registration is not rolled back"
    );

    let mut repaired = spoke_registration("hm_spoke", "spoke");
    repaired.presence.push(WorkspacePresenceDeclaration {
        workspace_id: "ws_checkoutless".to_string(),
        root: root.path().join("spoke-checkout"),
        last_verified: Utc::now(),
    });
    let complete = host
        .private_register_spoke(
            repaired,
            remote_context(McpCapability::Agent, "mcall-register-repair"),
        )
        .expect("typed complete result");
    assert!(complete.complete);
    assert_eq!(
        complete.last_committed_stage,
        Some(SpokeRegistrationStageV1::Snapshot)
    );
    assert_eq!(complete.presence_workspace_ids, ["ws_checkoutless"]);
}

#[test]
fn hub_checkoutless_dispatch_and_denials_each_write_one_trusted_audit() {
    let root = tempfile::tempdir().expect("global root");
    write_identity(&root, "hub", "hm_hub");
    let database = stamp_store(&root, "hm_hub");
    add_checkoutless_workspace(&root, "ws_checkoutless");
    let host = HubMcpHost::new(root.path().to_path_buf(), McpCapability::Agent).expect("hub host");

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

    let graph_error = host
        .call_tool(
            "orbit_graph_search",
            json!({"workspace": "ws_checkoutless", "query": "never"}),
            context(McpCapability::Agent, "mcall-hub-graph-denied"),
        )
        .expect_err("removed graph MCP tool denied");
    assert!(graph_error.to_string().contains("not found"));

    let operator_error = host
        .call_tool(
            "orbit_host_list",
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
        ("mcall-hub-graph-denied", AuditEventStatus::Denied),
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
        assert_eq!(row.3.as_deref(), Some("hm_hub"));
    }
}

#[test]
fn ssh_hub_call_never_defaults_a_missing_caller_to_the_hub_identity() {
    let root = tempfile::tempdir().expect("global root");
    write_identity(&root, "hub", "hm_hub");
    let database = stamp_store(&root, "hm_hub");
    add_checkoutless_workspace(&root, "ws_checkoutless");
    let host = HubMcpHost::new(root.path().to_path_buf(), McpCapability::Agent).expect("hub host");
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
    assert_eq!(row.3.as_deref(), Some("hm_hub"));
    assert_eq!(row.4.as_deref(), Some("ssh-mcp"));
}

#[test]
fn hub_rechecks_store_stamp_before_listing_and_dispatch_without_task_mutation() {
    let root = tempfile::tempdir().expect("global root");
    write_identity(&root, "hub", "hm_hub");
    let database = stamp_store(&root, "hm_hub");
    add_checkoutless_workspace(&root, "ws_checkoutless");
    let host = HubMcpHost::new(root.path().to_path_buf(), McpCapability::Agent).expect("hub host");

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
