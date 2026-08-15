//! Dormant v2 fleet-list command implementation (ADR-0358).
//! Not linked into the v1 CLI; see `docs/design/host-registry/2_design.md` §2.1.

use clap::Args;
use orbit_core::OrbitRuntime;
use orbit_remote::{host_registry_service, require_local_hub_identity};
use serde_json::{Value, json};

use crate::output::table::{Column, Table};

use crate::command::{CommandOut, Execute, Payload};

#[derive(Args)]
#[command(about = "List registered hosts from the canonical hub snapshot")]
pub struct HostListArgs {}

impl Execute for HostListArgs {
    fn execute(self, runtime: &OrbitRuntime) -> CommandOut {
        let local_hub = require_local_hub_identity(&runtime.global_root())?;
        let service = host_registry_service(runtime.sqlite_store()?)?;
        service.require_configured_local_hub(&local_hub)?;
        let snapshot = service.snapshot()?;
        let hub_machine_id = snapshot.hub_machine_id.as_deref();
        let records = snapshot
            .hosts
            .iter()
            .map(|host| host_to_json(host, hub_machine_id))
            .collect::<Vec<_>>();
        Ok(Payload::list(records, host_table(&snapshot.hosts, hub_machine_id)).into())
    }
}

/// `host list` had no `--json` at all: its only form was a `format!` with
/// hard-coded column widths, so a host id over 20 characters ran into the next
/// column. The record shape below is the machine-readable form ORB-10586 adds;
/// the widths now come from the data (`specs/table-rendering.md`).
fn host_to_json(host: &orbit_common::types::RegistryHostV1, hub_machine_id: Option<&str>) -> Value {
    json!({
        "host_id": host.host_id,
        "machine_id": host.machine_id,
        "status": host.status,
        "hub": hub_machine_id == Some(host.machine_id.as_str()),
        "labels": host.labels.iter().cloned().collect::<Vec<_>>(),
    })
}

pub(super) fn host_table(
    hosts: &[orbit_common::types::RegistryHostV1],
    hub_machine_id: Option<&str>,
) -> Table {
    let mut table = Table::new(vec![
        Column::new("HOST").fixed(),
        Column::new("MACHINE ID").fixed(),
        Column::new("STATUS").fixed(),
        Column::new("HUB").fixed(),
        Column::new("LABELS"),
    ])
    .empty_message("no hosts registered");
    for host in hosts {
        table.add_row(vec![
            host.host_id.clone(),
            host.machine_id.clone(),
            host.status.to_string(),
            if hub_machine_id == Some(host.machine_id.as_str()) {
                "yes".to_string()
            } else {
                "-".to_string()
            },
            if host.labels.is_empty() {
                "-".to_string()
            } else {
                host.labels.iter().cloned().collect::<Vec<_>>().join(",")
            },
        ]);
    }
    table
}
