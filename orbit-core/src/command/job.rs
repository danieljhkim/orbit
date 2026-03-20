use chrono::{DateTime, Utc};
use orbit_agent::{Agent, AgentConfig};
use orbit_store::JobCreateParams as StoreActivityCreateParams;
use orbit_store::JobUpdateParams as StoreJobUpdateParams;
use orbit_types::{
    Job, JobRun, JobScheduleState, JobStep, JobTargetType, OrbitError, OrbitEvent,
    default_job_max_active_runs,
};
use serde::Deserialize;
use serde_json::Value;

use crate::OrbitRuntime;
use crate::command::activity::activity_requires_agent_cli;

const DEFAULT_JOB_FILES: &[(&str, &str)] = &[
    (
        "job_review_tasks",
        include_str!("../../assets/jobs/job_review_tasks.yaml"),
    ),
    (
        "job_oversee_orbit_operations",
        include_str!("../../assets/jobs/job_oversee_orbit_operations.yaml"),
    ),
    (
        "job_perform_maintenance",
        include_str!("../../assets/jobs/job_perform_maintenance.yaml"),
    ),
    (
        "job_task_pipeline",
        include_str!("../../assets/jobs/job_task_pipeline.yaml"),
    ),
];

#[derive(Debug, Clone, Deserialize)]
struct DefaultJobFileSpec {
    job: DefaultJobEntry,
}

#[derive(Debug, Clone, Deserialize)]
struct DefaultJobEntry {
    job_id: String,
    state: String,
    #[serde(default)]
    default_input: Option<Value>,
    #[serde(default = "default_job_max_active_runs")]
    max_active_runs: u32,
    steps: Vec<DefaultJobStep>,
}

#[derive(Debug, Clone, Deserialize)]
struct DefaultJobStep {
    target_type: String,
    target_id: String,
    #[serde(default)]
    agent_cli: String,
    #[serde(default)]
    model: Option<String>,
    timeout_seconds: u64,
    #[serde(default)]
    env_extra: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct JobAddParams {
    pub job_id: Option<String>,
    pub default_input: Option<Value>,
    pub max_active_runs: Option<u32>,
    pub steps: Vec<JobStep>,
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
        let job = self.show_job(job_id)?;
        orbit_engine::run_job_with_input(self, job, input, debug)
    }

    pub(crate) fn recover_stale_active_run_for_job(
        &self,
        job: &Job,
        now: DateTime<Utc>,
    ) -> Result<bool, OrbitError> {
        orbit_engine::recover_stale_active_run_for_job(self, job, now)
    }

    pub fn add_job(&self, params: JobAddParams) -> Result<Job, OrbitError> {
        if params.steps.is_empty() {
            return Err(OrbitError::JobValidation(
                "job must have at least one step".to_string(),
            ));
        }
        let max_active_runs = validate_job_max_active_runs(params.max_active_runs)?;
        let default_input = normalize_job_default_input(params.default_input)?;

        for step in &params.steps {
            if step.target_id.trim().is_empty() {
                return Err(OrbitError::JobValidation(
                    "step target_id must not be empty".to_string(),
                ));
            }
            let activity =
                self.validate_activity_target_exists(step.target_type, &step.target_id)?;
            if activity_requires_agent_cli(&activity.spec_type) && step.agent_cli.trim().is_empty()
            {
                return Err(OrbitError::JobValidation(
                    "step agent_cli must not be empty for agent_invoke activities".to_string(),
                ));
            }
            if activity_requires_agent_cli(&activity.spec_type) {
                let _ = Agent::new(
                    &AgentConfig::cli(step.agent_cli.clone()).with_model(step.model.as_deref()),
                )?;
            }
        }

        let initial_state = params
            .initial_state_override
            .unwrap_or(JobScheduleState::Enabled);

        let steps = params
            .steps
            .into_iter()
            .map(|s| {
                let env_extra = crate::config::normalize_pass_list(s.env_extra)
                    .map_err(|e| OrbitError::JobValidation(e.to_string()))?;
                Ok(JobStep { env_extra, ..s })
            })
            .collect::<Result<Vec<_>, OrbitError>>()?;

        let job = self.add_job_record(StoreActivityCreateParams {
            job_id: params.job_id,
            default_input,
            max_active_runs,
            steps,
            initial_state,
        })?;
        self.record_event(OrbitEvent::JobAdded {
            job_id: job.job_id.clone(),
        })?;
        Ok(job)
    }

    pub(crate) fn update_job_definition(
        &self,
        job_id: &str,
        default_input: Option<Value>,
        max_active_runs: u32,
        steps: Vec<JobStep>,
        state: JobScheduleState,
    ) -> Result<Job, OrbitError> {
        let job = self.update_job_record(
            job_id,
            StoreJobUpdateParams {
                default_input: Some(normalize_job_default_input(default_input)?),
                max_active_runs: Some(validate_job_max_active_runs(Some(max_active_runs))?),
                steps: Some(steps),
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
                .list_job_runs_filtered_record(&JobRunQuery {
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
        let changed = self.mark_job_disabled_record(job_id)?;
        if !changed {
            return Err(OrbitError::JobNotFound(job_id.to_string()));
        }
        self.record_event(OrbitEvent::JobDeleted {
            job_id: job_id.to_string(),
        })
    }

    fn list_jobs_backend(&self, include_disabled: bool) -> Result<Vec<Job>, OrbitError> {
        self.list_job_records(include_disabled)
    }

    fn get_job_backend(&self, job_id: &str) -> Result<Option<Job>, OrbitError> {
        self.get_job_record(job_id)
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

fn load_default_job_specs(raw_specs: &[(&str, &str)]) -> Result<Vec<DefaultJobEntry>, OrbitError> {
    let mut specs = Vec::with_capacity(raw_specs.len());
    for (expected_id, raw) in raw_specs {
        let file_spec = serde_yaml::from_str::<DefaultJobFileSpec>(raw).map_err(|err| {
            OrbitError::InvalidInput(format!("invalid default job spec '{}': {err}", expected_id))
        })?;
        let entry = file_spec.job;
        let id = entry.job_id.trim();
        if id != *expected_id {
            return Err(OrbitError::InvalidInput(format!(
                "default job file key '{}' does not match spec job_id '{}'",
                expected_id, id
            )));
        }
        specs.push(entry);
    }
    Ok(specs)
}

pub(crate) fn seed_default_jobs(
    runtime: &OrbitRuntime,
    overwrite: bool,
) -> Result<usize, OrbitError> {
    let specs = load_default_job_specs(DEFAULT_JOB_FILES)?;
    let mut created = 0usize;
    for entry in specs {
        if runtime.show_job(&entry.job_id).is_ok() {
            if !overwrite {
                continue;
            }
            let initial_state = parse_default_job_state(&entry.state, &entry.job_id)?;
            let steps = default_job_steps(&entry)?;
            runtime.update_job_definition(
                &entry.job_id,
                entry.default_input.clone(),
                entry.max_active_runs,
                steps,
                initial_state,
            )?;
            created += 1;
            continue;
        }
        let initial_state = parse_default_job_state(&entry.state, &entry.job_id)?;
        let steps = default_job_steps(&entry)?;
        runtime.add_job(JobAddParams {
            job_id: Some(entry.job_id),
            default_input: entry.default_input,
            max_active_runs: Some(entry.max_active_runs),
            steps,
            initial_state_override: Some(initial_state),
        })?;
        created += 1;
    }
    Ok(created)
}

fn parse_default_job_state(state: &str, job_id: &str) -> Result<JobScheduleState, OrbitError> {
    match state {
        "enabled" => Ok(JobScheduleState::Enabled),
        "disabled" => Ok(JobScheduleState::Disabled),
        other => Err(OrbitError::InvalidInput(format!(
            "unsupported state '{}' in default job '{}'",
            other, job_id
        ))),
    }
}

fn default_job_steps(entry: &DefaultJobEntry) -> Result<Vec<JobStep>, OrbitError> {
    entry
        .steps
        .iter()
        .map(|s| {
            let target_type = match s.target_type.as_str() {
                "activity" => JobTargetType::Activity,
                other => {
                    return Err(OrbitError::InvalidInput(format!(
                        "unsupported target_type '{}' in default job '{}'",
                        other, entry.job_id
                    )));
                }
            };
            Ok(JobStep {
                target_type,
                target_id: s.target_id.clone(),
                agent_cli: s.agent_cli.clone(),
                model: s.model.clone(),
                timeout_seconds: s.timeout_seconds,
                env_extra: s.env_extra.clone(),
            })
        })
        .collect()
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

#[cfg(test)]
mod tests {
    use super::{DEFAULT_JOB_FILES, load_default_job_specs};

    #[test]
    fn bundled_default_job_specs_parse_successfully() {
        let specs =
            load_default_job_specs(DEFAULT_JOB_FILES).expect("bundled default jobs must parse");
        assert_eq!(specs.len(), DEFAULT_JOB_FILES.len());
        let ids = specs
            .iter()
            .map(|spec| spec.job_id.as_str())
            .collect::<Vec<_>>();
        assert_eq!(
            ids,
            vec![
                "job_review_tasks",
                "job_oversee_orbit_operations",
                "job_perform_maintenance",
                "job_task_pipeline",
            ]
        );
        for spec in &specs {
            assert!(!spec.job_id.is_empty(), "job_id must not be empty");
            assert!(spec.max_active_runs > 0, "max_active_runs must be positive");
            assert!(!spec.steps.is_empty(), "steps must not be empty");
            for step in &spec.steps {
                assert!(!step.target_id.is_empty(), "target_id must not be empty");
                assert!(step.timeout_seconds > 0, "timeout_seconds must be positive");
            }
        }
        let pipeline = specs
            .iter()
            .find(|spec| spec.job_id == "job_task_pipeline")
            .expect("task pipeline spec present");
        assert_eq!(pipeline.max_active_runs, 4);
        assert_eq!(pipeline.steps[0].model.as_deref(), Some("gpt-5.4-mini"));
        let review = specs
            .iter()
            .find(|spec| spec.job_id == "job_review_tasks")
            .expect("review job spec present");
        assert_eq!(review.steps[0].model.as_deref(), Some("sonnet-4.5"));
    }

    #[test]
    fn load_rejects_mismatched_file_key_and_job_id() {
        let specs = &[(
            "expected-id",
            "job:\n  job_id: actual-id\n  state: enabled\n  steps:\n    - target_type: activity\n      target_id: t\n      agent_cli: codex\n      timeout_seconds: 60\n",
        )];
        let err = load_default_job_specs(specs).expect_err("must fail");
        assert!(err.to_string().contains("does not match spec job_id"));
    }
}
