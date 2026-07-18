use clap::Args;
use orbit_core::routines::{
    HostIdentityState, inspect_host_identity, rename_current_host_identity,
};
use orbit_core::{HostRegistryService, OrbitError, OrbitRuntime};

use crate::command::Execute;

use super::command::resolve_machine_id;

#[derive(Args)]
#[command(about = "Rename a host, coordinating the local host.toml when it is this machine")]
pub struct HostRenameArgs {
    /// Current host name (or a tombstone alias resolving to the machine).
    current_name: String,
    /// New host name.
    new_name: String,
}

impl Execute for HostRenameArgs {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        let global_root = runtime.global_root();
        let service = HostRegistryService::new(runtime.sqlite_store()?);
        let machine_id = resolve_machine_id(&service, &self.current_name)?;

        // Is the rename target this very machine? Only then does the local
        // host.toml participate. Renaming another machine never touches a local
        // file.
        let local_machine_id = match inspect_host_identity(&global_root)? {
            HostIdentityState::Present(identity) => Some(identity.machine_id),
            HostIdentityState::Legacy { .. } | HostIdentityState::Absent => None,
        };
        let is_current_machine = local_machine_id.as_deref() == Some(machine_id.as_str());

        if is_current_machine {
            // Validation and the staged local write happen before committing
            // the durable registry mutation. The staged render is reparsed
            // before the atomic write, so a bad name fails before any change.
            rename_current_host_identity(&global_root, &self.new_name)?;
            match service.rename(&machine_id, &self.new_name) {
                Ok(record) => {
                    println!(
                        "renamed this machine to '{}' (machine_id {}); local host.toml and the hub \
                         registry now agree",
                        record.host_id, record.machine_id
                    );
                    Ok(())
                }
                Err(error) => Err(OrbitError::InvalidInput(format!(
                    "local host.toml was updated to '{}', but the hub registry rename did not \
                     commit: {error}. The local identity and hub registry now disagree; re-run \
                     `orbit host rename` once the hub is reachable to reconcile them",
                    self.new_name
                ))),
            }
        } else {
            // Registry-only rename for another machine; its local host.toml is
            // never pretended to be updated.
            let record = service.rename(&machine_id, &self.new_name)?;
            println!(
                "renamed host to '{}' (machine_id {}); this is a remote machine, so its local \
                 host.toml was not modified",
                record.host_id, record.machine_id
            );
            Ok(())
        }
    }
}
