use clap::Args;
use orbit_core::{OrbitError, OrbitRuntime};

use crate::command::Execute;

// ADR-0248: kept as a deliberate, human-invoked opt-in — `workspace init`
// no longer wires this up automatically.
#[derive(Args)]
pub struct HookInstallArgs;

impl Execute for HookInstallArgs {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        let providers = orbit_cmd::hook_install::install_for_runtime(runtime)?;
        if providers.is_empty() {
            println!("no hook providers auto-detected");
        } else {
            println!("installed hook integrations for {}", providers.join(", "));
        }
        Ok(())
    }
}

#[derive(Args)]
pub struct HookUninstallArgs;

impl Execute for HookUninstallArgs {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        let providers = orbit_cmd::hook_install::uninstall_for_runtime(runtime)?;
        if providers.is_empty() {
            println!("no hook integrations found");
        } else {
            println!("removed hook integrations for {}", providers.join(", "));
        }
        Ok(())
    }
}
