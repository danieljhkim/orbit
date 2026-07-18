use clap::{Args, Subcommand};
use orbit_common::types::HostNameResolution;
use orbit_core::{HostRegistryService, OrbitError, OrbitRuntime};

use crate::command::Execute;

use super::list::HostListArgs;
use super::register::HostRegisterArgs;
use super::rename::HostRenameArgs;
use super::retire::HostRetireArgs;

#[derive(Args)]
#[command(about = "Register and manage hub hosts")]
pub struct HostCommand {
    #[command(subcommand)]
    pub command: HostSubcommand,
}

impl Execute for HostCommand {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        self.command.execute(runtime)
    }
}

#[derive(Subcommand)]
pub enum HostSubcommand {
    /// Register this machine's `host.toml` identity, or an explicit remote host declaration
    Register(HostRegisterArgs),
    /// List active registered hosts
    List(HostListArgs),
    /// Rename a host (and this machine's local `host.toml` when it is the target)
    Rename(HostRenameArgs),
    /// Retire a host without deleting its identity or aliases
    Retire(HostRetireArgs),
}

impl Execute for HostSubcommand {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        match self {
            HostSubcommand::Register(args) => args.execute(runtime),
            HostSubcommand::List(args) => args.execute(runtime),
            HostSubcommand::Rename(args) => args.execute(runtime),
            HostSubcommand::Retire(args) => args.execute(runtime),
        }
    }
}

/// Resolve an operator-supplied host name to its stable `machine_id`, failing
/// with an actionable message for unknown and collision results. Retired and
/// tombstone-alias names still resolve so retirement stays idempotent and a
/// mistaken alias reuse is diagnosable.
pub(super) fn resolve_machine_id(
    service: &HostRegistryService,
    name: &str,
) -> Result<String, OrbitError> {
    match service.resolve(name)? {
        HostNameResolution::Active { host } | HostNameResolution::Alias { host, .. } => {
            Ok(host.machine_id)
        }
        HostNameResolution::Retired { host, .. } => Ok(host.machine_id),
        HostNameResolution::Unknown { host_id } => Err(OrbitError::InvalidInput(format!(
            "host name '{host_id}' is not a registered host"
        ))),
        HostNameResolution::Collision {
            host_id,
            machine_ids,
        } => Err(OrbitError::InvalidInput(format!(
            "host name '{host_id}' is ambiguous across machine_ids [{}]; refusing to act",
            machine_ids.join(", ")
        ))),
    }
}
