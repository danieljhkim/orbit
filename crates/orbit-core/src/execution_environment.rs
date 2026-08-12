//! Registry-neutral execution environment projection.
//!
//! The remote feature owns publication, ownership validation, and wire-profile
//! construction. Core owns only the execution facts that must be derived from
//! its effective runtime configuration and materialized job/activity catalog.

use std::collections::{BTreeMap, BTreeSet};

use orbit_common::types::activity_job::{
    Backend, JobV2Step, JobV2StepBody, resolve_activity_backends, resolve_job_backends,
    validate_job_loop_session_backends,
};
use orbit_common::types::{ActivityV2, Crew, JobV2, OrbitError};
use orbit_engine::{dispatch_error_to_orbit, resolve_job_catalog_refs_for_execution, validate_job};
use serde::Serialize;
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::OrbitRuntime;
use crate::command::backend_resolver::resolve_backend_precedence;
use crate::config::RuntimeConfig;
use crate::runtime::engine::ConfiguredCrewRegistryProjection;

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

/// Stable, path-free execution facts required by a remote profile publisher.
#[derive(Debug, Clone)]
pub struct ExecutionEnvironmentSnapshot {
    pub workspace_id: String,
    pub workflow_base_branch: String,
    pub crews: ConfiguredCrewRegistryProjection,
    pub resolved_backend: Backend,
    pub ship_closure_digest: String,
}

/// The named-crew registry of one machine's layered configuration, read
/// without opening an [`OrbitRuntime`], a store, or a connector.
///
/// [ORB-10729] v1 crew validation runs where the workspace is owned, so the
/// owner reads its own config directly instead of consuming a published
/// execution-profile projection (mcp-bridge §8.1). The checkoutless owner MCP
/// endpoint must not construct a runtime, so the layering and backend
/// precedence a runtime would apply are exposed here as a pure config read:
/// both entry points therefore answer from the same file and the same
/// precedence, and cannot drift.
#[derive(Debug, Clone)]
pub struct LocalCrewEnvironment {
    pub default_crew: Option<String>,
    pub crews: BTreeMap<String, Crew>,
    pub resolved_backend: Backend,
}

/// Read the layered crew registry for a checkout whose `.orbit` directory is
/// `orbit_dir`, exactly as [`RuntimeConfig`] layers it for a runtime.
///
/// Pass `global_root` itself as `orbit_dir` for a workspace with no local
/// checkout: layering then reads only `<global_root>/config.toml`, which is the
/// whole crew configuration such a workspace has.
pub fn local_crew_environment(
    global_root: &std::path::Path,
    orbit_dir: &std::path::Path,
) -> Result<LocalCrewEnvironment, OrbitError> {
    let config = RuntimeConfig::load_layered(global_root, orbit_dir)?;
    let resolved_backend = resolve_backend_precedence(
        None,
        std::env::var("ORBIT_BACKEND").ok().as_deref(),
        config.v2_backend(),
    )
    .backend;
    Ok(LocalCrewEnvironment {
        default_crew: config.default_crew.clone(),
        crews: config.crews.clone(),
        resolved_backend,
    })
}

impl OrbitRuntime {
    /// Snapshot the exact Core configuration and catalog authorities used by
    /// execution without naming a registry, owner, transport, or wire schema.
    pub fn execution_environment_snapshot(
        &self,
    ) -> Result<ExecutionEnvironmentSnapshot, OrbitError> {
        reject_execution_profile_env_overrides()?;
        let resolved_backend = self.resolve_v2_backend(None).backend;
        Ok(ExecutionEnvironmentSnapshot {
            workspace_id: self.workspace_id()?,
            workflow_base_branch: self.workflow_base_branch().to_string(),
            crews: self.configured_crew_registry_projection(),
            resolved_backend,
            ship_closure_digest: self.build_ship_closure_digest(resolved_backend)?,
        })
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

pub(crate) fn reject_execution_profile_env_overrides() -> Result<(), OrbitError> {
    reject_execution_profile_env_overrides_from(|name| std::env::var(name).ok())
}

pub(crate) fn reject_execution_profile_env_overrides_from(
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
mod tests {
    use super::reject_execution_profile_env_overrides_from;

    #[test]
    fn unsupported_execution_environment_overrides_fail_closed_without_values() {
        let error = reject_execution_profile_env_overrides_from(|name| {
            (name == "ORBIT_JOB_DIR").then(|| "/secret/catalog/path".to_string())
        })
        .expect_err("override must fail")
        .to_string();
        assert!(error.contains("ORBIT_JOB_DIR"));
        assert!(!error.contains("/secret/catalog/path"));
    }
}
