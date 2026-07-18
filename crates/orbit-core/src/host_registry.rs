//! Typed core service for the hub host registry [ORB-10255].
//!
//! This layer binds B1's stable local [`HostIdentity`] declaration to the
//! durable hub-store API. It intentionally does not coordinate local
//! `host.toml` renames, expose administration commands, or add transport;
//! those surfaces belong to the later registry-administration unit.

use std::collections::BTreeSet;

use orbit_common::types::{
    HostAlias, HostNameResolution, HostRecord, HostRegistration, OrbitError,
};
use orbit_store::Store;

use crate::routines::HostIdentity;

#[derive(Clone)]
pub struct HostRegistryService {
    store: Store,
}

impl HostRegistryService {
    pub fn new(store: Store) -> Self {
        Self { store }
    }

    /// Register B1's stable machine identity with a compatible label set.
    pub fn register_identity(
        &self,
        identity: &HostIdentity,
        labels: BTreeSet<String>,
    ) -> Result<HostRecord, OrbitError> {
        self.store.register_host(&HostRegistration {
            machine_id: identity.machine_id.clone(),
            host_id: identity.host_id.clone(),
            labels,
        })
    }

    pub fn rename(&self, machine_id: &str, new_host_id: &str) -> Result<HostRecord, OrbitError> {
        self.store.rename_host(machine_id, new_host_id)
    }

    pub fn retire(&self, machine_id: &str) -> Result<HostRecord, OrbitError> {
        self.store.retire_host(machine_id)
    }

    pub fn resolve(&self, host_id: &str) -> Result<HostNameResolution, OrbitError> {
        self.store.resolve_host_id(host_id)
    }

    pub fn active_hosts(&self) -> Result<Vec<HostRecord>, OrbitError> {
        self.store.list_active_hosts()
    }

    pub fn aliases(&self, machine_id: &str) -> Result<Vec<HostAlias>, OrbitError> {
        self.store.list_host_aliases(machine_id)
    }
}

#[cfg(test)]
#[path = "tests/host_registry.rs"]
mod tests;
