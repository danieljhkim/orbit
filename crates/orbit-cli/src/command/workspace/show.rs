use clap::Args;
use orbit_common::types::{Workspace, WorkspaceCheckout};
use orbit_core::workspace_registry;
use orbit_core::{OrbitError, OrbitRuntime};

use crate::command::Execute;

#[derive(Args)]
pub struct WorkspaceShowArgs {}

impl Execute for WorkspaceShowArgs {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        let data_root = runtime.data_root();
        let data_root_canonical = std::fs::canonicalize(&data_root).unwrap_or(data_root.clone());
        let global_root = runtime.global_root();
        let registry_path = workspace_registry::registry_path_for(&global_root);
        let registry = workspace_registry::load_registry_from(&registry_path)?;

        // Find workspace whose orbit_dir matches the current runtime's data root
        let checkout = registry.checkouts.iter().find(|checkout| {
            let ws_canonical = std::fs::canonicalize(&checkout.orbit_dir)
                .unwrap_or_else(|_| checkout.orbit_dir.clone());
            ws_canonical == data_root_canonical
        });

        match checkout.and_then(|checkout| {
            workspace_registry::find_workspace(&registry, &checkout.workspace_id)
                .map(|workspace| (workspace, checkout))
        }) {
            Some((workspace, checkout)) => {
                print!("{}", format_workspace_show(workspace, checkout));
            }
            None => {
                println!("current orbit root: {}", data_root.display());
                println!("(not registered as a workspace)");
            }
        }
        Ok(())
    }
}

pub(super) fn format_workspace_show(workspace: &Workspace, checkout: &WorkspaceCheckout) -> String {
    let mut output = format!(
        "name:        {}\nid:          {}\nroot:        {}\norbit_dir:   {}\nbase_branch: {}\nship_mode:   {}\nstatus:      {}\n",
        workspace.name,
        workspace.id,
        checkout.repo_root.display(),
        checkout.orbit_dir.display(),
        workspace.base_branch,
        orbit_core::resolved_ship_mode(workspace).as_input_value(),
        workspace.status,
    );
    if let Some(remote) = &workspace.git_remote {
        output.push_str(&format!("git_remote:  {remote}\n"));
    }
    output.push_str(&format!(
        "created_at:  {}\nupdated_at:  {}\n",
        workspace.created_at, workspace.updated_at
    ));
    output
}
