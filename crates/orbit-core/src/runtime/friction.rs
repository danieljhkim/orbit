//! Runtime-owned access to the workspace-partitioned friction repository.

use orbit_common::OrbitError;
use orbit_store::friction_store::FrictionStore;

use crate::OrbitRuntime;

/// Open the friction repository scoped by this runtime's workspace identity.
pub(crate) fn store_for(runtime: &OrbitRuntime) -> Result<FrictionStore, OrbitError> {
    FrictionStore::open(
        runtime.sqlite_store()?,
        runtime.workspace_id()?,
        runtime.data_root().join("frictions"),
    )
}
