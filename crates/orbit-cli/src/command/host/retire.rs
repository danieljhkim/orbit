use clap::Args;
use orbit_core::OrbitRuntime;
use orbit_remote::{host_registry_service_at, require_local_hub_identity};

use crate::command::{CommandOut, CommandOutput, Execute, require_confirmation};

use super::command::resolve_machine_id;

#[derive(Args)]
#[command(about = "Retire a host without deleting its identity or aliases")]
pub struct HostRetireArgs {
    /// Host name (or a tombstone alias resolving to the machine) to retire.
    name: String,
    /// Confirm irreversible host retirement
    #[arg(long)]
    confirm: bool,
}

impl Execute for HostRetireArgs {
    fn execute(self, runtime: &OrbitRuntime) -> CommandOut {
        require_confirmation(self.confirm, "host retirement")?;
        let local_hub = require_local_hub_identity(&runtime.global_root())?;
        let service = host_registry_service_at(&runtime.global_root())?;
        service.require_configured_local_hub(&local_hub)?;
        let machine_id = resolve_machine_id(&service, &self.name)?;
        // The singular configured hub cannot retire itself in v1; the guard
        // rejects it before any database mutation.
        let record = service.retire_guarding_hub(&machine_id)?;
        println!(
            "retired host '{}' (machine_id {}); identity and aliases are preserved",
            record.host_id, record.machine_id
        );
        Ok(CommandOutput::Silent)
    }
}
