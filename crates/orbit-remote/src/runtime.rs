//! Registry-aware composition over Core's neutral runtime seams.

use std::path::{Path, PathBuf};

use orbit_common::types::{
    OrbitError, Workspace, WorkspaceCheckout, WorkspaceCheckoutRole, WorkspaceRegistry,
    WorkspaceStatus,
};
use orbit_core::runtime::{
    OrbitRuntimeRoots, ResolvedOrbitRoots, WorkspaceRootHint, WorkspaceRuntimeBinding,
};
use orbit_core::{OrbitRuntime, resolved_ship_mode};
use orbit_store::sqlite::task_registry::{TaskRegistryStore, task_registry_path};
use orbit_store::workspace_id_for_orbit_dir;
use serde_json::Value;

use crate::{HOST_TOML_FILE, load_host_identity, workspace_registry};

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
        sync_task_prefix(&roots.global_root)?;
        let binding = binding_for_roots(&roots)?;
        let replica_owner = replica_owner_for_roots(&roots)?;
        OrbitRuntime::initialize_from_resolved_roots(roots, binding)
            .map(|runtime| runtime.with_coordination_write_owner(replica_owner))
    }

    pub fn open_resolved_roots(roots: OrbitRuntimeRoots) -> Result<OrbitRuntime, OrbitError> {
        sync_task_prefix(&roots.global_root)?;
        let binding = binding_for_roots(&roots)?;
        let replica_owner = replica_owner_for_roots(&roots)?;
        let runtime = match binding {
            Some(binding) => OrbitRuntime::from_resolved_roots_with_binding(
                &roots.global_root,
                &roots.shared_root,
                &roots.local_root,
                binding,
            ),
            None => OrbitRuntime::from_resolved_roots(
                &roots.global_root,
                &roots.shared_root,
                &roots.local_root,
            ),
        }?;
        Ok(runtime.with_coordination_write_owner(replica_owner))
    }

    pub fn open_registered_checkout(
        global_root: &Path,
        workspace: &Workspace,
        checkout: &WorkspaceCheckout,
    ) -> Result<OrbitRuntime, OrbitError> {
        sync_task_prefix(global_root)?;
        let binding = workspace_runtime_binding(workspace, checkout)?;
        OrbitRuntime::from_roots_with_binding(global_root, &checkout.orbit_dir, binding).map(
            |runtime| runtime.with_coordination_write_owner(replica_owner_for_checkout(checkout)),
        )
    }

    pub fn open_resolved_checkout(
        global_root: &Path,
        shared_root: &Path,
        local_root: &Path,
        binding: WorkspaceRuntimeBinding,
    ) -> Result<OrbitRuntime, OrbitError> {
        sync_task_prefix(global_root)?;
        OrbitRuntime::from_resolved_roots_with_binding(
            global_root,
            shared_root,
            local_root,
            binding,
        )
    }

    /// Bind a CLI `orbit tool run` invocation to the workspace named in `input`.
    ///
    /// ORB-10769: this is the single CLI-path resolver above the tools. A
    /// non-empty `workspace` either rebinds the runtime to that registered
    /// checkout or fails closed naming the selector. MCP resolution
    /// (`McpHost::resolve_workspace`) is a separate seam and still never
    /// falls back to process cwd.
    pub fn bind_cli_tool_workspace(
        runtime: &OrbitRuntime,
        input: &mut Value,
    ) -> Result<Option<OrbitRuntime>, OrbitError> {
        let Some(selector) = cli_workspace_selector(input)? else {
            return Ok(None);
        };
        let global_root = runtime.global_root();
        let registry = workspace_registry::load_registry_from(
            &workspace_registry::registry_path_for(&global_root),
        )?;
        match resolve_cli_workspace_target(&registry, runtime, &selector)? {
            CliWorkspaceTarget::CurrentRuntime => Ok(None),
            CliWorkspaceTarget::Checkout {
                workspace,
                checkout,
                rewrite_to_repo_root,
            } => {
                if workspace.status != WorkspaceStatus::Active {
                    return Err(unsupported_cli_workspace(&selector));
                }
                if rewrite_to_repo_root {
                    set_input_workspace(input, &checkout.repo_root)?;
                }
                if same_cli_checkout(runtime, checkout) {
                    return Ok(None);
                }
                Self::open_registered_checkout(&global_root, workspace, checkout).map(Some)
            }
        }
    }
}

enum CliWorkspaceTarget<'a> {
    CurrentRuntime,
    Checkout {
        workspace: &'a Workspace,
        checkout: &'a WorkspaceCheckout,
        rewrite_to_repo_root: bool,
    },
}

fn cli_workspace_selector(input: &Value) -> Result<Option<String>, OrbitError> {
    match input.get("workspace") {
        None => Ok(None),
        Some(Value::Null) => Ok(None),
        Some(Value::String(value)) => {
            let trimmed = value.trim();
            if trimmed.is_empty() {
                return Ok(None);
            }
            Ok(Some(trimmed.to_string()))
        }
        Some(_) => Err(OrbitError::InvalidInput(
            "`workspace` must be a string".to_string(),
        )),
    }
}

fn resolve_cli_workspace_target<'a>(
    registry: &'a WorkspaceRegistry,
    runtime: &OrbitRuntime,
    selector: &str,
) -> Result<CliWorkspaceTarget<'a>, OrbitError> {
    if selector_looks_like_path(selector) {
        return resolve_cli_workspace_path(registry, runtime, selector);
    }
    let checkout = workspace_registry::find_checkout(registry, selector)
        .ok_or_else(|| unsupported_cli_workspace(selector))?;
    let workspace = workspace_registry::find_workspace(registry, &checkout.workspace_id)
        .ok_or_else(|| unsupported_cli_workspace(selector))?;
    Ok(CliWorkspaceTarget::Checkout {
        workspace,
        checkout,
        rewrite_to_repo_root: true,
    })
}

fn resolve_cli_workspace_path<'a>(
    registry: &'a WorkspaceRegistry,
    runtime: &OrbitRuntime,
    selector: &str,
) -> Result<CliWorkspaceTarget<'a>, OrbitError> {
    let raw = Path::new(selector);
    let candidate = if raw.is_absolute() {
        raw.to_path_buf()
    } else {
        std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join(raw)
    };
    let canonical = candidate
        .canonicalize()
        .map_err(|_| unsupported_cli_workspace(selector))?;
    if !canonical.is_dir() {
        return Err(unsupported_cli_workspace(selector));
    }
    if let Some(checkout) = find_checkout_for_canonical_path(registry, &canonical) {
        let workspace = workspace_registry::find_workspace(registry, &checkout.workspace_id)
            .ok_or_else(|| unsupported_cli_workspace(selector))?;
        return Ok(CliWorkspaceTarget::Checkout {
            workspace,
            checkout,
            rewrite_to_repo_root: false,
        });
    }
    if path_is_inside(&runtime.paths().repo_root, &canonical) {
        return Ok(CliWorkspaceTarget::CurrentRuntime);
    }
    Err(unsupported_cli_workspace(selector))
}

fn find_checkout_for_canonical_path<'a>(
    registry: &'a WorkspaceRegistry,
    canonical: &Path,
) -> Option<&'a WorkspaceCheckout> {
    registry.checkouts.iter().find(|checkout| {
        canonical_path(&checkout.repo_root) == canonical
            || canonical_path(&checkout.orbit_dir) == canonical
            || checkout
                .path_overrides
                .iter()
                .any(|override_path| canonical_path(override_path) == canonical)
    })
}

fn same_cli_checkout(runtime: &OrbitRuntime, checkout: &WorkspaceCheckout) -> bool {
    canonical_path(&runtime.paths().repo_root) == canonical_path(&checkout.repo_root)
}

fn path_is_inside(parent: &Path, child: &Path) -> bool {
    child.starts_with(canonical_path(parent))
}

fn canonical_path(path: &Path) -> PathBuf {
    path.canonicalize().unwrap_or_else(|_| path.to_path_buf())
}

fn selector_looks_like_path(selector: &str) -> bool {
    let path = Path::new(selector);
    path.is_absolute()
        || selector == "."
        || selector == ".."
        || selector.contains('/')
        || selector.contains('\\')
}

fn set_input_workspace(input: &mut Value, repo_root: &Path) -> Result<(), OrbitError> {
    let Some(object) = input.as_object_mut() else {
        return Err(OrbitError::InvalidInput(
            "tool input must be a JSON object".to_string(),
        ));
    };
    object.insert(
        "workspace".to_string(),
        Value::String(repo_root.to_string_lossy().into_owned()),
    );
    Ok(())
}

fn unsupported_cli_workspace(selector: &str) -> OrbitError {
    OrbitError::InvalidInput(format!(
        "unsupported workspace '{selector}'; pass a registered workspace name or ID, or a path to a registered local checkout"
    ))
}

/// Project the host-owned task namespace into the neutral task allocator.
/// Custom/legacy roots without host.toml retain the historical ORB default;
/// once an identity exists, malformed or conflicting state fails closed.
pub(crate) fn sync_task_prefix(global_root: &Path) -> Result<(), OrbitError> {
    if !global_root.join(HOST_TOML_FILE).is_file() {
        return Ok(());
    }
    let identity = load_host_identity(global_root)?;
    let registry = TaskRegistryStore::open(&task_registry_path(global_root))?;
    registry.set_task_prefix(&identity.task_prefix)
}

fn replica_owner_for_checkout(checkout: &WorkspaceCheckout) -> Option<String> {
    (checkout.role == Some(WorkspaceCheckoutRole::Replica))
        .then(|| checkout.owner_machine_id.clone())
        .flatten()
}

fn replica_owner_for_roots(roots: &OrbitRuntimeRoots) -> Result<Option<String>, OrbitError> {
    let registry_path = workspace_registry::registry_path_for(&roots.global_root);
    let registry = workspace_registry::load_registry_from(&registry_path)?;
    let shared =
        std::fs::canonicalize(&roots.shared_root).unwrap_or_else(|_| roots.shared_root.clone());
    Ok(registry.checkouts.iter().find_map(|checkout| {
        let registered = std::fs::canonicalize(&checkout.orbit_dir)
            .unwrap_or_else(|_| checkout.orbit_dir.clone());
        (registered == shared)
            .then(|| replica_owner_for_checkout(checkout))
            .flatten()
    }))
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
