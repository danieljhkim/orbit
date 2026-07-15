use clap::Args;
use orbit_common::types::WorkspaceRegistry;
use orbit_core::workspace_registry;
use orbit_core::{OrbitError, OrbitRuntime};

use crate::command::Execute;

#[derive(Args)]
pub struct WorkspaceListArgs {}

impl Execute for WorkspaceListArgs {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        let global_root = runtime.global_root();
        let registry_path = workspace_registry::registry_path_for(&global_root);
        let mut registry = workspace_registry::load_registry_from(&registry_path)?;
        workspace_registry::validate_workspaces(&mut registry);

        if registry.workspaces.is_empty() {
            print!("{}", format_workspace_list(&registry));
            return Ok(());
        }

        // Save back if staleness changed any status
        workspace_registry::save_registry_to(&registry, &registry_path)?;
        print!("{}", format_workspace_list(&registry));
        Ok(())
    }
}

pub(super) fn format_workspace_list(registry: &WorkspaceRegistry) -> String {
    if registry.workspaces.is_empty() {
        return "no workspaces registered\n".to_string();
    }

    let mut output = format!(
        "{:<20} {:<12} {:<8} {:<10} ROOT\n",
        "NAME", "ID", "STATUS", "SHIP MODE"
    );
    for workspace in &registry.workspaces {
        output.push_str(&format!(
            "{:<20} {:<12} {:<8} {:<10} {}\n",
            workspace.name,
            workspace.id,
            workspace.status,
            orbit_core::resolved_ship_mode(workspace).as_input_value(),
            workspace.root.display()
        ));
    }
    output
}
