//! Versioned logical workspace catalog and machine-local checkout bindings.

use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use orbit_common::types::{
    NotFoundKind, OrbitError, WORKSPACE_REGISTRY_SCHEMA_VERSION, Workspace, WorkspaceCheckout,
    WorkspaceCheckoutRole, WorkspaceRegistry, WorkspaceStatus, validate_machine_id,
};
use orbit_common::utility::fs::atomic_write_text;
use serde::Deserialize;
use serde_json::Value;

use crate::host_identity::{HostIdentityState, inspect_host_identity};

/// Returns the global Orbit directory: `~/.orbit/`.
pub fn global_orbit_dir() -> Result<PathBuf, OrbitError> {
    orbit_core::runtime::resolve_global_root()
}

/// Returns the path to the global workspace registry file.
pub fn registry_path() -> Result<PathBuf, OrbitError> {
    Ok(registry_path_for(&global_orbit_dir()?))
}

/// Returns the workspace registry path for an already-resolved Orbit root.
pub fn registry_path_for(global_root: &Path) -> PathBuf {
    global_root.join("workspaces.json")
}

/// Loads the workspace registry from `~/.orbit/workspaces.json`.
/// Returns an empty registry if the file does not exist.
pub fn load_registry() -> Result<WorkspaceRegistry, OrbitError> {
    load_registry_from(&registry_path()?)
}

/// Loads the workspace registry from a specific path (for testing).
pub fn load_registry_from(path: &Path) -> Result<WorkspaceRegistry, OrbitError> {
    load_registry_from_with_writer(path, |registry, destination| {
        write_registry(registry, destination)
    })
}

/// Load and migrate a registry using the supplied persistence operation.
/// [`load_registry_from`] delegates here with the atomic production writer.
pub(crate) fn load_registry_from_with_writer(
    path: &Path,
    writer: impl FnOnce(&WorkspaceRegistry, &Path) -> Result<(), OrbitError>,
) -> Result<WorkspaceRegistry, OrbitError> {
    if !path.exists() {
        return Ok(WorkspaceRegistry::default());
    }
    let content = std::fs::read_to_string(path).map_err(|e| OrbitError::Io(e.to_string()))?;
    let context = registry_host_context(path)?;
    let (registry, migrated) = parse_registry(&content, &context)?;
    if migrated {
        writer(&registry, path)?;
    }
    Ok(registry)
}

/// Saves the workspace registry atomically.
pub fn save_registry(registry: &WorkspaceRegistry) -> Result<(), OrbitError> {
    save_registry_to(registry, &registry_path()?)
}

/// Saves the workspace registry to a specific path (for testing).
pub fn save_registry_to(registry: &WorkspaceRegistry, path: &Path) -> Result<(), OrbitError> {
    let context = registry_host_context(path)?;
    let mut canonical = registry.clone();
    validate_registry(&mut canonical, &context)?;
    write_registry(&canonical, path)
}

/// Registers a new workspace. Errors if a workspace with the same id or name already exists.
pub fn register_workspace(
    registry: &mut WorkspaceRegistry,
    ws: Workspace,
) -> Result<(), OrbitError> {
    if registry.workspaces.iter().any(|w| w.id == ws.id) {
        return Err(OrbitError::WorkspaceError(format!(
            "workspace with id '{}' already exists",
            ws.id
        )));
    }
    if registry.workspaces.iter().any(|w| w.name == ws.name) {
        return Err(OrbitError::WorkspaceError(format!(
            "workspace with name '{}' already exists",
            ws.name
        )));
    }
    registry.workspaces.push(ws);
    Ok(())
}

/// Registers a machine-local checkout for an existing logical workspace.
pub fn register_checkout(
    registry: &mut WorkspaceRegistry,
    checkout: WorkspaceCheckout,
) -> Result<(), OrbitError> {
    if !registry
        .workspaces
        .iter()
        .any(|workspace| workspace.id == checkout.workspace_id)
    {
        return Err(OrbitError::not_found(
            NotFoundKind::Workspace,
            checkout.workspace_id,
        ));
    }
    if registry
        .checkouts
        .iter()
        .any(|existing| existing.workspace_id == checkout.workspace_id)
    {
        return Err(OrbitError::WorkspaceError(format!(
            "local checkout for workspace '{}' already exists",
            checkout.workspace_id
        )));
    }
    if let Some(existing) = registry.checkouts.iter().find(|existing| {
        existing.repo_root == checkout.repo_root || existing.orbit_dir == checkout.orbit_dir
    }) {
        return Err(OrbitError::WorkspaceError(format!(
            "checkout path '{}' is already registered to workspace '{}'",
            checkout.repo_root.display(),
            existing.workspace_id
        )));
    }
    registry.checkouts.push(checkout);
    Ok(())
}

/// Record a machine-local checkout role for an existing logical workspace.
///
/// `Owner` requires no replica owner and leaves the logical owner to be
/// declared explicitly from the validated local `machine_id`. `Replica`
/// requires an explicit non-local `owner_machine_id`, which is mirrored onto
/// both the checkout binding and the logical workspace record so the stable
/// owner identity stays consistent. The optional local identity exists only
/// for pre-host-identity standalone compatibility. This mutates the in-memory
/// registry only; the caller persists via [`save_registry_to`], which validates a clone and
/// therefore leaves the previous file byte-valid on any contradiction. Owner
/// and replica declarations are never inferred from paths, workspace names,
/// presence, hostnames, or Git remotes.
pub fn assign_checkout_role(
    registry: &mut WorkspaceRegistry,
    id_or_name: &str,
    role: WorkspaceCheckoutRole,
    owner_machine_id: Option<&str>,
    local_machine_id: Option<&str>,
) -> Result<(), OrbitError> {
    let workspace = find_workspace(registry, id_or_name)
        .ok_or_else(|| OrbitError::not_found(NotFoundKind::Workspace, id_or_name.to_string()))?;
    let workspace_id = workspace.id.clone();
    let declared_owner = workspace.owner_machine_id.clone();
    let checkout_index = registry
        .checkouts
        .iter()
        .position(|checkout| checkout.workspace_id == workspace_id)
        .ok_or_else(|| {
            OrbitError::WorkspaceError(format!(
                "workspace '{workspace_id}' has no local checkout binding"
            ))
        })?;

    match role {
        WorkspaceCheckoutRole::Owner => {
            if owner_machine_id.is_some() {
                return Err(OrbitError::WorkspaceError(format!(
                    "workspace '{workspace_id}' owner role does not take an owner machine_id"
                )));
            }
            if let Some(local_machine_id) = local_machine_id {
                validate_machine_id(local_machine_id)?;
                if let Some(existing_owner) = declared_owner.as_deref()
                    && existing_owner != local_machine_id
                {
                    return Err(OrbitError::WorkspaceError(format!(
                        "workspace '{workspace_id}' is declared owned by machine \
                         '{existing_owner}'; local machine '{local_machine_id}' cannot assign \
                         itself the owner role"
                    )));
                }
                if declared_owner.is_none()
                    && let Some(workspace) = registry
                        .workspaces
                        .iter_mut()
                        .find(|workspace| workspace.id == workspace_id)
                {
                    workspace.owner_machine_id = Some(local_machine_id.to_string());
                }
            }
        }
        WorkspaceCheckoutRole::Replica => {
            let owner = owner_machine_id.ok_or_else(|| {
                OrbitError::WorkspaceError(format!(
                    "workspace '{workspace_id}' replica role requires an owner machine_id"
                ))
            })?;
            validate_machine_id(owner)?;
            if local_machine_id == Some(owner) {
                return Err(OrbitError::WorkspaceError(format!(
                    "workspace '{workspace_id}' cannot declare the local machine '{owner}' as \
                     its replica owner"
                )));
            }
            if let Some(existing_owner) = declared_owner.as_deref()
                && existing_owner != owner
            {
                return Err(OrbitError::WorkspaceError(format!(
                    "workspace '{workspace_id}' is already owned by machine '{existing_owner}'; \
                     refusing to rebind it to '{owner}' while assigning a replica role"
                )));
            }
            if let Some(existing_owner) = registry.checkouts[checkout_index]
                .owner_machine_id
                .as_deref()
                && existing_owner != owner
            {
                return Err(OrbitError::WorkspaceError(format!(
                    "workspace '{workspace_id}' checkout already mirrors owner machine \
                     '{existing_owner}'; refusing contradictory owner '{owner}'"
                )));
            }
            if let Some(workspace) = registry
                .workspaces
                .iter_mut()
                .find(|workspace| workspace.id == workspace_id)
            {
                workspace.owner_machine_id = Some(owner.to_string());
            }
        }
    }

    let checkout = &mut registry.checkouts[checkout_index];
    checkout.role = Some(role);
    checkout.owner_machine_id = match role {
        WorkspaceCheckoutRole::Owner => None,
        WorkspaceCheckoutRole::Replica => owner_machine_id.map(str::to_string),
    };
    Ok(())
}

/// Removes a workspace by id or name. Returns the removed workspace.
pub fn remove_workspace(
    registry: &mut WorkspaceRegistry,
    id_or_name: &str,
) -> Result<Workspace, OrbitError> {
    let idx = registry
        .workspaces
        .iter()
        .position(|w| w.id == id_or_name || w.name == id_or_name)
        .ok_or_else(|| OrbitError::not_found(NotFoundKind::Workspace, id_or_name.to_string()))?;
    let removed = registry.workspaces.remove(idx);
    registry
        .checkouts
        .retain(|checkout| checkout.workspace_id != removed.id);
    Ok(removed)
}

/// Finds a workspace by id or name.
pub fn find_workspace<'a>(
    registry: &'a WorkspaceRegistry,
    id_or_name: &str,
) -> Option<&'a Workspace> {
    registry
        .workspaces
        .iter()
        .find(|w| w.id == id_or_name || w.name == id_or_name)
}

/// Finds the local checkout for a workspace ID or name.
pub fn find_checkout<'a>(
    registry: &'a WorkspaceRegistry,
    id_or_name: &str,
) -> Option<&'a WorkspaceCheckout> {
    let workspace = find_workspace(registry, id_or_name)?;
    registry
        .checkouts
        .iter()
        .find(|checkout| checkout.workspace_id == workspace.id)
}

/// Iterates logical workspaces that have a machine-local checkout binding.
pub fn local_workspaces(
    registry: &WorkspaceRegistry,
) -> impl Iterator<Item = (&Workspace, &WorkspaceCheckout)> {
    registry.checkouts.iter().filter_map(|checkout| {
        registry
            .workspaces
            .iter()
            .find(|workspace| workspace.id == checkout.workspace_id)
            .map(|workspace| (workspace, checkout))
    })
}

/// Finds the local checkout for a path using longest-prefix matching across
/// its repository root and explicit path overrides.
pub fn find_checkout_by_path<'a>(
    registry: &'a WorkspaceRegistry,
    cwd: &Path,
) -> Option<&'a WorkspaceCheckout> {
    let mut best_match: Option<(&WorkspaceCheckout, usize)> = None;

    for checkout in &registry.checkouts {
        for candidate in std::iter::once(&checkout.repo_root).chain(&checkout.path_overrides) {
            if !cwd.starts_with(candidate) {
                continue;
            }
            let candidate_len = candidate.as_os_str().len();
            if best_match.is_none_or(|(_, current_len)| candidate_len > current_len) {
                best_match = Some((checkout, candidate_len));
            }
        }
    }

    best_match.map(|(checkout, _)| checkout)
}

/// Finds the logical workspace for a path, but only through a machine-local
/// checkout binding. Checkoutless catalog entries can never match a path.
pub fn find_workspace_by_path<'a>(
    registry: &'a WorkspaceRegistry,
    cwd: &Path,
) -> Option<&'a Workspace> {
    let checkout = find_checkout_by_path(registry, cwd)?;
    registry
        .workspaces
        .iter()
        .find(|workspace| workspace.id == checkout.workspace_id)
}

/// Sets a path override binding a directory to a workspace.
pub fn set_path_override(
    registry: &mut WorkspaceRegistry,
    path: PathBuf,
    workspace_id: &str,
) -> Result<(), OrbitError> {
    let checkout = registry
        .checkouts
        .iter_mut()
        .find(|checkout| checkout.workspace_id == workspace_id)
        .ok_or_else(|| {
            OrbitError::WorkspaceError(format!(
                "workspace '{workspace_id}' has no local checkout binding"
            ))
        })?;
    if !checkout.path_overrides.contains(&path) {
        checkout.path_overrides.push(path);
        checkout.path_overrides.sort();
    }
    Ok(())
}

/// Validates local checkout paths, marking their logical workspace invalid
/// when the local repository root no longer exists. Checkoutless logical
/// workspaces retain their catalog status.
pub fn validate_workspaces(registry: &mut WorkspaceRegistry) {
    let now = Utc::now();
    for ws in &mut registry.workspaces {
        let Some(checkout) = registry
            .checkouts
            .iter()
            .find(|checkout| checkout.workspace_id == ws.id)
        else {
            continue;
        };
        if checkout.repo_root.exists() {
            if ws.status == WorkspaceStatus::Invalid {
                ws.status = WorkspaceStatus::Active;
                ws.updated_at = now;
            }
        } else if ws.status == WorkspaceStatus::Active {
            ws.status = WorkspaceStatus::Invalid;
            ws.updated_at = now;
        }
    }
}

#[derive(Debug, Clone)]
struct RegistryHostContext {
    machine_id: Option<String>,
}

fn registry_host_context(path: &Path) -> Result<RegistryHostContext, OrbitError> {
    let global_root = path.parent().ok_or_else(|| {
        OrbitError::WorkspaceError(format!(
            "registry path '{}' has no parent directory",
            path.display()
        ))
    })?;
    match inspect_host_identity(global_root)? {
        HostIdentityState::Present(identity) => Ok(RegistryHostContext {
            machine_id: Some(identity.machine_id),
        }),
        // Pre-host-identity installations are the legacy standalone case.
        HostIdentityState::Legacy { .. } | HostIdentityState::Absent => {
            Ok(RegistryHostContext { machine_id: None })
        }
    }
}

fn parse_registry(
    content: &str,
    context: &RegistryHostContext,
) -> Result<(WorkspaceRegistry, bool), OrbitError> {
    let value: Value = serde_json::from_str(content)
        .map_err(|error| invalid_registry(format!("malformed JSON: {error}")))?;
    let Some(version_value) = value.get("schema_version") else {
        let mut migrated = migrate_legacy_registry(value, context)?;
        validate_registry(&mut migrated, context)?;
        return Ok((migrated, true));
    };
    let version = version_value.as_u64().ok_or_else(|| {
        invalid_registry("schema_version must be a non-negative integer".to_string())
    })?;
    if version > u64::from(WORKSPACE_REGISTRY_SCHEMA_VERSION) {
        return Err(invalid_registry(format!(
            "unsupported schema_version {version}; this build supports up to {WORKSPACE_REGISTRY_SCHEMA_VERSION}. Upgrade Orbit; the file is left unchanged"
        )));
    }
    if version != u64::from(WORKSPACE_REGISTRY_SCHEMA_VERSION) {
        return Err(invalid_registry(format!(
            "invalid schema_version {version}; expected {WORKSPACE_REGISTRY_SCHEMA_VERSION}"
        )));
    }

    validate_role_tokens(&value)?;
    let mut registry: WorkspaceRegistry =
        serde_json::from_value(value).map_err(|error| invalid_registry(error.to_string()))?;
    let changed = validate_registry(&mut registry, context)?;
    Ok((registry, changed))
}

fn validate_role_tokens(value: &Value) -> Result<(), OrbitError> {
    let Some(checkouts) = value.get("checkouts").and_then(Value::as_array) else {
        return Ok(());
    };
    for checkout in checkouts {
        let workspace_id = checkout
            .get("workspace_id")
            .and_then(Value::as_str)
            .unwrap_or("<unknown>");
        let Some(role) = checkout.get("role") else {
            continue;
        };
        match role.as_str() {
            Some("owner" | "replica") => {}
            Some(other) => {
                return Err(invalid_registry(format!(
                    "workspace '{workspace_id}' has unknown checkout role '{other}'"
                )));
            }
            None => {
                return Err(invalid_registry(format!(
                    "workspace '{workspace_id}' has a non-string checkout role"
                )));
            }
        }
    }
    Ok(())
}

fn validate_registry(
    registry: &mut WorkspaceRegistry,
    context: &RegistryHostContext,
) -> Result<bool, OrbitError> {
    if registry.schema_version != WORKSPACE_REGISTRY_SCHEMA_VERSION {
        return Err(invalid_registry(format!(
            "schema_version {} is not supported by this build",
            registry.schema_version
        )));
    }

    let mut changed = false;
    let mut workspace_ids = HashSet::new();
    let mut workspace_names = HashSet::new();
    for workspace in &registry.workspaces {
        if !workspace_ids.insert(workspace.id.clone()) {
            return Err(invalid_registry(format!(
                "duplicate workspace id '{}'",
                workspace.id
            )));
        }
        if !workspace_names.insert(workspace.name.clone()) {
            return Err(invalid_registry(format!(
                "duplicate workspace name '{}'",
                workspace.name
            )));
        }
        if let Some(owner_machine_id) = workspace.owner_machine_id.as_deref() {
            validate_machine_id(owner_machine_id).map_err(|error| {
                invalid_registry(format!(
                    "workspace '{}' has invalid owner_machine_id: {error}",
                    workspace.id
                ))
            })?;
        }
    }

    let mut checkout_ids = HashSet::new();
    for checkout in &mut registry.checkouts {
        let workspace = registry
            .workspaces
            .iter_mut()
            .find(|workspace| workspace.id == checkout.workspace_id)
            .ok_or_else(|| {
                invalid_registry(format!(
                    "checkout references unknown workspace '{}'",
                    checkout.workspace_id
                ))
            })?;
        if !checkout_ids.insert(checkout.workspace_id.clone()) {
            return Err(invalid_registry(format!(
                "workspace '{}' has more than one local checkout binding",
                checkout.workspace_id
            )));
        }

        if checkout.role.is_none() {
            if context.machine_id.is_none() {
                checkout.role = Some(WorkspaceCheckoutRole::Owner);
                changed = true;
            } else {
                return Err(invalid_registry(format!(
                    "workspace '{}' is missing a local checkout role; run `orbit workspace role` to declare owner or replica",
                    checkout.workspace_id
                )));
            }
        }

        match checkout.role {
            Some(WorkspaceCheckoutRole::Owner) => {
                if checkout.owner_machine_id.is_some() {
                    return Err(invalid_registry(format!(
                        "workspace '{}' has contradictory owner role and replica owner_machine_id",
                        checkout.workspace_id
                    )));
                }
                if let Some(machine_id) = context.machine_id.as_deref() {
                    match workspace.owner_machine_id.as_deref() {
                        Some(owner) if owner != machine_id => {
                            return Err(invalid_registry(format!(
                                "workspace '{}' declares local owner role, but logical owner is machine '{owner}' instead of local machine '{machine_id}'",
                                checkout.workspace_id
                            )));
                        }
                        None if context.machine_id.is_none() => {
                            workspace.owner_machine_id = Some(machine_id.to_string());
                            changed = true;
                        }
                        None => {
                            return Err(invalid_registry(format!(
                                "workspace '{}' declares local owner role but has no declared owner_machine_id",
                                checkout.workspace_id
                            )));
                        }
                        Some(_) => {}
                    }
                }
            }
            Some(WorkspaceCheckoutRole::Replica) => {
                if context.machine_id.is_none() {
                    return Err(invalid_registry(format!(
                        "workspace '{}' cannot validate replica role without a local machine_id",
                        checkout.workspace_id
                    )));
                }
                let binding_owner = checkout.owner_machine_id.as_deref().ok_or_else(|| {
                    invalid_registry(format!(
                        "workspace '{}' has replica role without owner_machine_id",
                        checkout.workspace_id
                    ))
                })?;
                validate_machine_id(binding_owner).map_err(|error| {
                    invalid_registry(format!(
                        "workspace '{}' has invalid replica owner_machine_id: {error}",
                        checkout.workspace_id
                    ))
                })?;
                let logical_owner = workspace.owner_machine_id.as_deref().ok_or_else(|| {
                    invalid_registry(format!(
                        "workspace '{}' has replica role but its logical record has no owner_machine_id",
                        checkout.workspace_id
                    ))
                })?;
                if binding_owner != logical_owner {
                    return Err(invalid_registry(format!(
                        "workspace '{}' replica owner '{binding_owner}' contradicts logical owner '{logical_owner}'",
                        checkout.workspace_id
                    )));
                }
                if context.machine_id.as_deref() == Some(binding_owner) {
                    return Err(invalid_registry(format!(
                        "workspace '{}' declares replica role of the local machine '{binding_owner}'",
                        checkout.workspace_id
                    )));
                }
            }
            None => unreachable!("missing checkout role handled above"),
        }

        let before = checkout.path_overrides.len();
        checkout.path_overrides.sort();
        checkout.path_overrides.dedup();
        changed |= checkout.path_overrides.len() != before;
    }
    if context.machine_id.is_some() {
        for workspace in &registry.workspaces {
            if workspace.owner_machine_id.is_none() {
                return Err(invalid_registry(format!(
                    "workspace '{}' is missing owner_machine_id",
                    workspace.id
                )));
            }
        }
    }
    Ok(changed)
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct LegacyWorkspaceRegistry {
    #[serde(default)]
    workspaces: Vec<LegacyWorkspace>,
    #[serde(default)]
    path_overrides: BTreeMap<PathBuf, String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct LegacyWorkspace {
    id: String,
    name: String,
    root: PathBuf,
    orbit_dir: PathBuf,
    #[serde(default)]
    git_remote: Option<String>,
    #[serde(default)]
    ship_mode: Option<String>,
    #[serde(default = "legacy_default_base_branch")]
    base_branch: String,
    #[serde(default = "legacy_default_status")]
    status: WorkspaceStatus,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

fn migrate_legacy_registry(
    value: Value,
    context: &RegistryHostContext,
) -> Result<WorkspaceRegistry, OrbitError> {
    let legacy: LegacyWorkspaceRegistry = serde_json::from_value(value)
        .map_err(|error| invalid_registry(format!("invalid legacy registry: {error}")))?;
    let valid_ids: HashSet<String> = legacy
        .workspaces
        .iter()
        .map(|workspace| workspace.id.clone())
        .collect();
    let mut overrides_by_workspace: BTreeMap<String, Vec<PathBuf>> = BTreeMap::new();
    for (path, workspace_id) in legacy.path_overrides {
        if valid_ids.contains(&workspace_id) {
            overrides_by_workspace
                .entry(workspace_id)
                .or_default()
                .push(path);
        }
    }

    let mut registry = WorkspaceRegistry::default();
    for legacy_workspace in legacy.workspaces {
        let workspace_id = legacy_workspace.id.clone();
        registry.workspaces.push(Workspace {
            id: legacy_workspace.id,
            name: legacy_workspace.name,
            owner_machine_id: context.machine_id.clone(),
            git_remote: legacy_workspace.git_remote,
            ship_mode: legacy_workspace.ship_mode,
            base_branch: legacy_workspace.base_branch,
            status: legacy_workspace.status,
            created_at: legacy_workspace.created_at,
            updated_at: legacy_workspace.updated_at,
        });
        registry.checkouts.push(WorkspaceCheckout {
            workspace_id: workspace_id.clone(),
            repo_root: legacy_workspace.root,
            orbit_dir: legacy_workspace.orbit_dir,
            // Only installations without a host identity may use the legacy
            // owner default. Identity-bearing machines must declare a role.
            role: context
                .machine_id
                .is_none()
                .then_some(WorkspaceCheckoutRole::Owner),
            owner_machine_id: None,
            path_overrides: overrides_by_workspace
                .remove(&workspace_id)
                .unwrap_or_default(),
        });
    }
    Ok(registry)
}

fn write_registry(registry: &WorkspaceRegistry, path: &Path) -> Result<(), OrbitError> {
    let content = serde_json::to_string_pretty(registry)
        .map_err(|e| OrbitError::WorkspaceError(format!("failed to serialize registry: {e}")))?;
    atomic_write_text(path, &content).map_err(Into::into)
}

fn invalid_registry(message: String) -> OrbitError {
    OrbitError::WorkspaceError(format!("invalid registry: {message}"))
}

fn legacy_default_base_branch() -> String {
    "main".to_string()
}

fn legacy_default_status() -> WorkspaceStatus {
    WorkspaceStatus::Active
}
