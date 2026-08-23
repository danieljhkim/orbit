//! The mux against fake destinations: no SSH, no local registry, no cache.

use std::collections::BTreeSet;
use std::sync::Arc;

use orbit_common::OrbitError;
use orbit_types::tool::{McpToolScope, ToolSessionContext};
use serde_json::Value;

use super::super::host::{FEDERATED_WORKSPACE_LIST_TOOL, FederatedMcpHost};
use super::super::probe::DestinationSnapshot;
use super::fixtures::{OWNER_MACHINE, REPLICA_MACHINE, ScriptedProbe, destination, workspace};
use crate::McpHost;

/// A gateway holding one owner destination, one replica destination, and one
/// destination whose SSH never comes up.
fn three_destination_mux() -> FederatedMcpHost {
    let destinations = vec![
        destination("orbit-owner", OWNER_MACHINE),
        destination("operator@orbit-replica", REPLICA_MACHINE),
        destination("orbit-down", "hm_down"),
    ];
    let probe = ScriptedProbe::new()
        .answering(
            OWNER_MACHINE,
            DestinationSnapshot {
                machine_id: OWNER_MACHINE.to_string(),
                workspaces: vec![workspace("ws_orbit", Some(OWNER_MACHINE))],
            },
        )
        .answering(
            REPLICA_MACHINE,
            DestinationSnapshot {
                // The replica holds a checkout of a workspace another machine
                // owns, which is exactly what makes it execute-only.
                machine_id: REPLICA_MACHINE.to_string(),
                workspaces: vec![workspace("ws_orbit", Some(OWNER_MACHINE))],
            },
        )
        .refusing(
            "hm_down",
            OrbitError::UnreachableDestination("hm_down: could not start SSH".to_string()),
        );
    FederatedMcpHost::new(destinations, Arc::new(probe))
}

fn list(host: &FederatedMcpHost) -> Vec<Value> {
    let listed = host
        .call_tool(
            FEDERATED_WORKSPACE_LIST_TOOL,
            Value::Null,
            ToolSessionContext::default(),
        )
        .expect("federated list");
    assert!(
        listed.get("machine_id").is_none(),
        "machine_id belongs on each descriptor, not the envelope: {listed}"
    );
    listed["workspaces"]
        .as_array()
        .expect("workspace rows")
        .clone()
}

#[test]
fn the_mux_advertises_only_the_federated_list() {
    let host = three_destination_mux();
    let definitions = host.list_mcp_tool_definitions().expect("definitions");

    assert_eq!(
        definitions
            .iter()
            .map(|definition| definition.schema.name.as_str())
            .collect::<Vec<_>>(),
        [FEDERATED_WORKSPACE_LIST_TOOL],
        "routing has not landed, so no other tool may be advertised",
    );
    let listing = &definitions[0];
    // Session-unbound: no workspace parameter, and global scope so the kernel
    // never demands a selector for it.
    assert!(listing.schema.parameters.is_empty());
    assert_eq!(listing.scope, McpToolScope::Global);
    assert_ne!(
        listing.schema.description,
        "List active workspaces with a checkout registered on this machine.",
        "the federated list is a new shape, not v1's machine-local list",
    );
}

#[test]
fn an_unadvertised_tool_is_not_on_this_host() {
    let host = three_destination_mux();

    let error = host
        .call_tool("orbit.task.add", Value::Null, ToolSessionContext::default())
        .expect_err("unrouted tools must be refused by name");
    assert!(matches!(error, OrbitError::ToolNotOnThisHost(_)), "{error}");
}

#[test]
fn every_configured_destination_is_listed_with_its_own_identity() {
    let rows = list(&three_destination_mux());

    assert_eq!(rows.len(), 3, "no configured destination may be omitted");
    assert_eq!(
        rows.iter()
            .map(|row| row["machine_id"].as_str().expect("machine_id"))
            .collect::<Vec<_>>(),
        [OWNER_MACHINE, REPLICA_MACHINE, "hm_down"],
    );
    assert_eq!(
        rows.iter()
            .map(|row| row["host"].as_str().expect("host"))
            .collect::<Vec<_>>(),
        ["orbit-owner", "operator@orbit-replica", "orbit-down"],
    );
}

#[test]
fn a_reachable_owner_may_advertise_control_plane_and_execute() {
    let rows = list(&three_destination_mux());
    let owner = &rows[0];

    assert_eq!(owner["selector"], format!("{OWNER_MACHINE}/ws_orbit"));
    assert_eq!(owner["reachability"], "reachable");
    assert_eq!(owner["checkout_health"], "active");
    assert_eq!(
        owner["capabilities"],
        serde_json::json!(["control_plane", "execute"])
    );
    // The v1 workspace fields ride along unchanged.
    assert_eq!(owner["id"], "ws_orbit");
    assert_eq!(owner["status"], "active");
    assert_eq!(owner["base_branch"], "main");
    assert_eq!(owner["owner_machine_id"], OWNER_MACHINE);
}

#[test]
fn a_replica_advertises_execute_only() {
    let rows = list(&three_destination_mux());
    let replica = &rows[1];

    assert_eq!(replica["selector"], format!("{REPLICA_MACHINE}/ws_orbit"));
    assert_eq!(replica["reachability"], "reachable");
    assert_eq!(
        replica["capabilities"],
        serde_json::json!(["execute"]),
        "a checkout of another machine's workspace is not a control plane",
    );
}

#[test]
fn an_ssh_down_destination_is_listed_as_unreachable_with_unknown_health() {
    let rows = list(&three_destination_mux());
    let down = &rows[2];

    assert_eq!(down["reachability"], "unreachable");
    assert_eq!(
        down["checkout_health"], "unknown",
        "a host that never answered says nothing about its checkouts",
    );
    assert_eq!(down["selector"], Value::Null);
    assert_eq!(down["capabilities"], serde_json::json!([]));
    assert!(
        down.get("id").is_none(),
        "no workspace was observed, so none may be claimed: {down}",
    );
}

#[test]
fn every_descriptor_carries_the_pinned_federated_keys() {
    let federated_keys = BTreeSet::from([
        "selector",
        "host",
        "machine_id",
        "reachability",
        "checkout_health",
        "capabilities",
    ]);
    for row in list(&three_destination_mux()) {
        let keys = row
            .as_object()
            .expect("descriptor object")
            .keys()
            .map(String::as_str)
            .collect::<BTreeSet<_>>();
        assert!(
            federated_keys.is_subset(&keys),
            "descriptor is missing pinned keys: {row}",
        );
        assert!(
            !keys.contains("health"),
            "reachability and checkout health must not collapse into one key: {row}",
        );
    }
}

#[test]
fn a_destination_answering_under_another_identity_is_unreachable() {
    let destinations = vec![destination("orbit-owner", OWNER_MACHINE)];
    // The pinned machine is not what answered, so the configured destination
    // was not reached — whatever else was.
    let probe = ScriptedProbe::new().answering(
        OWNER_MACHINE,
        DestinationSnapshot {
            machine_id: "hm_impostor".to_string(),
            workspaces: vec![workspace("ws_orbit", Some("hm_impostor"))],
        },
    );
    let host = FederatedMcpHost::new(destinations, Arc::new(probe));

    let rows = list(&host);
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["machine_id"], OWNER_MACHINE);
    assert_eq!(rows[0]["reachability"], "unreachable");
    assert_eq!(rows[0]["checkout_health"], "unknown");
    assert_eq!(rows[0]["selector"], Value::Null);
}

#[test]
fn a_reachable_destination_with_no_workspaces_still_appears() {
    let destinations = vec![destination("orbit-empty", OWNER_MACHINE)];
    let probe = ScriptedProbe::new().answering(
        OWNER_MACHINE,
        DestinationSnapshot {
            machine_id: OWNER_MACHINE.to_string(),
            workspaces: Vec::new(),
        },
    );
    let host = FederatedMcpHost::new(destinations, Arc::new(probe));

    let rows = list(&host);
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["reachability"], "reachable");
    assert_eq!(rows[0]["checkout_health"], "unknown");
    assert_eq!(rows[0]["selector"], Value::Null);
}

#[test]
fn an_inactive_workspace_is_listed_rather_than_filtered_out() {
    let destinations = vec![destination("orbit-owner", OWNER_MACHINE)];
    let mut invalid = workspace("ws_broken", Some(OWNER_MACHINE));
    invalid.status = orbit_types::workspace::WorkspaceStatus::Invalid;
    let probe = ScriptedProbe::new().answering(
        OWNER_MACHINE,
        DestinationSnapshot {
            machine_id: OWNER_MACHINE.to_string(),
            workspaces: vec![invalid],
        },
    );
    let host = FederatedMcpHost::new(destinations, Arc::new(probe));

    let rows = list(&host);
    assert_eq!(rows.len(), 1, "the mux applies no Active filter of its own");
    assert_eq!(rows[0]["reachability"], "reachable");
    assert_eq!(rows[0]["checkout_health"], "invalid");
    assert_eq!(rows[0]["selector"], format!("{OWNER_MACHINE}/ws_broken"));
}

#[test]
fn the_list_comes_only_from_probed_destinations() {
    // A mux with no configured destinations lists nothing, whatever workspaces
    // this machine's own registry happens to hold. The host has no registry
    // handle at all, which is the structural half of that guarantee.
    let host = FederatedMcpHost::new(Vec::new(), Arc::new(ScriptedProbe::new()));

    assert!(list(&host).is_empty());
}

#[test]
fn each_call_reprobes_rather_than_reusing_the_last_answer() {
    let destinations = vec![destination("orbit-owner", OWNER_MACHINE)];
    let probe = ScriptedProbe::new().answering(
        OWNER_MACHINE,
        DestinationSnapshot {
            machine_id: OWNER_MACHINE.to_string(),
            workspaces: vec![workspace("ws_orbit", Some(OWNER_MACHINE))],
        },
    );
    let calls = probe.call_counter();
    let host = FederatedMcpHost::new(destinations, Arc::new(probe));

    list(&host);
    list(&host);
    assert_eq!(calls.count(), 2, "list freshness comes from a live probe");
}
