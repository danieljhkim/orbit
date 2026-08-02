use clap::Args;
use orbit_common::types::{Workspace, WorkspaceCheckout};
use orbit_core::OrbitRuntime;
use orbit_remote::workspace_registry;
use serde_json::{Value, json};

use crate::command::{CommandOut, Execute, Payload};

#[derive(Args)]
pub struct WorkspaceShowArgs {}

impl Execute for WorkspaceShowArgs {
    fn execute(self, runtime: &OrbitRuntime) -> CommandOut {
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
            Some((workspace, checkout)) => Ok(Payload::detail(
                workspace_show_json(workspace, checkout),
                format_workspace_show(workspace, checkout),
            )
            .into()),
            // An unregistered root is still a describable state, not an error:
            // the record says which root it is and that it is not registered.
            None => Ok(Payload::detail(
                json!({
                    "orbit_root": data_root.to_string_lossy(),
                    "registered": false,
                    "workspace": Value::Null,
                    "checkout": Value::Null,
                }),
                format!(
                    "current orbit root: {}\n(not registered as a workspace)",
                    data_root.display()
                ),
            )
            .into()),
        }
    }
}

/// `workspace show` had no machine-readable form; this is the record behind
/// the text below, added with the rest of the payload conversion (ORB-10586).
/// Both halves are built from the same two structs, so they cannot disagree.
pub(super) fn workspace_show_json(workspace: &Workspace, checkout: &WorkspaceCheckout) -> Value {
    json!({
        "orbit_root": checkout.orbit_dir.to_string_lossy(),
        "registered": true,
        "workspace": {
            "id": workspace.id,
            "name": workspace.name,
            "base_branch": workspace.base_branch,
            "ship_mode": orbit_core::resolved_ship_mode(workspace).as_input_value(),
            "status": workspace.status.to_string(),
            "owner_machine_id": workspace.owner_machine_id,
        },
        "checkout": {
            "repo_root": checkout.repo_root.to_string_lossy(),
            "orbit_dir": checkout.orbit_dir.to_string_lossy(),
            "role": checkout.role.map(|role| role.to_string()),
            "owner_machine_id": checkout.owner_machine_id,
        },
    })
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
    output.push_str(&format!(
        "owner:       {}\nrole:        {}\n",
        workspace.owner_machine_id.as_deref().unwrap_or("-"),
        checkout
            .role
            .map(|role| role.to_string())
            .unwrap_or_else(|| "-".to_string()),
    ));
    if let Some(replica_owner) = &checkout.owner_machine_id {
        output.push_str(&format!("owner_mirror: {replica_owner}\n"));
    }
    if let Some(remote) = &workspace.git_remote {
        output.push_str(&format!("git_remote:  {remote}\n"));
    }
    output.push_str(&format!(
        "created_at:  {}\nupdated_at:  {}\n",
        workspace.created_at, workspace.updated_at
    ));
    output
}
