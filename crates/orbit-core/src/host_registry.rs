//! Execution-profile construction retained in `orbit-core`.
//!
//! The host/workspace registry domain lives in `orbit-registry`; this module
//! keeps the runtime/catalog/ship-closure logic and temporarily re-exports the
//! registry service for existing `orbit-core` consumers.

pub use orbit_registry::host_registry::*;

use std::collections::{BTreeMap, BTreeSet};

use chrono::{DateTime, Utc};
use orbit_common::types::activity_job::{
    Backend, JobV2Step, JobV2StepBody, resolve_activity_backends, resolve_job_backends,
    validate_job_loop_session_backends,
};
use orbit_common::types::{
    ActivityV2, Crew, CrewRoleAssignment, EXECUTION_PROFILE_SCHEMA_VERSION, ExecutionProfileCrewV1,
    ExecutionProfileShipV1, ExecutionProfileV1, JobV2, OrbitError, RegistrySnapshotV1, Workspace,
};
use orbit_engine::{dispatch_error_to_orbit, resolve_job_catalog_refs_for_execution, validate_job};
use orbit_store::Store;
use serde::Serialize;
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::{OrbitRuntime, resolved_ship_mode};

const SHIP_CLOSURE_DIGEST_DOMAIN: &[u8] = b"orbit.ship-closure.v1\0";
const SHIP_CONTRACT_REVISION: u32 = 1;
const SHIP_JOB_NAMES: [&str; 4] = [
    "task_auto_pipeline",
    "task_gate_pipeline",
    "task_local_pipeline",
    "task_pr_pipeline",
];
const UNSUPPORTED_PROFILE_ENV_OVERRIDES: [&str; 5] = [
    "ORBIT_JOB_DIR",
    "ORBIT_V2_JOB_DIR",
    "ORBIT_ACTIVITY_DIR",
    "ORBIT_V2_CATALOG_DIR",
    "ORBIT_BACKEND",
];

/// Read the path-free coordination registry without constructing a workspace
/// runtime. Long-running brokers use this for global discovery tools and must
/// not manufacture a checkout merely to open the hub registry.
pub fn registry_snapshot_at(
    global_root: &std::path::Path,
) -> Result<RegistrySnapshotV1, OrbitError> {
    let database = crate::config::resolved_audit_db_path(global_root, global_root)?;
    HostRegistryService::new(Store::open(&database)?).snapshot()
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
        let runtime_base = self.workflow_base_branch().trim();
        if registry_base.is_empty() || runtime_base.is_empty() || registry_base != runtime_base {
            return Err(OrbitError::InvalidInput(format!(
                "workspace_id '{}' registry base_branch '{}' does not match runtime workflow base_branch '{}'",
                workspace.id,
                workspace.base_branch,
                self.workflow_base_branch()
            )));
        }

        let registry = self.configured_crew_registry_projection();
        let default_crew = registry.default_crew.ok_or_else(|| {
            OrbitError::InvalidInput(
                "execution profile publication requires a configured default_crew".to_string(),
            )
        })?;
        let resolved_backend = self.resolve_v2_backend(None).backend;
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
        let ship_closure_digest = self.build_ship_closure_digest(resolved_backend)?;
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
                base_branch: runtime_base.to_string(),
                ship_closure_digest,
            },
        };
        profile.config_digest = profile.compute_config_digest()?;
        profile.validate()?;
        Ok(profile)
    }

    fn build_ship_closure_digest(&self, resolved_backend: Backend) -> Result<String, OrbitError> {
        let catalog = self.v2_activity_catalog().map_err(|error| {
            OrbitError::InvalidInput(format!(
                "build execution activity catalog for profile: {error}"
            ))
        })?;
        let mut reachable_activities = BTreeSet::new();
        let mut jobs = Vec::with_capacity(SHIP_JOB_NAMES.len());

        for name in SHIP_JOB_NAMES {
            let (_, mut job) = self.load_v2_job_asset_by_name(name)?;
            resolve_job_catalog_refs_for_execution(&mut job, &catalog)
                .map_err(dispatch_error_to_orbit)?;
            resolve_job_backends(&mut job, resolved_backend);
            validate_job_loop_session_backends(&job, name)
                .map_err(|error| OrbitError::InvalidInput(error.to_string()))?;
            validate_job(&job).map_err(dispatch_error_to_orbit)?;

            let mut activity_bindings = BTreeMap::new();
            collect_job_activity_bindings(&job, &mut activity_bindings, &mut reachable_activities);
            let materialized_job = serde_json::to_value(&job).map_err(|error| {
                OrbitError::Store(format!(
                    "serialize materialized ship job '{name}' for digest: {error}"
                ))
            })?;
            jobs.push(ShipJobSemanticV1 {
                name: name.to_string(),
                materialized_job,
                activity_bindings,
            });
        }

        let mut activities = BTreeMap::new();
        for name in reachable_activities {
            let mut activity = catalog.get(&name).cloned().ok_or_else(|| {
                OrbitError::InvalidInput(format!(
                    "reachable ship activity '{name}' is absent from execution catalog"
                ))
            })?;
            resolve_activity_backends(&mut activity, resolved_backend);
            activities.insert(name, activity);
        }

        let dto = ShipClosureSemanticV1 {
            contract: ShipContractSemanticV1 {
                revision: SHIP_CONTRACT_REVISION,
                alias: "ship",
                root_job: "task_auto_pipeline",
                explicit_task_ids: "non-empty task_ids select exactly those tasks; empty selects backlog",
                supported_modes: ["local", "pr"],
                base_semantics: "submission uses the effective runtime workflow base branch",
                review_semantics: "review is PR-only and requires explicit task_ids plus review_crew",
            },
            jobs,
            activities,
        };
        let canonical = serde_json::to_vec(&dto).map_err(|error| {
            OrbitError::Store(format!("serialize ship closure digest DTO: {error}"))
        })?;
        let mut hasher = Sha256::new();
        hasher.update(SHIP_CLOSURE_DIGEST_DOMAIN);
        hasher.update(canonical);
        Ok(format!("{:x}", hasher.finalize()))
    }
}

#[derive(Serialize)]
struct ShipClosureSemanticV1 {
    contract: ShipContractSemanticV1,
    jobs: Vec<ShipJobSemanticV1>,
    activities: BTreeMap<String, ActivityV2>,
}

#[derive(Serialize)]
struct ShipContractSemanticV1 {
    revision: u32,
    alias: &'static str,
    root_job: &'static str,
    explicit_task_ids: &'static str,
    supported_modes: [&'static str; 2],
    base_semantics: &'static str,
    review_semantics: &'static str,
}

#[derive(Serialize)]
struct ShipJobSemanticV1 {
    name: String,
    materialized_job: Value,
    activity_bindings: BTreeMap<String, String>,
}

fn collect_job_activity_bindings(
    job: &JobV2,
    bindings: &mut BTreeMap<String, String>,
    reachable: &mut BTreeSet<String>,
) {
    if let Some(name) = &job.recovery_activity {
        bindings.insert("recovery".to_string(), name.clone());
        reachable.insert(name.clone());
    }
    for (index, step) in job.steps.iter().enumerate() {
        collect_step_activity_bindings(step, &index.to_string(), bindings, reachable);
    }
}

fn collect_step_activity_bindings(
    step: &JobV2Step,
    path: &str,
    bindings: &mut BTreeMap<String, String>,
    reachable: &mut BTreeSet<String>,
) {
    if let Some(name) = &step.recovery_activity {
        bindings.insert(format!("{path}.recovery"), name.clone());
        reachable.insert(name.clone());
    }
    match &step.body {
        JobV2StepBody::Target(target) => {
            if let Some(name) = &target.activity_name {
                bindings.insert(format!("{path}.activity"), name.clone());
                reachable.insert(name.clone());
            }
        }
        JobV2StepBody::TargetRef(reference) => {
            bindings.insert(format!("{path}.activity"), reference.target.clone());
            reachable.insert(reference.target.clone());
        }
        JobV2StepBody::Parallel { parallel } => {
            for (index, branch) in parallel.branches.iter().enumerate() {
                collect_step_activity_bindings(
                    branch,
                    &format!("{path}.parallel.{index}"),
                    bindings,
                    reachable,
                );
            }
        }
        JobV2StepBody::FanOut { fan_out, .. } => collect_step_activity_bindings(
            &fan_out.worker,
            &format!("{path}.fan_out"),
            bindings,
            reachable,
        ),
        JobV2StepBody::Loop { loop_ } => {
            for (index, nested) in loop_.steps.iter().enumerate() {
                collect_step_activity_bindings(
                    nested,
                    &format!("{path}.loop.{index}"),
                    bindings,
                    reachable,
                );
            }
        }
    }
}

fn reject_execution_profile_env_overrides() -> Result<(), OrbitError> {
    reject_execution_profile_env_overrides_from(|name| std::env::var(name).ok())
}

fn reject_execution_profile_env_overrides_from(
    read: impl Fn(&str) -> Option<String>,
) -> Result<(), OrbitError> {
    let active = UNSUPPORTED_PROFILE_ENV_OVERRIDES
        .iter()
        .filter(|name| read(name).is_some_and(|value| !value.trim().is_empty()))
        .copied()
        .collect::<Vec<_>>();
    if active.is_empty() {
        return Ok(());
    }
    Err(OrbitError::InvalidInput(format!(
        "execution profile publication does not support execution-affecting environment overrides: {}",
        active.join(", ")
    )))
}

#[cfg(test)]
#[path = "tests/host_registry.rs"]
mod tests;
