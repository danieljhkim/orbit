use clap::Args;
use orbit_core::{HostRegistryService, OrbitError, OrbitRuntime};

use crate::command::Execute;

#[derive(Args)]
#[command(about = "List active registered hosts")]
pub struct HostListArgs {}

impl Execute for HostListArgs {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        let service = HostRegistryService::new(runtime.sqlite_store()?);
        let hosts = service.active_hosts()?;
        let hub = service.hub_machine_id()?;
        print!("{}", format_host_list(&hosts, hub.as_deref()));
        Ok(())
    }
}

pub(super) fn format_host_list(
    hosts: &[orbit_common::types::HostRecord],
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
