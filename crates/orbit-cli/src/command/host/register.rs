use std::collections::BTreeSet;

use clap::Args;
use orbit_core::routines::{
    HOST_IDENTITY_SCHEMA_VERSION, HostIdentity, HostMode, load_host_identity,
};
use orbit_core::{HostRegistryService, OrbitError, OrbitRuntime};

use crate::command::Execute;

#[derive(Args)]
#[command(
    about = "Register this machine's host.toml identity, or an explicit remote host declaration"
)]
pub struct HostRegisterArgs {
    /// Explicit stable machine_id for a remote host declaration (requires --host-id).
    /// Never generated or inferred — omit both flags to register this machine.
    #[arg(long)]
    machine_id: Option<String>,
    /// Host display name for an explicit remote declaration.
    #[arg(long)]
    host_id: Option<String>,
    /// Host label (repeatable).
    #[arg(long = "label")]
    labels: Vec<String>,
}

impl Execute for HostRegisterArgs {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        let service = HostRegistryService::new(runtime.sqlite_store()?);
        let labels: BTreeSet<String> = self.labels.into_iter().collect();

        let (identity, is_current_machine) = match (self.machine_id, self.host_id) {
            (Some(machine_id), Some(host_id)) => (
                HostIdentity {
                    schema_version: HOST_IDENTITY_SCHEMA_VERSION,
                    machine_id,
                    host_id,
                    // Mode is not persisted in the hub record; a declared remote
                    // host carries no local operating mode here.
                    mode: HostMode::Standalone,
                },
                false,
            ),
            (None, None) => (load_host_identity(&runtime.global_root())?, true),
            _ => {
                return Err(OrbitError::InvalidInput(
                    "an explicit host declaration requires both --machine-id and --host-id; omit \
                     both to register this machine's host.toml identity"
                        .to_string(),
                ));
            }
        };

        let record = service.register_identity(&identity, labels)?;

        // When this machine registers itself as a hub, stamp the singular hub
        // identity so it cannot later retire itself.
        if is_current_machine && identity.mode == HostMode::Hub {
            service.configure_hub_identity(&record.machine_id)?;
        }

        println!(
            "registered host '{}' (machine_id {}), status {}",
            record.host_id, record.machine_id, record.status
        );
        Ok(())
    }
}
