//! Local-only routine placement composition over Core's scheduler kernels.
//!
//! Pin placement is built exclusively from `host.toml` and `workspaces.json`.

use std::path::Path;

use chrono::Utc;
use orbit_common::types::{OrbitError, WorkspaceStatus};
use orbit_core::routines::{
    DiscoveredWorkspaces, RoutineHostIdentity, RoutineLoadError, RoutinePlacementProjection,
    RoutinePlacementProvider, RoutineRegistryView, RoutineStatusReport, RoutineWorkspaceProvider,
    SweepOptions, SweepOutcome,
};

use orbit_registry::host_identity::{HostIdentity, load_host_identity};
use orbit_registry::workspace_registry;

use crate::remote_runtime::RemoteRuntimeFactory;

struct RemoteRoutineEnvironment {
    global_root: std::path::PathBuf,
    identity: HostIdentity,
}

impl RemoteRoutineEnvironment {
    fn load(global_root: &Path) -> Result<Self, OrbitError> {
        Ok(Self {
            global_root: global_root.to_path_buf(),
            identity: load_host_identity(global_root)?,
        })
    }

    fn local_host(&self) -> RoutineHostIdentity {
        RoutineHostIdentity {
            machine_id: self.identity.machine_id.clone(),
            host_id: self.identity.host_id.clone(),
        }
    }
}

impl RoutinePlacementProvider for RemoteRoutineEnvironment {
    fn load_routine_placement(&self) -> Result<RoutinePlacementProjection, OrbitError> {
        load_routine_placement_at(&self.global_root, &self.identity)
    }
}

/// Build the v1 routine placement view from local files only.
pub(crate) fn load_routine_placement_at(
    global_root: &Path,
    identity: &HostIdentity,
) -> Result<RoutinePlacementProjection, OrbitError> {
    let workspace_registry = workspace_registry::load_registry_from(
        &workspace_registry::registry_path_for(global_root),
    )?;
    let registry =
        RoutineRegistryView::from_workspace_registry(&workspace_registry, &identity.machine_id);
    Ok(RoutinePlacementProjection {
        local_host: RoutineHostIdentity {
            machine_id: identity.machine_id.clone(),
            host_id: identity.host_id.clone(),
        },
        registry,
    })
}

impl RoutineWorkspaceProvider for RemoteRoutineEnvironment {
    fn discover_workspaces(&self, global_root: &Path) -> Result<DiscoveredWorkspaces, OrbitError> {
        discover_registered_workspaces(global_root)
    }
}

/// Discover runnable registered checkouts for routine execution.
/// The provider delegates here so this production path can be exercised with
/// an explicit global root.
pub(crate) fn discover_registered_workspaces(
    global_root: &Path,
) -> Result<DiscoveredWorkspaces, OrbitError> {
    let registry_path = workspace_registry::registry_path_for(global_root);
    let mut registry = workspace_registry::load_registry_from(&registry_path)?;
    workspace_registry::validate_workspaces(&mut registry);
    workspace_registry::save_registry_to(&registry, &registry_path)?;

    let mut discovered = DiscoveredWorkspaces::default();
    for (workspace, checkout) in workspace_registry::local_workspaces(&registry) {
        if workspace.status != WorkspaceStatus::Active || !checkout.orbit_dir.exists() {
            continue;
        }
        match RemoteRuntimeFactory::open_registered_checkout(global_root, workspace, checkout) {
            Ok(runtime) => discovered.entries.push((workspace.clone(), runtime)),
            Err(error) => discovered.errors.push(RoutineLoadError {
                source_workspace: workspace.name.clone(),
                path: Some(checkout.orbit_dir.clone()),
                message: format!("failed to open workspace runtime: {error}"),
            }),
        }
    }
    Ok(discovered)
}

pub fn routine_statuses(global_root: &Path) -> Result<RoutineStatusReport, OrbitError> {
    let environment = RemoteRoutineEnvironment::load(global_root)?;
    orbit_core::routines::routine_statuses_with_providers(
        global_root,
        &environment,
        &environment,
        Utc::now(),
    )
}

pub fn run_sweep(options: SweepOptions) -> Result<SweepOutcome, OrbitError> {
    let global_root = workspace_registry::global_orbit_dir()?;
    let environment = RemoteRoutineEnvironment::load(&global_root)?;
    orbit_core::routines::run_sweep_with_providers(
        options,
        environment.local_host(),
        &environment,
        &environment,
    )
}

pub fn run_sweep_at(global_root: &Path, options: SweepOptions) -> Result<SweepOutcome, OrbitError> {
    let environment = RemoteRoutineEnvironment::load(global_root)?;
    orbit_core::routines::run_sweep_at_with_providers(
        global_root,
        options,
        environment.local_host(),
        &environment,
        &environment,
    )
}
