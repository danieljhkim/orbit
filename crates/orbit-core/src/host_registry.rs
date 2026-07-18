//! Typed core service for the hub host registry [ORB-10255].
//!
//! This layer binds B1's stable local [`HostIdentity`] declaration to the
//! durable hub-store API. It intentionally does not coordinate local
//! `host.toml` renames, expose administration commands, or add transport;
//! those surfaces belong to the later registry-administration unit.

use std::collections::{BTreeMap, BTreeSet};

use chrono::{DateTime, Duration, Utc};
use orbit_common::types::activity_job::{
    Backend, JobV2Step, JobV2StepBody, resolve_activity_backends, resolve_job_backends,
    validate_job_loop_session_backends,
};
use orbit_common::types::{
    ActivityV2, Crew, CrewRoleAssignment, EXECUTION_PROFILE_SCHEMA_VERSION, ExecutionProfileCrewV1,
    ExecutionProfileShipV1, ExecutionProfileV1, HostAlias, HostNameResolution, HostRecord,
    HostRegistration, HostWorkspacePresence, JobV2, OrbitError, SanitizedExecutionProfile,
    SanitizedWorkspacePresence, StoredExecutionProfile, Workspace, WorkspaceOwnership,
    WorkspacePresenceDeclaration, WorkspaceRegistry, WorkspaceStatus,
};
use orbit_engine::{dispatch_error_to_orbit, resolve_job_catalog_refs_for_execution, validate_job};
use orbit_store::Store;
use serde::Serialize;
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::routines::HostIdentity;
use crate::{OrbitRuntime, resolved_ship_mode};

const PROFILE_FRESHNESS_TTL: Duration = Duration::minutes(10);
const PROFILE_MAX_OBSERVATION_AGE: Duration = Duration::minutes(10);
const PROFILE_MAX_FUTURE_SKEW: Duration = Duration::minutes(2);
const PRESENCE_FRESHNESS_TTL: Duration = Duration::minutes(5);
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

#[derive(Clone)]
pub struct HostRegistryService {
    store: Store,
}

impl HostRegistryService {
    pub fn new(store: Store) -> Self {
        Self { store }
    }

    /// Register B1's stable machine identity with a compatible label set.
    pub fn register_identity(
        &self,
        identity: &HostIdentity,
        labels: BTreeSet<String>,
    ) -> Result<HostRecord, OrbitError> {
        self.store.register_host(&HostRegistration {
            machine_id: identity.machine_id.clone(),
            host_id: identity.host_id.clone(),
            labels,
        })
    }

    pub fn rename(&self, machine_id: &str, new_host_id: &str) -> Result<HostRecord, OrbitError> {
        self.store.rename_host(machine_id, new_host_id)
    }

    pub fn retire(&self, machine_id: &str) -> Result<HostRecord, OrbitError> {
        self.store.retire_host(machine_id)
    }

    pub fn resolve(&self, host_id: &str) -> Result<HostNameResolution, OrbitError> {
        self.store.resolve_host_id(host_id)
    }

    pub fn active_hosts(&self) -> Result<Vec<HostRecord>, OrbitError> {
        self.store.list_active_hosts()
    }

    pub fn aliases(&self, machine_id: &str) -> Result<Vec<HostAlias>, OrbitError> {
        self.store.list_host_aliases(machine_id)
    }

    pub fn bind_workspace_owner(
        &self,
        registry: &WorkspaceRegistry,
        workspace_id: &str,
        owner_machine_id: &str,
    ) -> Result<WorkspaceOwnership, OrbitError> {
        let workspace = require_logical_workspace(registry, workspace_id)?;
        if let Some(mirror) = workspace.owner_machine_id.as_deref()
            && mirror != owner_machine_id
        {
            return Err(OrbitError::InvalidInput(format!(
                "workspace_id '{workspace_id}' local owner mirror '{mirror}' does not match requested hub owner '{owner_machine_id}'"
            )));
        }
        self.store
            .bind_workspace_owner(workspace_id, owner_machine_id)
    }

    pub fn publish_presence(
        &self,
        registry: &WorkspaceRegistry,
        caller_machine_id: &str,
        declarations: &[WorkspacePresenceDeclaration],
    ) -> Result<Vec<HostWorkspacePresence>, OrbitError> {
        self.publish_presence_at(registry, caller_machine_id, declarations, Utc::now())
    }

    fn publish_presence_at(
        &self,
        registry: &WorkspaceRegistry,
        caller_machine_id: &str,
        declarations: &[WorkspacePresenceDeclaration],
        received_at: DateTime<Utc>,
    ) -> Result<Vec<HostWorkspacePresence>, OrbitError> {
        for declaration in declarations {
            require_logical_workspace(registry, &declaration.workspace_id)?;
        }
        self.store
            .replace_host_workspace_presence(caller_machine_id, declarations, received_at)
    }

    pub fn presence_status(
        &self,
        machine_id: &str,
        workspace_id: &str,
    ) -> Result<SanitizedWorkspacePresence, OrbitError> {
        self.store.sanitized_workspace_presence(
            machine_id,
            workspace_id,
            Utc::now(),
            PRESENCE_FRESHNESS_TTL,
        )
    }

    pub fn publish_execution_profile(
        &self,
        caller_machine_id: &str,
        expected_generation: u64,
        profile: &ExecutionProfileV1,
    ) -> Result<StoredExecutionProfile, OrbitError> {
        self.publish_execution_profile_at(
            caller_machine_id,
            expected_generation,
            profile,
            Utc::now(),
        )
    }

    fn publish_execution_profile_at(
        &self,
        caller_machine_id: &str,
        expected_generation: u64,
        profile: &ExecutionProfileV1,
        received_at: DateTime<Utc>,
    ) -> Result<StoredExecutionProfile, OrbitError> {
        self.store.publish_execution_profile(
            caller_machine_id,
            expected_generation,
            profile,
            received_at,
            PROFILE_MAX_OBSERVATION_AGE,
            PROFILE_MAX_FUTURE_SKEW,
        )
    }

    pub fn execution_profile_status(
        &self,
        workspace_id: &str,
    ) -> Result<SanitizedExecutionProfile, OrbitError> {
        self.store
            .sanitized_execution_profile(workspace_id, Utc::now(), PROFILE_FRESHNESS_TTL)
    }
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

fn require_logical_workspace<'a>(
    registry: &'a WorkspaceRegistry,
    workspace_id: &str,
) -> Result<&'a Workspace, OrbitError> {
    let workspace = registry
        .workspaces
        .iter()
        .find(|workspace| workspace.id == workspace_id)
        .ok_or_else(|| {
            OrbitError::InvalidInput(format!("unknown logical workspace_id '{workspace_id}'"))
        })?;
    if workspace.status != WorkspaceStatus::Active {
        return Err(OrbitError::InvalidInput(format!(
            "logical workspace_id '{workspace_id}' is not active"
        )));
    }
    Ok(workspace)
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
