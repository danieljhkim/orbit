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

    let id_width = registry
        .workspaces
        .iter()
        .map(|workspace| workspace.id.chars().count())
        .max()
        .unwrap_or_default()
        .max("ID".len());
    let mut output = format!(
        "{:<20} {:<id_width$} {:<8} {:<10} {:<22} {:<8} ROOT\n",
        "NAME",
        "ID",
        "STATUS",
        "SHIP MODE",
        "OWNER",
        "ROLE",
        id_width = id_width
    );
    for workspace in &registry.workspaces {
        let checkout = workspace_registry::find_checkout(registry, &workspace.id);
        let root = checkout
            .map(|checkout| checkout.repo_root.display().to_string())
            .unwrap_or_else(|| "-".to_string());
        // Owner and local role are visible in multi-host output and render as
        // "-" in the standalone case where neither is declared.
        let owner = workspace.owner_machine_id.as_deref().unwrap_or("-");
        let role = checkout
            .and_then(|checkout| checkout.role)
            .map(|role| role.to_string())
            .unwrap_or_else(|| "-".to_string());
        output.push_str(&format!(
            "{:<20} {:<id_width$} {:<8} {:<10} {:<22} {:<8} {}\n",
            workspace.name,
            workspace.id,
            workspace.status,
            orbit_core::resolved_ship_mode(workspace).as_input_value(),
            owner,
            role,
            root,
            id_width = id_width
        ));
    }
    output
}
