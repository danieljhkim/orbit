use clap::Args;
use orbit_core::{HostRegistryService, OrbitError, OrbitRuntime};

use crate::command::Execute;

use super::command::resolve_machine_id;

#[derive(Args)]
#[command(about = "Retire a host without deleting its identity or aliases")]
pub struct HostRetireArgs {
    /// Host name (or a tombstone alias resolving to the machine) to retire.
    name: String,
}

impl Execute for HostRetireArgs {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        let service = HostRegistryService::new(runtime.sqlite_store()?);
        let machine_id = resolve_machine_id(&service, &self.name)?;
        // The singular configured hub cannot retire itself in v1; the guard
        // rejects it before any database mutation.
        let record = service.retire_guarding_hub(&machine_id)?;
        println!(
            "retired host '{}' (machine_id {}); identity and aliases are preserved",
            record.host_id, record.machine_id
        );
        Ok(())
    }
}
