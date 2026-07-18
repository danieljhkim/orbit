use std::collections::BTreeSet;

use orbit_common::types::{HostNameResolution, HostStatus};
use orbit_store::Store;

use super::HostRegistryService;
use crate::routines::{HOST_IDENTITY_SCHEMA_VERSION, HostIdentity, HostMode};

fn identity(machine_id: &str, host_id: &str, mode: HostMode) -> HostIdentity {
    HostIdentity {
        schema_version: HOST_IDENTITY_SCHEMA_VERSION,
        machine_id: machine_id.to_string(),
        host_id: host_id.to_string(),
        mode,
    }
}

#[test]
fn service_registers_stable_identity_and_preserves_typed_lifecycle_results() {
    let service = HostRegistryService::new(Store::open_in_memory().expect("store"));
    let hub = identity("hm_hub", "hub", HostMode::Hub);
    let spoke = identity("hm_spoke", "spoke", HostMode::Spoke);

    let registered = service
        .register_identity(&hub, BTreeSet::from(["codex".to_string()]))
        .expect("register hub");
    assert_eq!(registered.machine_id, hub.machine_id);
    assert_eq!(registered.host_id, hub.host_id);
    assert_eq!(registered.status, HostStatus::Active);
    assert_eq!(
        service
            .register_identity(&hub, BTreeSet::from(["codex".to_string()]))
            .expect("idempotent registration"),
        registered
    );
    service
        .register_identity(&spoke, BTreeSet::new())
        .expect("register spoke");

    service.rename("hm_spoke", "worker").expect("rename");
    match service.resolve("spoke").expect("resolve alias") {
        HostNameResolution::Alias { host, alias } => {
            assert_eq!(host.host_id, "worker");
            assert_eq!(alias.alias_host_id, "spoke");
        }
        other => panic!("expected alias, got {other:?}"),
    }
    service.retire("hm_spoke").expect("retire");
    assert_eq!(
        service
            .active_hosts()
            .expect("active hosts")
            .iter()
            .map(|host| host.host_id.as_str())
            .collect::<Vec<_>>(),
        vec!["hub"]
    );
    assert_eq!(service.aliases("hm_spoke").expect("aliases").len(), 1);
}
