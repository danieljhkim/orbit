//! Logical workspace catalog and machine-local persistence.

mod catalog;
mod io;

pub use catalog::{
    WorkspaceRegistryHostContext, assign_checkout_role, find_checkout, find_checkout_by_path,
    find_workspace, find_workspace_by_path, local_workspaces, parse_workspace_registry,
    register_checkout, register_workspace, remove_workspace, rename_local_owner_host_id,
    resolve_logical_workspace, set_path_override, validate_workspace_registry, validate_workspaces,
};
pub use io::{
    global_orbit_dir, load_registry, load_registry_from, registry_path, registry_path_for,
    save_registry, save_registry_to,
};

#[cfg(test)]
pub(crate) use io::load_registry_from_with_writer;
