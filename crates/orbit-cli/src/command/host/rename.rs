use clap::Args;
use orbit_common::types::validate_host_id;
use orbit_common::utility::fs::with_exclusive_file_lock;
use orbit_core::{OrbitError, OrbitRuntime};
use orbit_remote::workspace_registry;
use orbit_remote::{HOST_TOML_FILE, load_host_identity, rename_current_host_identity};

use crate::command::{CommandOut, CommandOutput, Execute};

#[derive(Args)]
#[command(about = "Rename this machine's local host identity")]
pub struct HostRenameArgs {
    /// Current local host name. Tombstone aliases are a dormant v2 feature.
    current_name: String,
    /// New local host name.
    new_name: String,
}

impl Execute for HostRenameArgs {
    fn execute(self, runtime: &OrbitRuntime) -> CommandOut {
        let global_root = runtime.global_root();
        let current = load_host_identity(&global_root)?;
        if self.current_name != current.host_id {
            return Err(OrbitError::InvalidInput(format!(
                "host rename is local-only in v1: current host.toml names '{}', not '{}'",
                current.host_id, self.current_name
            )));
        }
        validate_host_id(&self.new_name)?;

        let lock_target = global_root.join(HOST_TOML_FILE);
        let (renamed, workspace_records) = with_exclusive_file_lock(
            &lock_target,
            "local host rename",
            || -> Result<_, OrbitError> {
                let registry_path = workspace_registry::registry_path_for(&global_root);
                let mut registry = workspace_registry::load_registry_from(&registry_path)?;
                let affected = workspace_registry::rename_local_owner_host_id(
                    &mut registry,
                    &current.machine_id,
                    &self.new_name,
                )?;

                // Each file is crash-safe on its own. Keep the host identity as
                // the deciding write, then persist the derived local owner-name
                // projection while the shared host lock excludes another rename.
                let renamed = rename_current_host_identity(&global_root, &self.new_name)?;
                workspace_registry::save_registry_to(&registry, &registry_path)?;
                Ok((renamed, affected))
            },
        )?;

        println!(
            "renamed this machine to '{}' (machine_id {}); updated host.toml and {} local workspace owner record(s)",
            renamed.host_id, renamed.machine_id, workspace_records
        );
        Ok(CommandOutput::Silent)
    }
}
