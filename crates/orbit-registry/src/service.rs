//! Registry service composition over a caller-supplied Store.

use orbit_common::types::OrbitError;
use orbit_store::Store;

use crate::{HostRegistryService, RegistryStore};

/// Open the host registry service over a Store already resolved by the
/// executable composition layer.
pub fn host_registry_service(store: Store) -> Result<HostRegistryService, OrbitError> {
    Ok(HostRegistryService::new(RegistryStore::from_store(store)?))
}
