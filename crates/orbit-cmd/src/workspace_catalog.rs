//! Registry-backed resolution for Core's federated-search scope.
//!
//! Core owns the fan-out, fusion, and attribution; it deliberately owns no
//! workspace catalog, because `orbit-registry` sits above it in the crate
//! graph. This module is the composition point that closes that gap: it turns
//! a [`WorkspaceScope`] into registered checkouts and opens a runtime for one
//! [ORB-11027].
//!
//! It adds no dependency edge — `orbit-cmd` already joins Core to Registry.

use std::path::{Path, PathBuf};

use orbit_common::OrbitError;
use orbit_core::{FederatedWorkspaceTarget, OrbitRuntime, WorkspaceCatalog, WorkspaceScope};
use orbit_registry::workspace_registry;
use orbit_types::workspace::{Workspace, WorkspaceCheckout, WorkspaceRegistry, WorkspaceStatus};

use crate::registry_runtime::RegisteredRuntimeFactory;

/// Resolves federated scope against this machine's workspace registry.
#[derive(Debug, Clone)]
pub struct RegistryWorkspaceCatalog {
    global_root: PathBuf,
}

impl RegistryWorkspaceCatalog {
    pub fn new(global_root: impl Into<PathBuf>) -> Self {
        Self {
            global_root: global_root.into(),
        }
    }

    fn load_registry(&self) -> Result<WorkspaceRegistry, OrbitError> {
        workspace_registry::load_registry_from(&workspace_registry::registry_path_for(
            &self.global_root,
        ))
    }
}

impl WorkspaceCatalog for RegistryWorkspaceCatalog {
    fn resolve_scope(
        &self,
        scope: &WorkspaceScope,
    ) -> Result<Vec<FederatedWorkspaceTarget>, OrbitError> {
        let registry = self.load_registry()?;
        let selected = match scope {
            // Core never asks a catalog to resolve its own checkout.
            WorkspaceScope::Current => Vec::new(),
            WorkspaceScope::AllRegistered => workspace_registry::local_workspaces(&registry)
                .filter(|(workspace, _)| workspace.status == WorkspaceStatus::Active)
                .map(|(workspace, checkout)| target_for(workspace, checkout))
                .collect(),
            // A selector is resolved through the same fail-closed grammar the
            // `--workspace` binder uses, so an unknown or ambiguous name is
            // named rather than quietly dropped from the scope.
            WorkspaceScope::Selectors(selectors) => selectors
                .iter()
                .map(|selector| {
                    RegisteredRuntimeFactory::resolve_workspace_selector(
                        &self.global_root,
                        selector,
                    )
                    .map(|selected| target_for(&selected.workspace, &selected.checkout))
                })
                .collect::<Result<Vec<_>, OrbitError>>()?,
        };
        Ok(dedupe_by_workspace_id(selected))
    }

    fn open(&self, target: &FederatedWorkspaceTarget) -> Result<OrbitRuntime, OrbitError> {
        let registry = self.load_registry()?;
        let (workspace, checkout) = registry
            .workspaces
            .iter()
            .find(|workspace| workspace.id == target.workspace_id)
            .zip(
                registry
                    .checkouts
                    .iter()
                    .find(|checkout| checkout.workspace_id == target.workspace_id),
            )
            .ok_or_else(|| {
                OrbitError::WorkspaceError(format!(
                    "workspace '{}' is no longer registered on this machine",
                    target.name
                ))
            })?;
        RegisteredRuntimeFactory::open_registered_checkout(&self.global_root, workspace, checkout)
    }
}

fn target_for(workspace: &Workspace, checkout: &WorkspaceCheckout) -> FederatedWorkspaceTarget {
    FederatedWorkspaceTarget {
        workspace_id: workspace.id.clone(),
        name: workspace.name.clone(),
        repo_root: checkout.repo_root.clone(),
    }
}

/// Two selectors can name the same workspace (`ws_*`, name, and path all
/// resolve to one checkout). Opening it twice would double-count its hits in
/// the fused list, so the first mention wins and order is preserved.
fn dedupe_by_workspace_id(targets: Vec<FederatedWorkspaceTarget>) -> Vec<FederatedWorkspaceTarget> {
    let mut seen = std::collections::BTreeSet::new();
    targets
        .into_iter()
        .filter(|target| seen.insert(target.workspace_id.clone()))
        .collect()
}

/// Attach registry-backed federated search to a runtime this crate opened.
pub(crate) fn attach(runtime: OrbitRuntime, global_root: &Path) -> OrbitRuntime {
    runtime.with_workspace_catalog(std::sync::Arc::new(RegistryWorkspaceCatalog::new(
        global_root,
    )))
}
