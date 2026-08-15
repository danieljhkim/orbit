use clap::Args;
use orbit_core::{OrbitError, OrbitRuntime};
use orbit_registry::workspace_registry;
use orbit_registry::{host_registry_service, require_local_hub_identity};

use crate::command::{CommandOut, CommandOutput, Execute};

#[derive(Args)]
#[command(about = "Bind a workspace's singular owner by human host name (hub-side)")]
pub struct WorkspaceLinkArgs {
    /// Logical workspace id or name.
    workspace: String,
    /// Owner host name (an active name or a tombstone alias resolving to it).
    #[arg(long)]
    owner: String,
}

impl Execute for WorkspaceLinkArgs {
    fn execute(self, runtime: &OrbitRuntime) -> CommandOut {
        let global_root = runtime.global_root();
        let local_hub = require_local_hub_identity(&global_root)?;
        let service = host_registry_service(runtime.sqlite_store()?)?;
        service.require_configured_local_hub(&local_hub)?;
        let registry_path = workspace_registry::registry_path_for(&global_root);
        let registry = workspace_registry::load_registry_from(&registry_path)?;

        // Resolve the workspace id/name to its logical id for the store binding.
        let workspace_id = workspace_registry::find_workspace(&registry, &self.workspace)
            .map(|workspace| workspace.id.clone())
            .ok_or_else(|| {
                OrbitError::InvalidInput(format!("unknown workspace '{}'", self.workspace))
            })?;
        let link = service.link_workspace_owner(&registry, &workspace_id, &self.owner)?;

        if let Some(warning) = &link.warning {
            eprintln!("warning: {warning}");
        }
        println!(
            "workspace '{}' owner bound to machine_id {}",
            workspace_id, link.ownership.owner_machine_id
        );
        Ok(CommandOutput::Silent)
    }
}
