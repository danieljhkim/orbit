//! Runtime-owned access to the workspace-partitioned friction repository.

use orbit_common::OrbitError;
use orbit_store::compose::workspace_friction_store_from_path;
use orbit_store::contracts::FrictionStoreBackend;
use std::sync::Arc;

use crate::OrbitRuntime;

/// Open the friction repository scoped by this runtime's workspace identity.
pub(crate) fn store_for(
    runtime: &OrbitRuntime,
) -> Result<Arc<dyn FrictionStoreBackend>, OrbitError> {
    workspace_friction_store_from_path(
        &runtime.context.persistence().audit_db,
        runtime.workspace_id()?,
        runtime.data_root().join("frictions"),
    )
}
