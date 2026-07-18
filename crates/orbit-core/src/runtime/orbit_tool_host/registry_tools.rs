//! Host implementations for the canonical sanitized discovery tools
//! `orbit.host.list` and `orbit.workspace.list` [ORB-10267].
//!
//! Both read the single path-free [`RegistrySnapshotV1`] projection and emit
//! only its allowlisted sanitized fields. The snapshot already excludes
//! presence roots, checkout paths, raw execution-profile payloads, crews, and
//! models, so serializing its typed host/workspace entries directly cannot leak
//! any of them.

use orbit_common::types::OrbitError;
use serde_json::{Value, json};

use crate::HostRegistryService;
use crate::OrbitRuntime;

pub(super) fn host_list(runtime: &OrbitRuntime) -> Result<Value, OrbitError> {
    let snapshot = HostRegistryService::new(runtime.sqlite_store()?).snapshot()?;
    let hosts = serde_json::to_value(&snapshot.hosts)
        .map_err(|error| OrbitError::Store(format!("serialize host list: {error}")))?;
    Ok(json!({
        "hub_machine_id": snapshot.hub_machine_id,
        "registry_revision": snapshot.registry_revision,
        "hosts": hosts,
    }))
}

pub(super) fn workspace_list(runtime: &OrbitRuntime) -> Result<Value, OrbitError> {
    let snapshot = HostRegistryService::new(runtime.sqlite_store()?).snapshot()?;
    let workspaces = serde_json::to_value(&snapshot.workspaces)
        .map_err(|error| OrbitError::Store(format!("serialize workspace list: {error}")))?;
    Ok(json!({
        "hub_machine_id": snapshot.hub_machine_id,
        "registry_revision": snapshot.registry_revision,
        "workspaces": workspaces,
    }))
}
