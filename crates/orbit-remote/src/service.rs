//! Config-resolved Remote service composition.

use std::path::Path;

use orbit_common::types::{OrbitError, RegistrySnapshotV1};
use orbit_store::{AuditEventInsertParams, Store};

use crate::{HostRegistryService, RemoteStore};

/// Open the Remote feature store selected by the effective global config.
pub fn remote_store_at(global_root: &Path) -> Result<RemoteStore, OrbitError> {
    let database = orbit_core::config::resolved_audit_db_path(global_root, global_root)?;
    RemoteStore::open(&database)
}

/// Open the one registry service for a machine-global root.
pub fn host_registry_service_at(global_root: &Path) -> Result<HostRegistryService, OrbitError> {
    Ok(HostRegistryService::new(remote_store_at(global_root)?))
}

/// Read the path-free coordination registry without constructing a workspace
/// runtime.
pub fn registry_snapshot_at(global_root: &Path) -> Result<RegistrySnapshotV1, OrbitError> {
    host_registry_service_at(global_root)?.snapshot()
}

/// Persist a broker outcome into the config-resolved global audit database.
pub fn record_global_audit_event_at(
    global_root: &Path,
    params: &AuditEventInsertParams,
) -> Result<(), OrbitError> {
    let database = orbit_core::config::resolved_audit_db_path(global_root, global_root)?;
    Store::open(&database)?.insert_audit_event_record(params)
}
