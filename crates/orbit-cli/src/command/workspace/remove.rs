use clap::Args;
use orbit_core::workspace_registry;
use orbit_core::{OrbitError, OrbitRuntime};

use crate::command::Execute;

#[derive(Args)]
pub struct WorkspaceRemoveArgs {
    /// Workspace name or id
    pub workspace: String,
}

impl Execute for WorkspaceRemoveArgs {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        let global_root = runtime.global_root();
        let registry_path = workspace_registry::registry_path_for(&global_root);
        let mut registry = workspace_registry::load_registry_from(&registry_path)?;
        let removed = workspace_registry::remove_workspace(&mut registry, &self.workspace)?;
        workspace_registry::save_registry_to(&registry, &registry_path)?;
        println!("workspace '{}' removed from registry", removed.name);
        Ok(())
    }
}
