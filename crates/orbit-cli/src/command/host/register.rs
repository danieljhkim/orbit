use std::collections::BTreeSet;

use clap::Args;
use orbit_core::{OrbitError, OrbitRuntime};
use orbit_remote::{HOST_IDENTITY_SCHEMA_VERSION, HostIdentity, HostMode, load_host_identity};
use orbit_remote::{host_registry_service_at, require_local_hub_identity};

use crate::command::{CommandOut, CommandOutput, Execute};

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
    fn execute(self, runtime: &OrbitRuntime) -> CommandOut {
        let labels: BTreeSet<String> = self.labels.into_iter().collect();
        let local_identity = load_host_identity(&runtime.global_root())?;
        if local_identity.mode == HostMode::Spoke {
            if self.machine_id.is_some() || self.host_id.is_some() {
                return Err(OrbitError::InvalidInput(
                    "spoke registration reads machine_id and host_id only from validated host.toml; --machine-id/--host-id overrides are forbidden"
                        .to_string(),
                ));
            }
            let record = orbit_remote::register_local_spoke(runtime, &local_identity, labels)?;
            println!(
                "registered spoke '{}' (machine_id {}) with the verified hub and refreshed the local registry cache",
                record.host_id, record.machine_id
            );
            return Ok(CommandOutput::Silent);
        }

        // Direct coordination-store administration remains hub-local.
        let local_hub = require_local_hub_identity(&runtime.global_root())?;

        let (identity, is_current_machine) = match (self.machine_id, self.host_id) {
            (Some(machine_id), Some(host_id)) if machine_id == local_hub.machine_id => {
                if host_id != local_hub.host_id {
                    return Err(OrbitError::InvalidInput(format!(
                        "explicit declaration for this hub machine_id '{}' must use host.toml host_id '{}', not '{host_id}'",
                        local_hub.machine_id, local_hub.host_id
                    )));
                }
                (local_hub.clone(), true)
            }
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
            (None, None) => (local_hub.clone(), true),
            _ => {
                return Err(OrbitError::InvalidInput(
                    "an explicit host declaration requires both --machine-id and --host-id; omit \
                     both to register this machine's host.toml identity"
                        .to_string(),
                ));
            }
        };

        let service = host_registry_service_at(&runtime.global_root())?;
        let record = if is_current_machine {
            // Registration and the singular hub snapshot identity share one
            // store transaction; neither can commit without the other.
            service.register_hub_identity(&identity, labels)?
        } else {
            service.require_configured_local_hub(&local_hub)?;
            service.register_identity(&identity, labels)?
        };

        println!(
            "registered host '{}' (machine_id {}), status {}",
            record.host_id, record.machine_id, record.status
        );
        Ok(CommandOutput::Silent)
    }
}
