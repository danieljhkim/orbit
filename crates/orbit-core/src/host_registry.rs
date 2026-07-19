//! Execution-profile construction retained in `orbit-core`.
//!
//! The host/workspace registry domain lives in `orbit-registry`; this module
//! keeps the runtime/catalog/ship-closure logic and temporarily re-exports the
//! registry service for existing `orbit-core` consumers.

pub use orbit_registry::host_registry::*;

use chrono::{DateTime, Utc};
use orbit_common::types::{
    Crew, CrewRoleAssignment, EXECUTION_PROFILE_SCHEMA_VERSION, ExecutionProfileCrewV1,
    ExecutionProfileShipV1, ExecutionProfileV1, OrbitError, RegistrySnapshotV1, Workspace,
};
use orbit_store::Store;

use crate::execution_environment::reject_execution_profile_env_overrides;
use crate::{OrbitRuntime, resolved_ship_mode};

/// Read the path-free coordination registry without constructing a workspace
/// runtime. Long-running brokers use this for global discovery tools and must
/// not manufacture a checkout merely to open the hub registry.
pub fn registry_snapshot_at(
    global_root: &std::path::Path,
) -> Result<RegistrySnapshotV1, OrbitError> {
    host_registry_service_at(global_root)?.snapshot()
}

/// Open the one store-backed registry service for a machine-global root.
/// Hub MCP composition uses this without constructing a checkout runtime.
pub fn host_registry_service_at(
    global_root: &std::path::Path,
) -> Result<HostRegistryService, OrbitError> {
    let database = crate::config::resolved_audit_db_path(global_root, global_root)?;
    Ok(HostRegistryService::new(Store::open(&database)?))
}

/// Persist a broker denial into the global coordination audit database when a
/// workspace runtime is deliberately unavailable (for example a global tool
/// or a checkoutless preflight denial).
pub fn record_global_audit_event_at(
    global_root: &std::path::Path,
    params: &orbit_store::AuditEventInsertParams,
) -> Result<(), OrbitError> {
    let database = crate::config::resolved_audit_db_path(global_root, global_root)?;
    Store::open(&database)?.insert_audit_event_record(params)
}

impl OrbitRuntime {
    /// Build the frozen owner payload from the exact runtime/config/catalog
    /// authorities execution uses. The returned value contains only stable
    /// IDs and semantic digests; no source path or raw asset is retained.
    pub fn build_execution_profile_v1(
        &self,
        workspace: &Workspace,
        owner_machine_id: &str,
        observed_at: DateTime<Utc>,
    ) -> Result<ExecutionProfileV1, OrbitError> {
        // Preserve the legacy validation order: unsupported process-local
        // execution overrides and catalog mismatches fail before materializing
        // the comparatively expensive job/activity closure digest.
        reject_execution_profile_env_overrides()?;
        let runtime_workspace_id = self.workspace_id()?;
        if runtime_workspace_id != workspace.id {
            return Err(OrbitError::InvalidInput(format!(
                "runtime workspace_id '{runtime_workspace_id}' does not match logical workspace_id '{}'",
                workspace.id
            )));
        }
        if let Some(mirror) = workspace.owner_machine_id.as_deref()
            && mirror != owner_machine_id
        {
            return Err(OrbitError::InvalidInput(format!(
                "workspace_id '{}' local owner mirror '{mirror}' does not match publishing owner '{owner_machine_id}'",
                workspace.id
            )));
        }
        let registry_base = workspace.base_branch.trim();
        let runtime_base = self.workflow_base_branch().trim().to_string();
        if registry_base.is_empty() || runtime_base.is_empty() || registry_base != runtime_base {
            return Err(OrbitError::InvalidInput(format!(
                "workspace_id '{}' registry base_branch '{}' does not match runtime workflow base_branch '{}'",
                workspace.id, workspace.base_branch, runtime_base
            )));
        }

        let environment = self.execution_environment_snapshot()?;
        let registry = environment.crews;
        let default_crew = registry.default_crew.ok_or_else(|| {
            OrbitError::InvalidInput(
                "execution profile publication requires a configured default_crew".to_string(),
            )
        })?;
        let resolved_backend = environment.resolved_backend;
        let mut crews = registry
            .crews
            .into_iter()
            .map(|crew| {
                ExecutionProfileCrewV1::from_crew(
                    &Crew {
                        name: crew.name,
                        assignment: CrewRoleAssignment {
                            provider: crew.provider,
                            model: crew.model,
                            backend: crew.backend,
                        },
                        description: crew.description,
                        tags: crew.tags,
                    },
                    resolved_backend,
                )
            })
            .collect::<Result<Vec<_>, _>>()?;
        crews.sort_by(|left, right| left.name.cmp(&right.name));

        let mode = resolved_ship_mode(workspace).as_input_value().to_string();
        let ship_closure_digest = environment.ship_closure_digest;
        let mut profile = ExecutionProfileV1 {
            schema_version: EXECUTION_PROFILE_SCHEMA_VERSION,
            workspace_id: workspace.id.clone(),
            owner_machine_id: owner_machine_id.to_string(),
            observed_at,
            config_digest: String::new(),
            default_crew,
            crews,
            ship: ExecutionProfileShipV1 {
                mode,
                base_branch: runtime_base,
                ship_closure_digest,
            },
        };
        profile.config_digest = profile.compute_config_digest()?;
        profile.validate()?;
        Ok(profile)
    }
}

#[cfg(test)]
#[path = "tests/host_registry.rs"]
mod tests;
