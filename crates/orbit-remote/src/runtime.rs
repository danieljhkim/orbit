//! Registry-aware composition over Core's neutral runtime seams.

use std::path::{Path, PathBuf};

use orbit_common::types::{
    OrbitError, Workspace, WorkspaceCheckout, WorkspaceCheckoutRole, WorkspaceRegistry,
};
use orbit_core::command::knowledge_policy::KnowledgeOwnerAccess;
use orbit_core::runtime::{
    OrbitRuntimeRoots, ResolvedOrbitRoots, WorkspaceRootHint, WorkspaceRuntimeBinding,
};
use orbit_core::{OrbitRuntime, resolved_ship_mode};
use orbit_store::workspace_id_for_orbit_dir;

use crate::workspace_registry;

/// Remote workspace metadata keeps the logical catalog ID distinct from the
/// task/runtime ID stored in `.orbit/config.yaml`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedWorkspaceBinding {
    pub logical_workspace_id: String,
    pub runtime: WorkspaceRuntimeBinding,
    pub role: Option<WorkspaceCheckoutRole>,
    pub owner_machine_id: Option<String>,
}

/// Build Core's authoritative runtime binding for a registered checkout.
/// The runtime ID deliberately comes from config.yaml rather than the logical
/// registry record because legacy installations may validly differ (L-0098).
pub fn workspace_runtime_binding(
    workspace: &Workspace,
    checkout: &WorkspaceCheckout,
) -> Result<WorkspaceRuntimeBinding, OrbitError> {
    Ok(WorkspaceRuntimeBinding {
        workspace_id: workspace_id_for_orbit_dir(&checkout.orbit_dir)?,
        repo_root: checkout.repo_root.clone(),
        ship_mode: resolved_ship_mode(workspace),
    })
}

pub fn resolved_workspace_binding(
    workspace: &Workspace,
    checkout: &WorkspaceCheckout,
) -> Result<ResolvedWorkspaceBinding, OrbitError> {
    Ok(ResolvedWorkspaceBinding {
        logical_workspace_id: workspace.id.clone(),
        runtime: workspace_runtime_binding(workspace, checkout)?,
        role: checkout.role,
        owner_machine_id: workspace.owner_machine_id.clone(),
    })
}

/// Registry-aware runtime factory. Registered checkouts carry an explicit
/// Core workspace binding.
pub struct RemoteRuntimeFactory;

impl RemoteRuntimeFactory {
    pub fn resolve_roots_for_cwd(
        cwd: &Path,
        root_override: Option<&Path>,
    ) -> Result<OrbitRuntimeRoots, OrbitError> {
        let hint = workspace_root_hint(cwd);
        OrbitRuntime::resolve_roots_for_cwd_with_hint(cwd, root_override, hint.as_ref())
    }

    pub fn resolve_bootstrap_roots_for_cwd(
        cwd: &Path,
        root_override: Option<&Path>,
    ) -> Result<OrbitRuntimeRoots, OrbitError> {
        let hint = workspace_root_hint(cwd);
        OrbitRuntime::resolve_bootstrap_roots_for_cwd_with_hint(cwd, root_override, hint.as_ref())
    }

    pub fn try_resolve_initialized_roots(
        cwd: &Path,
        root_override: Option<&Path>,
    ) -> Result<Option<ResolvedOrbitRoots>, OrbitError> {
        let hint = workspace_root_hint(cwd);
        orbit_core::runtime::try_resolve_initialized_roots_with_hint(
            cwd,
            root_override,
            hint.as_ref(),
        )
    }

    pub fn initialize_with_root_override(
        root_override: Option<&Path>,
    ) -> Result<OrbitRuntime, OrbitError> {
        let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        let roots = Self::resolve_roots_for_cwd(&cwd, root_override)?;
        let binding = binding_for_roots(&roots)?;
        let access = knowledge_access_for_roots(&roots)?;
        OrbitRuntime::initialize_from_resolved_roots(roots, binding)
            .map(|runtime| runtime.with_knowledge_owner_access(access))
    }

    pub fn open_resolved_roots(roots: OrbitRuntimeRoots) -> Result<OrbitRuntime, OrbitError> {
        let binding = binding_for_roots(&roots)?;
        let access = knowledge_access_for_roots(&roots)?;
        let runtime = match binding {
            Some(binding) => OrbitRuntime::from_resolved_roots_with_binding(
                &roots.global_root,
                &roots.shared_root,
                &roots.local_root,
                binding,
            )?,
            None => OrbitRuntime::from_resolved_roots(
                &roots.global_root,
                &roots.shared_root,
                &roots.local_root,
            )?,
        };
        Ok(runtime.with_knowledge_owner_access(access))
    }

    pub fn open_registered_checkout(
        global_root: &Path,
        workspace: &Workspace,
        checkout: &WorkspaceCheckout,
    ) -> Result<OrbitRuntime, OrbitError> {
        let binding = workspace_runtime_binding(workspace, checkout)?;
        let access = knowledge_access(global_root, workspace, checkout)?;
        OrbitRuntime::from_roots_with_binding(global_root, &checkout.orbit_dir, binding)
            .map(|runtime| runtime.with_knowledge_owner_access(access))
    }

    pub fn open_resolved_checkout(
        global_root: &Path,
        shared_root: &Path,
        local_root: &Path,
        binding: WorkspaceRuntimeBinding,
    ) -> Result<OrbitRuntime, OrbitError> {
        let access = knowledge_access_for_shared_root(global_root, shared_root)?;
        OrbitRuntime::from_resolved_roots_with_binding(
            global_root,
            shared_root,
            local_root,
            binding,
        )
        .map(|runtime| runtime.with_knowledge_owner_access(access))
    }
}

fn workspace_root_hint(cwd: &Path) -> Option<WorkspaceRootHint> {
    let registry = workspace_registry::load_registry().ok()?;
    let checkout = workspace_registry::find_checkout_by_path(&registry, cwd)?;
    Some(WorkspaceRootHint {
        orbit_dir: checkout.orbit_dir.clone(),
    })
}

fn binding_for_roots(
    roots: &OrbitRuntimeRoots,
) -> Result<Option<WorkspaceRuntimeBinding>, OrbitError> {
    let registry_path = workspace_registry::registry_path_for(&roots.global_root);
    let registry = workspace_registry::load_registry_from(&registry_path)?;
    binding_for_registry_roots(&registry, &roots.shared_root)
}

fn knowledge_access_for_roots(
    roots: &OrbitRuntimeRoots,
) -> Result<KnowledgeOwnerAccess, OrbitError> {
    knowledge_access_for_shared_root(&roots.global_root, &roots.shared_root)
}

fn knowledge_access_for_shared_root(
    global_root: &Path,
    shared_root: &Path,
) -> Result<KnowledgeOwnerAccess, OrbitError> {
    let registry_path = workspace_registry::registry_path_for(global_root);
    let registry = workspace_registry::load_registry_from(&registry_path)?;
    let shared = std::fs::canonicalize(shared_root).unwrap_or_else(|_| shared_root.to_path_buf());
    for (workspace, checkout) in workspace_registry::local_workspaces(&registry) {
        let registered = std::fs::canonicalize(&checkout.orbit_dir)
            .unwrap_or_else(|_| checkout.orbit_dir.clone());
        if registered == shared {
            return knowledge_access(global_root, workspace, checkout);
        }
    }
    Ok(KnowledgeOwnerAccess::Standalone)
}

fn knowledge_access(
    global_root: &Path,
    workspace: &Workspace,
    checkout: &WorkspaceCheckout,
) -> Result<KnowledgeOwnerAccess, OrbitError> {
    let identity = crate::inspect_host_identity(global_root)?;
    let crate::HostIdentityState::Present(identity) = identity else {
        return Ok(KnowledgeOwnerAccess::Standalone);
    };
    if identity.mode == crate::HostMode::Standalone {
        return Ok(KnowledgeOwnerAccess::Standalone);
    }
    let owner_machine_id = workspace.owner_machine_id.clone().ok_or_else(|| {
        OrbitError::InvalidInput(format!(
            "workspace '{}' has no declared owner in {} mode",
            workspace.id, identity.mode
        ))
    })?;
    match checkout.role {
        Some(WorkspaceCheckoutRole::Owner) if owner_machine_id == identity.machine_id => {
            Ok(KnowledgeOwnerAccess::Owner { owner_machine_id })
        }
        Some(WorkspaceCheckoutRole::Replica) => {
            Ok(KnowledgeOwnerAccess::Replica { owner_machine_id })
        }
        _ => Ok(KnowledgeOwnerAccess::Unavailable { owner_machine_id }),
    }
}

/// Runtime-free owner guard for compatibility commands that must inspect or
/// migrate a checkout before opening its normal runtime.
pub fn require_local_knowledge_owner(
    global_root: &Path,
    selected_path: &Path,
) -> Result<(), OrbitError> {
    match crate::inspect_host_identity(global_root)? {
        crate::HostIdentityState::Present(identity)
            if identity.mode != crate::HostMode::Standalone => {}
        _ => return Ok(()),
    }
    let registry = workspace_registry::load_registry_from(&workspace_registry::registry_path_for(
        global_root,
    ))?;
    let checkout =
        workspace_registry::find_checkout_by_path(&registry, selected_path).ok_or_else(|| {
            OrbitError::InvalidInput(format!(
                "knowledge operation path '{}' is not inside a registered checkout",
                selected_path.display()
            ))
        })?;
    let workspace = registry
        .workspaces
        .iter()
        .find(|workspace| workspace.id == checkout.workspace_id)
        .ok_or_else(|| {
            OrbitError::InvalidInput(format!(
                "knowledge checkout names unknown workspace '{}'",
                checkout.workspace_id
            ))
        })?;
    match knowledge_access(global_root, workspace, checkout)? {
        KnowledgeOwnerAccess::Standalone | KnowledgeOwnerAccess::Owner { .. } => Ok(()),
        KnowledgeOwnerAccess::Replica { owner_machine_id }
        | KnowledgeOwnerAccess::Unavailable { owner_machine_id } => {
            Err(OrbitError::InvalidInput(format!(
                "current-state unavailable; owner={owner_machine_id}; replica layout mutation is forbidden"
            )))
        }
    }
}

fn binding_for_registry_roots(
    registry: &WorkspaceRegistry,
    shared_root: &Path,
) -> Result<Option<WorkspaceRuntimeBinding>, OrbitError> {
    let shared = std::fs::canonicalize(shared_root).unwrap_or_else(|_| shared_root.to_path_buf());
    for (workspace, checkout) in workspace_registry::local_workspaces(registry) {
        let registered = std::fs::canonicalize(&checkout.orbit_dir)
            .unwrap_or_else(|_| checkout.orbit_dir.clone());
        if registered == shared {
            return workspace_runtime_binding(workspace, checkout).map(Some);
        }
    }
    Ok(None)
}
