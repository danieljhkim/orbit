//! Owner execution-profile publication over Core's neutral environment facts.

use chrono::{DateTime, Utc};
use orbit_common::types::{
    Crew, CrewAssignment, EXECUTION_PROFILE_SCHEMA_VERSION, ExecutionProfileCrewV1,
    ExecutionProfileShipV1, ExecutionProfileV1, OrbitError, Workspace,
};
use orbit_core::{ExecutionEnvironmentSnapshot, resolved_ship_mode};

use crate::runtime::ResolvedWorkspaceBinding;

/// Build the frozen owner payload from the exact Core execution environment
/// and Remote workspace authority. No source path or raw asset is retained.
pub fn build_execution_profile_v1(
    environment: ExecutionEnvironmentSnapshot,
    workspace: &Workspace,
    binding: &ResolvedWorkspaceBinding,
    owner_machine_id: &str,
    observed_at: DateTime<Utc>,
) -> Result<ExecutionProfileV1, OrbitError> {
    if binding.logical_workspace_id != workspace.id {
        return Err(OrbitError::InvalidInput(format!(
            "resolved logical workspace_id '{}' does not match registry workspace_id '{}'",
            binding.logical_workspace_id, workspace.id
        )));
    }
    if environment.workspace_id != binding.runtime.workspace_id {
        return Err(OrbitError::InvalidInput(format!(
            "runtime workspace_id '{}' does not match resolved runtime workspace_id '{}' for logical workspace_id '{}'",
            environment.workspace_id, binding.runtime.workspace_id, workspace.id
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
    let runtime_base = environment.workflow_base_branch.trim().to_string();
    if registry_base.is_empty() || runtime_base.is_empty() || registry_base != runtime_base {
        return Err(OrbitError::InvalidInput(format!(
            "workspace_id '{}' registry base_branch '{}' does not match runtime workflow base_branch '{}'",
            workspace.id, workspace.base_branch, runtime_base
        )));
    }

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
                    assignment: CrewAssignment {
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

    let registry_ship_mode = resolved_ship_mode(workspace);
    if binding.runtime.ship_mode != registry_ship_mode {
        return Err(OrbitError::InvalidInput(format!(
            "workspace_id '{}' resolved ship_mode '{}' does not match registry ship_mode '{}'",
            workspace.id,
            binding.runtime.ship_mode.as_input_value(),
            registry_ship_mode.as_input_value()
        )));
    }
    let mode = registry_ship_mode.as_input_value().to_string();
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
            ship_closure_digest: environment.ship_closure_digest,
        },
    };
    profile.config_digest = profile.compute_config_digest()?;
    profile.validate()?;
    Ok(profile)
}
