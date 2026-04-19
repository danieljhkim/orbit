use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use orbit_engine::EnvironmentHost;
use orbit_store::JobCreateParams as StoreActivityCreateParams;
use orbit_store::JobUpdateParams as StoreJobUpdateParams;
use orbit_types::{
    Job, JobAsset, JobKind, JobResource, JobRun, JobScheduleState, JobStep, JobTargetType, JobV2,
    OrbitError, OrbitEvent, RESOURCE_SCHEMA_VERSION, ResourceKind, default_job_max_active_runs,
    load_job_asset, resolve_agent_model_pair,
};
use serde_json::Value;

use crate::OrbitRuntime;
use crate::command::activity::activity_requires_agent_cli;

const JOB_PARALLEL_TASK_PIPELINE: &str = "job_parallel_task_pipeline";
const JOB_LOCAL_TASK_PIPELINE: &str = "job_local_task_pipeline";
const REPO_V2_SAMPLE_JOBS_DIR: &str = "crates/orbit-core/assets/jobs/v2_samples";
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
    // Legacy compatibility shim for callers that still deserialize job resources
    // with a per-job policy field. This input is intentionally ignored.
    pub policy: Option<String>,
    pub initial_state_override: Option<JobScheduleState>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JobCatalogFilter {
    WorkflowsOnly,
    All,
    Kind(JobKind),
}

#[derive(Debug, Clone)]
pub enum JobCatalogDefinition {
    Legacy(Job),
    V2 { path: PathBuf, spec: JobV2 },
}

#[derive(Debug, Clone)]
pub struct JobCatalogEntry {
    pub job_id: String,
    pub kind: JobKind,
    pub callers: Vec<String>,
    pub orphaned_subroutine: bool,
    pub definition: JobCatalogDefinition,
}

impl JobCatalogEntry {
    pub fn schema_version(&self) -> u32 {
        match self.definition {
            JobCatalogDefinition::Legacy(_) => 1,
            JobCatalogDefinition::V2 { .. } => 2,
        }
    }

    pub fn state(&self) -> JobScheduleState {
        match &self.definition {
            JobCatalogDefinition::Legacy(job) => job.state,
            JobCatalogDefinition::V2 { spec, .. } => spec.state,
        }
    }

    pub fn max_active_runs(&self) -> u32 {
        match &self.definition {
            JobCatalogDefinition::Legacy(job) => job.max_active_runs,
            JobCatalogDefinition::V2 { spec, .. } => spec.max_active_runs,
        }
    }

    pub fn default_input(&self) -> Option<&Value> {
        match &self.definition {
            JobCatalogDefinition::Legacy(job) => job.default_input.as_ref(),
            JobCatalogDefinition::V2 { spec, .. } => spec.default_input.as_ref(),
        }
    }

    pub fn legacy_job(&self) -> Option<&Job> {
        match &self.definition {
            JobCatalogDefinition::Legacy(job) => Some(job),
            JobCatalogDefinition::V2 { .. } => None,
        }
    }

    pub fn v2_job(&self) -> Option<&JobV2> {
        match &self.definition {
            JobCatalogDefinition::Legacy(_) => None,
            JobCatalogDefinition::V2 { spec, .. } => Some(spec),
        }
    }

    pub fn v2_job_path(&self) -> Option<&Path> {
        match &self.definition {
            JobCatalogDefinition::Legacy(_) => None,
            JobCatalogDefinition::V2 { path, .. } => Some(path.as_path()),
        }
    }
}

#[derive(Debug, Clone)]
struct V2JobAssetEntry {
    path: PathBuf,
    spec: JobV2,
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
            policy: None,
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
        _policy: Option<String>,
        state: JobScheduleState,
    ) -> Result<Job, OrbitError> {
        let steps = normalize_job_steps(steps)?;
        let job = self.stores().jobs().update(
            job_id,
            StoreJobUpdateParams {
                default_input: Some(normalize_job_default_input(default_input)?),
                max_active_runs: Some(validate_job_max_active_runs(Some(max_active_runs))?),
                max_iterations: Some(max_iterations),
                steps: Some(steps),
                policy: None,
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

    pub fn list_job_catalog_with_last_run(
        &self,
        include_disabled: bool,
        filter: JobCatalogFilter,
    ) -> Result<Vec<(JobCatalogEntry, Option<JobRun>)>, OrbitError> {
        use orbit_store::JobRunQuery;

        let legacy_jobs = self.list_jobs_backend(true)?;
        let v2_jobs = self.load_v2_job_assets()?;
        let callers = build_job_callers(&legacy_jobs);
        emit_orphan_subroutine_warnings(&v2_jobs, &callers);

        let now = Utc::now();
        let mut result = Vec::new();

        for (job_id, asset) in &v2_jobs {
            if !include_disabled && asset.spec.state == JobScheduleState::Disabled {
                continue;
            }
            if !matches_job_filter(asset.spec.kind, filter) {
                continue;
            }
            let last_run = self
                .stores()
                .jobs()
                .list_runs_filtered(&JobRunQuery {
                    job_id: Some(job_id.clone()),
                    state: None,
                    created_since: None,
                    limit: Some(1),
                })
                .ok()
                .and_then(|runs| runs.into_iter().next());
            result.push((
                JobCatalogEntry {
                    job_id: job_id.clone(),
                    kind: asset.spec.kind,
                    callers: callers_for(&callers, job_id),
                    orphaned_subroutine: is_orphaned_subroutine(job_id, &asset.spec, &callers),
                    definition: JobCatalogDefinition::V2 {
                        path: asset.path.clone(),
                        spec: asset.spec.clone(),
                    },
                },
                last_run,
            ));
        }

        for job in legacy_jobs {
            if v2_jobs.contains_key(&job.job_id) {
                continue;
            }
            if !include_disabled && job.state == JobScheduleState::Disabled {
                continue;
            }
            if !matches_job_filter(JobKind::Workflow, filter) {
                continue;
            }
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
            result.push((
                JobCatalogEntry {
                    job_id: job.job_id.clone(),
                    kind: JobKind::Workflow,
                    callers: callers_for(&callers, &job.job_id),
                    orphaned_subroutine: false,
                    definition: JobCatalogDefinition::Legacy(job),
                },
                last_run,
            ));
        }

        result.sort_by(|left, right| left.0.job_id.cmp(&right.0.job_id));
        Ok(result)
    }

    pub fn show_job_catalog_entry(&self, job_id: &str) -> Result<JobCatalogEntry, OrbitError> {
        let legacy_jobs = self.list_jobs_backend(true)?;
        let v2_jobs = self.load_v2_job_assets()?;
        let callers = build_job_callers(&legacy_jobs);
        emit_orphan_subroutine_warnings(&v2_jobs, &callers);

        if let Some(asset) = v2_jobs.get(job_id) {
            return Ok(JobCatalogEntry {
                job_id: job_id.to_string(),
                kind: asset.spec.kind,
                callers: callers_for(&callers, job_id),
                orphaned_subroutine: is_orphaned_subroutine(job_id, &asset.spec, &callers),
                definition: JobCatalogDefinition::V2 {
                    path: asset.path.clone(),
                    spec: asset.spec.clone(),
                },
            });
        }

        let job = legacy_jobs
            .into_iter()
            .find(|candidate| candidate.job_id == job_id)
            .ok_or_else(|| OrbitError::JobNotFound(job_id.to_string()))?;

        Ok(JobCatalogEntry {
            job_id: job_id.to_string(),
            kind: JobKind::Workflow,
            callers: callers_for(&callers, job_id),
            orphaned_subroutine: false,
            definition: JobCatalogDefinition::Legacy(job),
        })
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

    fn load_v2_job_assets(&self) -> Result<BTreeMap<String, V2JobAssetEntry>, OrbitError> {
        let mut entries = BTreeMap::new();
        let mut sources = BTreeMap::new();
        for dir in self.v2_job_asset_dirs() {
            if dir.is_dir() {
                load_v2_job_assets_from_dir(&dir, &mut entries, &mut sources)?;
            }
        }
        Ok(entries)
    }

    fn v2_job_asset_dirs(&self) -> Vec<PathBuf> {
        let mut dirs = Vec::new();

        if let Ok(raw) = std::env::var("ORBIT_V2_JOB_DIR") {
            for entry in raw.split(':').filter(|value| !value.is_empty()) {
                dirs.push(PathBuf::from(entry));
            }
        }

        dirs.push(self.paths().orbit_dir.join("jobs/v2"));
        dirs.push(self.paths().global_dir.join("jobs/v2"));
        dirs.push(self.paths().repo_root.join(REPO_V2_SAMPLE_JOBS_DIR));
        dirs
    }

    pub(crate) fn load_v2_job_asset_by_name(
        &self,
        job_id: &str,
    ) -> Result<(PathBuf, JobV2), OrbitError> {
        let mut found = None;
        for dir in self.v2_job_asset_dirs() {
            if dir.is_dir() {
                find_v2_job_asset_in_dir(&dir, job_id, &mut found)?;
            }
        }
        found.ok_or_else(|| OrbitError::JobNotFound(job_id.to_string()))
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

fn matches_job_filter(kind: JobKind, filter: JobCatalogFilter) -> bool {
    match filter {
        JobCatalogFilter::WorkflowsOnly => kind == JobKind::Workflow,
        JobCatalogFilter::All => true,
        JobCatalogFilter::Kind(expected) => kind == expected,
    }
}

fn build_job_callers(legacy_jobs: &[Job]) -> BTreeMap<String, BTreeSet<String>> {
    let mut callers = BTreeMap::new();
    for job in legacy_jobs {
        for step in &job.steps {
            if step.target_type == JobTargetType::Job {
                callers
                    .entry(step.target_id.clone())
                    .or_insert_with(BTreeSet::new)
                    .insert(job.job_id.clone());
            }
        }
    }
    callers
}

fn callers_for(callers: &BTreeMap<String, BTreeSet<String>>, job_id: &str) -> Vec<String> {
    callers
        .get(job_id)
        .map(|items| items.iter().cloned().collect())
        .unwrap_or_default()
}

fn is_orphaned_subroutine(
    job_id: &str,
    spec: &JobV2,
    callers: &BTreeMap<String, BTreeSet<String>>,
) -> bool {
    spec.kind == JobKind::Subroutine && callers.get(job_id).is_none_or(BTreeSet::is_empty)
}

fn emit_orphan_subroutine_warnings(
    v2_jobs: &BTreeMap<String, V2JobAssetEntry>,
    callers: &BTreeMap<String, BTreeSet<String>>,
) {
    for (job_id, asset) in v2_jobs {
        if is_orphaned_subroutine(job_id, &asset.spec, callers) {
            eprintln!(
                "orbit: warning: subroutine job '{}' at {} has no callers in the loaded job corpus",
                job_id,
                asset.path.display()
            );
        }
    }
}

fn load_v2_job_assets_from_dir(
    dir: &Path,
    entries: &mut BTreeMap<String, V2JobAssetEntry>,
    sources: &mut BTreeMap<String, PathBuf>,
) -> Result<(), OrbitError> {
    let iter = std::fs::read_dir(dir)
        .map_err(|err| OrbitError::InvalidInput(format!("read dir {}: {err}", dir.display())))?;
    for entry in iter {
        let entry = entry.map_err(|err| {
            OrbitError::InvalidInput(format!("read dir {}: {err}", dir.display()))
        })?;
        let path = entry.path();
        if path.is_dir() {
            load_v2_job_assets_from_dir(&path, entries, sources)?;
            continue;
        }
        let is_yaml = path
            .extension()
            .and_then(|ext| ext.to_str())
            .is_some_and(|ext| ext == "yaml" || ext == "yml");
        if !is_yaml {
            continue;
        }
        let yaml = std::fs::read_to_string(&path).map_err(|err| {
            OrbitError::InvalidInput(format!("read file {}: {err}", path.display()))
        })?;
        let asset = load_job_asset(&yaml)
            .map_err(|err| OrbitError::InvalidInput(format!("parse {}: {err}", path.display())))?;
        let JobAsset::V2(asset) = asset else {
            continue;
        };
        if let Some(first) = sources.get(&asset.name) {
            return Err(OrbitError::InvalidInput(format!(
                "duplicate v2 job name '{}' — defined in both {} and {}",
                asset.name,
                first.display(),
                path.display()
            )));
        }
        sources.insert(asset.name.clone(), path.clone());
        entries.insert(
            asset.name,
            V2JobAssetEntry {
                path,
                spec: asset.spec,
            },
        );
    }
    Ok(())
}

fn find_v2_job_asset_in_dir(
    dir: &Path,
    expected_job_id: &str,
    found: &mut Option<(PathBuf, JobV2)>,
) -> Result<(), OrbitError> {
    let iter = std::fs::read_dir(dir)
        .map_err(|err| OrbitError::InvalidInput(format!("read dir {}: {err}", dir.display())))?;
    for entry in iter {
        let entry = entry.map_err(|err| {
            OrbitError::InvalidInput(format!("read dir {}: {err}", dir.display()))
        })?;
        let path = entry.path();
        if path.is_dir() {
            find_v2_job_asset_in_dir(&path, expected_job_id, found)?;
            continue;
        }
        let is_yaml = path
            .extension()
            .and_then(|ext| ext.to_str())
            .is_some_and(|ext| ext == "yaml" || ext == "yml");
        if !is_yaml {
            continue;
        }

        let yaml = match std::fs::read_to_string(&path) {
            Ok(yaml) => yaml,
            Err(_) => continue,
        };
        let asset = match load_job_asset(&yaml) {
            Ok(JobAsset::V2(asset)) => asset,
            _ => continue,
        };
        if asset.name != expected_job_id {
            continue;
        }
        if let Some((first_path, _)) = found {
            return Err(OrbitError::InvalidInput(format!(
                "duplicate v2 job name '{}' — defined in both {} and {}",
                expected_job_id,
                first_path.display(),
                path.display()
            )));
        }
        *found = Some((path, asset.spec));
    }
    Ok(())
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
            policy: None,
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
