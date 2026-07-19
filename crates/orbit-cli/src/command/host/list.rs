use clap::Args;
use orbit_core::{OrbitError, OrbitRuntime};
use orbit_remote::{host_registry_service_at, require_local_hub_identity};

use crate::command::Execute;

#[derive(Args)]
#[command(about = "List registered hosts from the canonical hub snapshot")]
pub struct HostListArgs {}

impl Execute for HostListArgs {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        let local_hub = require_local_hub_identity(&runtime.global_root())?;
        let service = host_registry_service_at(&runtime.global_root())?;
        service.require_configured_local_hub(&local_hub)?;
        let snapshot = service.snapshot()?;
        print!(
            "{}",
            format_host_list(&snapshot.hosts, snapshot.hub_machine_id.as_deref())
        );
        Ok(())
    }
}

pub(super) fn format_host_list(
    hosts: &[orbit_common::types::RegistryHostV1],
    hub_machine_id: Option<&str>,
) -> String {
    if hosts.is_empty() {
        return "no hosts registered\n".to_string();
    }
    let mut output = format!(
        "{:<20} {:<22} {:<8} {:<6} LABELS\n",
        "HOST", "MACHINE ID", "STATUS", "HUB"
    );
    for host in hosts {
        let hub = if hub_machine_id == Some(host.machine_id.as_str()) {
            "yes"
        } else {
            "-"
        };
        let labels = if host.labels.is_empty() {
            "-".to_string()
        } else {
            host.labels.iter().cloned().collect::<Vec<_>>().join(",")
        };
        output.push_str(&format!(
            "{:<20} {:<22} {:<8} {:<6} {}\n",
            host.host_id, host.machine_id, host.status, hub, labels
        ));
    }
    output
}
