//! Descriptor projection: capability advertisement and selector encoding.

use serde_json::{Value, json};

use super::super::descriptor::WorkspaceDescriptor;
use super::fixtures::{OWNER_MACHINE, REPLICA_MACHINE, destination, workspace};

fn project(destination_machine_id: &str, owner_machine_id: Option<&str>) -> Value {
    let destination = destination("orbit-box", destination_machine_id);
    let descriptor =
        WorkspaceDescriptor::reachable(&destination, workspace("ws_orbit", owner_machine_id));
    serde_json::to_value(descriptor).expect("serialize descriptor")
}

#[test]
fn owning_the_workspace_advertises_both_classes() {
    assert_eq!(
        project(OWNER_MACHINE, Some(OWNER_MACHINE))["capabilities"],
        json!(["control_plane", "execute"]),
    );
}

#[test]
fn holding_another_machines_workspace_advertises_execute_only() {
    assert_eq!(
        project(REPLICA_MACHINE, Some(OWNER_MACHINE))["capabilities"],
        json!(["execute"]),
    );
}

#[test]
fn a_workspace_without_an_owner_machine_cannot_advertise_control_plane() {
    // Standalone registries predating host identity omit `owner_machine_id`;
    // they are not a control-plane authority for anyone.
    assert_eq!(
        project(OWNER_MACHINE, None)["capabilities"],
        json!(["execute"])
    );
}

#[test]
fn the_selector_is_the_destinations_machine_and_the_workspace_id() {
    let projected = project(REPLICA_MACHINE, Some(OWNER_MACHINE));

    assert_eq!(
        projected["selector"], "hm_replica/ws_orbit",
        "the selector names where the call is delivered, not who owns the workspace",
    );
}

#[test]
fn a_workspace_id_outside_the_encoding_is_listed_without_a_selector() {
    let destination = destination("orbit-box", OWNER_MACHINE);
    let descriptor =
        WorkspaceDescriptor::reachable(&destination, workspace("legacy-workspace", None));
    let projected = serde_json::to_value(descriptor).expect("serialize descriptor");

    assert_eq!(projected["id"], "legacy-workspace");
    assert_eq!(
        projected["selector"],
        Value::Null,
        "an unaddressable workspace is still visible, just not routable",
    );
}
