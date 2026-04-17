use chrono::{DateTime, Utc};
use orbit_engine::EnvironmentHost;
use orbit_store::JobCreateParams as StoreActivityCreateParams;
use orbit_store::JobUpdateParams as StoreJobUpdateParams;
use orbit_types::{
    Job, JobResource, JobRun, JobScheduleState, JobStep, JobTargetType, OrbitError, OrbitEvent,
    RESOURCE_SCHEMA_VERSION, ResourceKind, default_job_max_active_runs, resolve_agent_model_pair,
};
use serde_json::Value;

use crate::OrbitRuntime;
use crate::command::activity::activity_requires_agent_cli;

const JOB_PARALLEL_TASK_PIPELINE: &str = "job_parallel_task_pipeline";
const JOB_LOCAL_TASK_PIPELINE: &str = "job_local_task_pipeline";
const DEFAULT_JOB_FILES: &[(&str, &str)] = &[
    (
        "job_parallel_task_worker",
        include_str!("../../assets/jobs/job_parallel_task_worker.yaml"),
    ),
    (
        "job_batch_review_loop",
        include_str!("../../assets/jobs/job_batch_review_loop.yaml"),
    ),
    (
        "job_batch_review_cycle",
        include_str!("../../assets/jobs/job_batch_review_cycle.yaml"),
    ),
    (
        "job_duel_review_loop",
        include_str!("../../assets/jobs/job_duel_review_loop.yaml"),
    ),
    (
        "job_duel_review_cycle",
        include_str!("../../assets/jobs/job_duel_review_cycle.yaml"),
    ),
    (
        "job_duel_pipeline",
        include_str!("../../assets/jobs/job_duel_pipeline.yaml"),
    ),
    (
        "job_duel_plan_pipeline",
        include_str!("../../assets/jobs/job_duel_plan_pipeline.yaml"),
    ),
    (
        "job_parallel_task_pipeline",
        include_str!("../../assets/jobs/job_parallel_task_pipeline.yaml"),
    ),
    (
        "job_local_task_pipeline",
        include_str!("../../assets/jobs/job_local_task_pipeline.yaml"),
    ),
];

#[derive(Debug, Clone)]
pub struct JobAddParams {
    pub job_id: Option<String>,
    pub default_input: Option<Value>,
    pub max_active_runs: Option<u32>,
    pub max_iterations: Option<u32>,
    pub steps: Vec<JobStep>,
    pub policy: Option<String>,
    pub initial_state_override: Option<JobScheduleState>,
}

impl OrbitRuntime {
    pub fn run_job_now(&self, job_id: &str) -> Result<orbit_engine::JobRunResult, OrbitError> {
        self.run_job_now_with_input(job_id, serde_json::json!({}))
    }

    pub fn run_job_now_with_input(
        &self,
        job_id: &str,
        input: Value,
    ) -> Result<orbit_engine::JobRunResult, OrbitError> {
        self.run_job_now_with_input_debug(job_id, input, false)
    }

    pub fn run_job_now_with_input_debug(
        &self,
        job_id: &str,
        input: Value,
        debug: bool,
    ) -> Result<orbit_engine::JobRunResult, OrbitError> {
        self.ensure_pipeline_mode_is_exclusive(job_id)?;
        let job = self.show_job(job_id)?;
        orbit_engine::run_job_with_input(self, &self.data_root(), job, input, debug)
    }

    fn ensure_pipeline_mode_is_exclusive(&self, job_id: &str) -> Result<(), OrbitError> {
        match job_id {
            // Task pipelines now rely on per-job `max_active_runs`, `dispatch_batch`
            // conflict exclusion, and merge retry logic instead of a global
            // single-flight gate.
            JOB_PARALLEL_TASK_PIPELINE | JOB_LOCAL_TASK_PIPELINE => Ok(()),
            _ => Ok(()),
        }
    }

    pub(crate) fn recover_stale_active_run_for_job(
        &self,
        job: &Job,
        now: DateTime<Utc>,
    ) -> Result<bool, OrbitError> {
        orbit_engine::recover_stale_active_run_for_job(self, &self.data_root(), job, now)
    }

    pub fn add_job(&self, params: JobAddParams) -> Result<Job, OrbitError> {
        if params.steps.is_empty() {
            return Err(OrbitError::JobValidation(
                "job must have at least one step".to_string(),
            ));
        }
        let max_active_runs = validate_job_max_active_runs(params.max_active_runs)?;
        let default_input = normalize_job_default_input(params.default_input)?;
        if let Some(ref policy_name) = params.policy {
            self.stores().policies().get(policy_name)?.ok_or_else(|| {
                OrbitError::JobValidation(format!("policy '{}' does not exist", policy_name))
            })?;
        }
        self.validate_job_steps(params.job_id.as_deref(), &params.steps, true)?;

        let initial_state = params
            .initial_state_override
            .unwrap_or(JobScheduleState::Enabled);

        let steps = normalize_job_steps(params.steps)?;

        let max_iterations = params.max_iterations.unwrap_or(1);
        let job = self.stores().jobs().add(StoreActivityCreateParams {
            job_id: params.job_id,
            default_input,
            max_active_runs,
            max_iterations,
            steps,
            policy: params.policy,
            initial_state,
        })?;
        self.record_event(OrbitEvent::JobAdded {
            job_id: job.job_id.clone(),
        })?;
        Ok(job)
    }

    fn validate_job_steps(
        &self,
        job_id: Option<&str>,
        steps: &[JobStep],
        resolve_activity_skills: bool,
    ) -> Result<(), OrbitError> {
        for step in steps {
            if step.target_id.trim().is_empty() {
                return Err(OrbitError::JobValidation(
                    "step target_id must not be empty".to_string(),
                ));
            }
            if step.target_type == JobTargetType::Job {
                if let Some(job_id) = job_id
                    && step.target_id == job_id
                {
                    return Err(OrbitError::JobValidation(format!(
                        "job '{}' cannot reference itself as a step",
                        job_id
                    )));
                }
                let referenced_job = self.get_job_backend(&step.target_id)?.ok_or_else(|| {
                    OrbitError::JobValidation(format!(
                        "step references job '{}' which does not exist",
                        step.target_id
                    ))
                })?;
                if let Some(job_id) = job_id {
                    for sub_step in &referenced_job.steps {
                        if sub_step.target_type == JobTargetType::Job
                            && sub_step.target_id == job_id
                        {
                            return Err(OrbitError::JobValidation(format!(
                                "cycle detected: job '{}' references '{}' which references back",
                                job_id, step.target_id
                            )));
                        }
                    }
                }
                continue;
            }
            let activity = if resolve_activity_skills {
                self.validate_activity_target_exists(step.target_type, &step.target_id)?
            } else {
                self.show_activity(&step.target_id)?
            };
            if activity_requires_agent_cli(&activity.spec_type) {
                if !step.agent_cli.trim().is_empty() {
                    self.validate_agent_cli(&step.agent_cli, step.model.as_deref())?;
                } else if let Some(executor_name) = step
                    .executor
                    .as_deref()
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                {
                    let executor_def =
                        self.stores()
                            .executors()
                            .get(executor_name)?
                            .ok_or_else(|| {
                                OrbitError::JobValidation(format!(
                                    "step references executor '{}' which does not exist",
                                    executor_name
                                ))
                            })?;
                    let command = executor_def
                        .command
                        .clone()
                        .unwrap_or_else(|| executor_name.to_string());
                    let model = step
                        .model
                        .clone()
                        .or_else(|| resolve_executor_tier_model(&command, &executor_def, step));
                    self.validate_agent_cli(&command, model.as_deref())?;
                }
            }
        }

        Ok(())
    }

    pub fn update_job_definition(
        &self,
        job_id: &str,
        default_input: Option<Value>,
        max_active_runs: u32,
        max_iterations: u32,
        steps: Vec<JobStep>,
        policy: Option<String>,
        state: JobScheduleState,
    ) -> Result<Job, OrbitError> {
        if let Some(ref policy_name) = policy {
            self.stores().policies().get(policy_name)?.ok_or_else(|| {
                OrbitError::JobValidation(format!("policy '{}' does not exist", policy_name))
            })?;
        }
        let steps = normalize_job_steps(steps)?;
        let job = self.stores().jobs().update(
            job_id,
            StoreJobUpdateParams {
                default_input: Some(normalize_job_default_input(default_input)?),
                max_active_runs: Some(validate_job_max_active_runs(Some(max_active_runs))?),
                max_iterations: Some(max_iterations),
                steps: Some(steps),
                policy: Some(policy),
                state: Some(state),
            },
        )?;
        self.record_event(OrbitEvent::JobUpdated {
            job_id: job.job_id.clone(),
        })?;
        Ok(job)
    }

    pub fn list_jobs(&self, include_disabled: bool) -> Result<Vec<Job>, OrbitError> {
        self.list_jobs_backend(include_disabled)
    }

    pub fn list_jobs_with_last_run(
        &self,
        include_disabled: bool,
    ) -> Result<Vec<(Job, Option<JobRun>)>, OrbitError> {
        use orbit_store::JobRunQuery;

        let now = Utc::now();
        let jobs = self.list_jobs_backend(include_disabled)?;
        let mut result = Vec::with_capacity(jobs.len());
        for job in jobs {
            let _ = self.recover_stale_active_run_for_job(&job, now);
            let last_run = self
                .stores()
                .jobs()
                .list_runs_filtered(&JobRunQuery {
                    job_id: Some(job.job_id.clone()),
                    state: None,
                    created_since: None,
                    limit: Some(1),
                })
                .ok()
                .and_then(|runs| runs.into_iter().next());
            result.push((job, last_run));
        }
        Ok(result)
    }

    pub fn show_job(&self, job_id: &str) -> Result<Job, OrbitError> {
        self.get_job_backend(job_id)?
            .ok_or_else(|| OrbitError::JobNotFound(job_id.to_string()))
    }

    pub fn delete_job(&self, job_id: &str) -> Result<(), OrbitError> {
        let changed = self.stores().jobs().mark_disabled(job_id)?;
        if !changed {
            return Err(OrbitError::JobNotFound(job_id.to_string()));
        }
        self.record_event(OrbitEvent::JobDeleted {
            job_id: job_id.to_string(),
        })
    }

    fn list_jobs_backend(&self, include_disabled: bool) -> Result<Vec<Job>, OrbitError> {
        self.stores().jobs().list(include_disabled)
    }

    fn get_job_backend(&self, job_id: &str) -> Result<Option<Job>, OrbitError> {
        self.stores().jobs().get(job_id)
    }
}

fn resolve_executor_tier_model(
    agent_cli: &str,
    executor_def: &orbit_types::ExecutorDef,
    step: &JobStep,
) -> Option<String> {
    let tier = step
        .model_tier
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())?;
    if let Some(model) = executor_def.model_for_tier(tier) {
        return Some(model.to_string());
    }
    match tier {
        "strong" => resolve_agent_model_pair(agent_cli).map(|pair| pair.orchestrator),
        "weak" => resolve_agent_model_pair(agent_cli).map(|pair| pair.helper),
        _ => None,
    }
}

fn normalize_job_default_input(default_input: Option<Value>) -> Result<Option<Value>, OrbitError> {
    match default_input {
        None => Ok(None),
        Some(Value::Object(map)) => Ok(Some(Value::Object(map))),
        Some(other) => Err(OrbitError::JobValidation(format!(
            "job default_input must be an object, got {}",
            json_value_type_name(&other)
        ))),
    }
}

fn normalize_job_steps(steps: Vec<JobStep>) -> Result<Vec<JobStep>, OrbitError> {
    steps
        .into_iter()
        .map(|step| {
            let env_extra = crate::config::normalize_pass_list(step.env_extra)
                .map_err(|e| OrbitError::JobValidation(e.to_string()))?;
            let default_input = normalize_job_default_input(step.default_input)?;
            Ok(JobStep {
                env_extra,
                default_input,
                ..step
            })
        })
        .collect()
}

fn json_value_type_name(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

fn load_default_job_specs(raw_specs: &[(&str, &str)]) -> Result<Vec<JobResource>, OrbitError> {
    let mut specs = Vec::with_capacity(raw_specs.len());
    for (expected_id, raw) in raw_specs {
        let resource = serde_yaml::from_str::<JobResource>(raw).map_err(|err| {
            OrbitError::InvalidInput(format!("invalid default job spec '{}': {err}", expected_id))
        })?;
        if resource.schema_version != RESOURCE_SCHEMA_VERSION {
            return Err(OrbitError::InvalidInput(format!(
                "default job '{}' uses unsupported schemaVersion {}",
                expected_id, resource.schema_version
            )));
        }
        if resource.kind != ResourceKind::Job {
            return Err(OrbitError::InvalidInput(format!(
                "default job '{}' has unexpected kind {}",
                expected_id, resource.kind
            )));
        }
        let id = resource.metadata.name.trim();
        if id != *expected_id {
            return Err(OrbitError::InvalidInput(format!(
                "default job file key '{}' does not match spec job_id '{}'",
                expected_id, id
            )));
        }
        specs.push(resource);
    }
    Ok(specs)
}

pub(crate) fn seed_default_jobs(
    runtime: &OrbitRuntime,
    overwrite: bool,
) -> Result<usize, OrbitError> {
    let specs = load_default_job_specs(DEFAULT_JOB_FILES)?;
    let mut created = 0usize;
    for resource in specs {
        let job_id = resource.metadata.name.clone();
        let spec = resource.spec;
        if runtime.show_job(&job_id).is_ok() {
            if !overwrite {
                continue;
            }
            runtime.validate_job_steps(Some(&job_id), &spec.steps, false)?;
            runtime.update_job_definition(
                &job_id,
                spec.default_input,
                spec.max_active_runs,
                spec.max_iterations,
                spec.steps,
                spec.policy,
                spec.state,
            )?;
            created += 1;
            continue;
        }
        if let Some(ref policy_name) = spec.policy {
            runtime
                .stores()
                .policies()
                .get(policy_name)?
                .ok_or_else(|| {
                    OrbitError::JobValidation(format!("policy '{}' does not exist", policy_name))
                })?;
        }
        runtime.validate_job_steps(Some(&job_id), &spec.steps, false)?;
        let default_input = normalize_job_default_input(spec.default_input)?;
        let max_active_runs = validate_job_max_active_runs(Some(spec.max_active_runs))?;
        let steps = normalize_job_steps(spec.steps)?;
        runtime.stores().jobs().add(StoreActivityCreateParams {
            job_id: Some(job_id),
            default_input,
            max_active_runs,
            max_iterations: spec.max_iterations,
            steps,
            policy: spec.policy,
            initial_state: spec.state,
        })?;
        created += 1;
    }
    Ok(created)
}

fn validate_job_max_active_runs(max_active_runs: Option<u32>) -> Result<u32, OrbitError> {
    let value = max_active_runs.unwrap_or_else(default_job_max_active_runs);
    if value == 0 {
        return Err(OrbitError::JobValidation(
            "job max_active_runs must be at least 1".to_string(),
        ));
    }
    Ok(value)
}
