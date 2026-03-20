use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use chrono::{DateTime, Utc};
use orbit_types::{Job, JobRun, JobRunState, JobRunStep, JobScheduleState, OrbitError};

use super::contracts::{
    JobCreateParams, JobRunQuery, JobRunStepParams, JobStoreBackend, JobUpdateParams,
};

#[derive(Default)]
struct JobStoreState {
    jobs: HashMap<String, Job>,
    active_runs: HashMap<String, JobRun>,
    archived_runs: HashMap<String, JobRun>,
}

#[derive(Clone, Default)]
pub struct MemoryJobStoreBackend {
    inner: Arc<Mutex<JobStoreState>>,
}

fn lock_err<T>(e: std::sync::PoisonError<T>) -> OrbitError {
    OrbitError::Store(format!("mutex poisoned: {e}"))
}

fn validate_max_active_runs(max_active_runs: u32) -> Result<u32, OrbitError> {
    if max_active_runs == 0 {
        return Err(OrbitError::JobValidation(
            "job max_active_runs must be at least 1".to_string(),
        ));
    }
    Ok(max_active_runs)
}

fn next_job_id(state: &JobStoreState) -> String {
    let now = Utc::now();
    let base = format!("job-{}", now.format("%Y%m%d-%H%M%S"));
    if !state.jobs.contains_key(&base) {
        return base;
    }
    for suffix in 2..1024_u32 {
        let candidate = format!("{base}-{suffix}");
        if !state.jobs.contains_key(&candidate) {
            return candidate;
        }
    }
    base
}

fn next_run_id(state: &JobStoreState) -> String {
    let now = Utc::now();
    let base = format!("jrun-{}", now.format("%Y%m%d-%H%M%S"));
    if !state.active_runs.contains_key(&base) && !state.archived_runs.contains_key(&base) {
        return base;
    }
    for suffix in 2..1024_u32 {
        let candidate = format!("{base}-{suffix}");
        if !state.active_runs.contains_key(&candidate)
            && !state.archived_runs.contains_key(&candidate)
        {
            return candidate;
        }
    }
    base
}

impl JobStoreBackend for MemoryJobStoreBackend {
    fn add_job(&self, params: JobCreateParams) -> Result<Job, OrbitError> {
        validate_max_active_runs(params.max_active_runs)?;
        let mut state = self.inner.lock().map_err(lock_err)?;
        let job_id = match params.job_id {
            Some(id) => {
                if state.jobs.contains_key(&id) {
                    return Err(OrbitError::JobValidation(format!(
                        "job id already exists: {id}"
                    )));
                }
                id
            }
            None => next_job_id(&state),
        };
        let now = Utc::now();
        let job = Job {
            job_id: job_id.clone(),
            state: params.initial_state,
            default_input: params.default_input,
            max_active_runs: params.max_active_runs,
            steps: params.steps,
            created_at: now,
            updated_at: now,
        };
        state.jobs.insert(job_id, job.clone());
        Ok(job)
    }

    fn update_job(&self, job_id: &str, params: JobUpdateParams) -> Result<Job, OrbitError> {
        let mut state = self.inner.lock().map_err(lock_err)?;
        let Some(job) = state.jobs.get_mut(job_id) else {
            return Err(OrbitError::JobNotFound(job_id.to_string()));
        };
        if let Some(v) = params.default_input {
            job.default_input = v;
        }
        if let Some(v) = params.max_active_runs {
            validate_max_active_runs(v)?;
            job.max_active_runs = v;
        }
        if let Some(v) = params.steps {
            job.steps = v;
        }
        if let Some(v) = params.state {
            job.state = v;
        }
        job.updated_at = Utc::now();
        Ok(job.clone())
    }

    fn list_jobs(&self, include_disabled: bool) -> Result<Vec<Job>, OrbitError> {
        let state = self.inner.lock().map_err(lock_err)?;
        let mut jobs: Vec<Job> = state
            .jobs
            .values()
            .filter(|j| include_disabled || j.state != JobScheduleState::Disabled)
            .cloned()
            .collect();
        jobs.sort_by(|a, b| {
            b.created_at
                .cmp(&a.created_at)
                .then_with(|| a.job_id.cmp(&b.job_id))
        });
        Ok(jobs)
    }

    fn get_job(&self, job_id: &str) -> Result<Option<Job>, OrbitError> {
        let state = self.inner.lock().map_err(lock_err)?;
        Ok(state.jobs.get(job_id).cloned())
    }

    fn list_job_runs(&self, job_id: &str) -> Result<Vec<JobRun>, OrbitError> {
        let state = self.inner.lock().map_err(lock_err)?;
        let mut runs: Vec<JobRun> = state
            .active_runs
            .values()
            .filter(|r| r.job_id == job_id)
            .cloned()
            .collect();
        runs.sort_by(|a, b| {
            b.created_at
                .cmp(&a.created_at)
                .then_with(|| a.run_id.cmp(&b.run_id))
        });
        Ok(runs)
    }

    fn list_job_runs_filtered(&self, query: &JobRunQuery) -> Result<Vec<JobRun>, OrbitError> {
        let state = self.inner.lock().map_err(lock_err)?;
        let mut runs: Vec<JobRun> = if let Some(ref job_id) = query.job_id {
            state
                .active_runs
                .values()
                .filter(|r| &r.job_id == job_id)
                .cloned()
                .collect()
        } else {
            state.active_runs.values().cloned().collect()
        };
        if let Some(s) = query.state {
            runs.retain(|r| r.state == s);
        }
        if let Some(since) = query.created_since {
            runs.retain(|r| r.created_at >= since);
        }
        runs.sort_by(|a, b| {
            b.created_at
                .cmp(&a.created_at)
                .then_with(|| a.run_id.cmp(&b.run_id))
        });
        if let Some(limit) = query.limit {
            runs.truncate(limit);
        }
        Ok(runs)
    }

    fn get_job_run(&self, run_id: &str) -> Result<Option<JobRun>, OrbitError> {
        let state = self.inner.lock().map_err(lock_err)?;
        Ok(state.active_runs.get(run_id).cloned())
    }

    fn list_pending_or_running_job_runs(&self, job_id: &str) -> Result<Vec<JobRun>, OrbitError> {
        let state = self.inner.lock().map_err(lock_err)?;
        let mut runs: Vec<JobRun> = state
            .active_runs
            .values()
            .filter(|r| {
                r.job_id == job_id
                    && (r.state == JobRunState::Pending || r.state == JobRunState::Running)
            })
            .cloned()
            .collect();
        runs.sort_by(|a, b| b.created_at.cmp(&a.created_at));
        Ok(runs)
    }

    fn set_job_state(&self, job_id: &str, state: JobScheduleState) -> Result<bool, OrbitError> {
        let mut inner = self.inner.lock().map_err(lock_err)?;
        let Some(job) = inner.jobs.get_mut(job_id) else {
            return Ok(false);
        };
        job.state = state;
        job.updated_at = Utc::now();
        Ok(true)
    }

    fn mark_job_disabled(&self, job_id: &str) -> Result<bool, OrbitError> {
        self.set_job_state(job_id, JobScheduleState::Disabled)
    }

    fn insert_job_run(
        &self,
        job_id: &str,
        attempt: u32,
        scheduled_at: DateTime<Utc>,
    ) -> Result<JobRun, OrbitError> {
        let mut state = self.inner.lock().map_err(lock_err)?;
        let run_id = next_run_id(&state);
        let run = JobRun {
            run_id: run_id.clone(),
            job_id: job_id.to_string(),
            attempt,
            state: JobRunState::Pending,
            scheduled_at,
            started_at: None,
            finished_at: None,
            duration_ms: None,
            pid: None,
            created_at: Utc::now(),
            steps: vec![],
        };
        state.active_runs.insert(run_id, run.clone());
        Ok(run)
    }

    fn mark_job_run_running(
        &self,
        run_id: &str,
        started_at: DateTime<Utc>,
        pid: u32,
    ) -> Result<bool, OrbitError> {
        let mut state = self.inner.lock().map_err(lock_err)?;
        let Some(run) = state.active_runs.get_mut(run_id) else {
            return Ok(false);
        };
        run.state = JobRunState::Running;
        run.started_at = Some(started_at);
        run.pid = Some(pid);
        Ok(true)
    }

    fn abandon_job_run(
        &self,
        run_id: &str,
        finished_at: DateTime<Utc>,
    ) -> Result<bool, OrbitError> {
        self.finalize_job_run(run_id, JobRunState::Failed, finished_at, None)
    }

    fn complete_job_run_step(
        &self,
        run_id: &str,
        params: &JobRunStepParams,
    ) -> Result<bool, OrbitError> {
        let mut state = self.inner.lock().map_err(lock_err)?;
        let Some(run) = state.active_runs.get_mut(run_id) else {
            return Ok(false);
        };
        let step = JobRunStep {
            step_index: params.step_index as u32,
            target_type: params.target_type,
            target_id: params.target_id.clone(),
            started_at: Some(params.started_at),
            finished_at: Some(params.finished_at),
            duration_ms: params.duration_ms,
            exit_code: params.exit_code,
            agent_response_json: params.agent_response_json.clone(),
            state: params.state,
            error_code: params.error_code.clone(),
            error_message: params.error_message.clone(),
        };
        if let Some(existing) = run
            .steps
            .iter_mut()
            .find(|s| s.step_index == params.step_index as u32)
        {
            *existing = step;
        } else {
            run.steps.push(step);
        }
        Ok(true)
    }

    fn finalize_job_run(
        &self,
        run_id: &str,
        state: JobRunState,
        finished_at: DateTime<Utc>,
        duration_ms: Option<u64>,
    ) -> Result<bool, OrbitError> {
        let mut inner = self.inner.lock().map_err(lock_err)?;
        let Some(run) = inner.active_runs.get_mut(run_id) else {
            return Ok(false);
        };
        run.state = state;
        run.finished_at = Some(finished_at);
        run.duration_ms = duration_ms;
        Ok(true)
    }

    fn archive_job_run(&self, run_id: &str) -> Result<String, OrbitError> {
        let mut state = self.inner.lock().map_err(lock_err)?;
        let Some(run) = state.active_runs.remove(run_id) else {
            return Err(OrbitError::JobRunNotFound(run_id.to_string()));
        };
        let job_id = run.job_id.clone();
        state.archived_runs.insert(run_id.to_string(), run);
        Ok(job_id)
    }

    fn delete_job_run(&self, run_id: &str) -> Result<String, OrbitError> {
        let mut state = self.inner.lock().map_err(lock_err)?;
        if let Some(run) = state.active_runs.remove(run_id) {
            return Ok(run.job_id);
        }
        if let Some(run) = state.archived_runs.remove(run_id) {
            return Ok(run.job_id);
        }
        Err(OrbitError::JobRunNotFound(run_id.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use chrono::Utc;
    use orbit_types::{JobRunState, JobScheduleState, JobStep, JobTargetType};

    use super::MemoryJobStoreBackend;
    use crate::backend::contracts::{JobCreateParams, JobRunStepParams, JobStoreBackend};

    fn make_params(target_id: &str) -> JobCreateParams {
        JobCreateParams {
            job_id: None,
            default_input: None,
            max_active_runs: 1,
            steps: vec![JobStep {
                target_type: JobTargetType::Activity,
                target_id: target_id.to_string(),
                agent_cli: "mock".to_string(),
                model: None,
                timeout_seconds: 300,
                env_extra: vec![],
                retry_max_attempts: 0,
                retry_backoff_seconds: 10,
            }],
            initial_state: JobScheduleState::Enabled,
        }
    }

    #[test]
    fn add_and_get_job_roundtrip() {
        let store = MemoryJobStoreBackend::default();
        let job = store.add_job(make_params("act-1")).expect("add");
        assert!(job.job_id.starts_with("job-"));

        let got = store.get_job(&job.job_id).expect("get").expect("exists");
        assert_eq!(got.job_id, job.job_id);
        assert_eq!(got.state, JobScheduleState::Enabled);
    }

    #[test]
    fn insert_run_and_lifecycle() {
        let store = MemoryJobStoreBackend::default();
        let job = store.add_job(make_params("act-lifecycle")).expect("add");
        let now = Utc::now();

        let run = store
            .insert_job_run(&job.job_id, 1, now)
            .expect("insert run");
        assert_eq!(run.state, JobRunState::Pending);
        assert!(run.run_id.starts_with("jrun-"));

        store
            .mark_job_run_running(&run.run_id, now, 99999)
            .expect("mark running");

        let step_params = JobRunStepParams {
            step_index: 0,
            target_type: JobTargetType::Activity,
            target_id: "act-lifecycle".to_string(),
            started_at: now,
            finished_at: now,
            duration_ms: Some(42),
            exit_code: Some(0),
            agent_response_json: None,
            state: JobRunState::Success,
            error_code: None,
            error_message: None,
        };
        store
            .complete_job_run_step(&run.run_id, &step_params)
            .expect("complete step");

        store
            .finalize_job_run(&run.run_id, JobRunState::Success, now, Some(42))
            .expect("finalize");

        let got = store
            .get_job_run(&run.run_id)
            .expect("get")
            .expect("exists");
        assert_eq!(got.state, JobRunState::Success);
        assert_eq!(got.steps.len(), 1);
        assert_eq!(got.steps[0].exit_code, Some(0));
    }

    #[test]
    fn archive_and_delete_run() {
        let store = MemoryJobStoreBackend::default();
        let job = store.add_job(make_params("act-archive")).expect("add");
        let run = store
            .insert_job_run(&job.job_id, 1, Utc::now())
            .expect("insert run");

        store.archive_job_run(&run.run_id).expect("archive");
        // After archiving, get_job_run returns None (archived not in active)
        assert!(store.get_job_run(&run.run_id).expect("get").is_none());

        // Delete from archived
        store.delete_job_run(&run.run_id).expect("delete archived");
        let err = store.delete_job_run(&run.run_id).unwrap_err();
        assert!(matches!(err, orbit_types::OrbitError::JobRunNotFound(_)));
    }

    #[test]
    fn list_pending_or_running_excludes_finished() {
        let store = MemoryJobStoreBackend::default();
        let job = store.add_job(make_params("act-active")).expect("add");
        let now = Utc::now();

        let r1 = store
            .insert_job_run(&job.job_id, 1, now)
            .expect("insert r1");
        let r2 = store
            .insert_job_run(&job.job_id, 2, now)
            .expect("insert r2");
        let r3 = store
            .insert_job_run(&job.job_id, 3, now)
            .expect("insert r3");
        store
            .finalize_job_run(&r3.run_id, JobRunState::Success, now, None)
            .expect("finalize r3");

        let active = store
            .list_pending_or_running_job_runs(&job.job_id)
            .expect("list active");
        let ids: Vec<_> = active.iter().map(|r| r.run_id.as_str()).collect();
        assert_eq!(active.len(), 2);
        assert!(ids.contains(&r1.run_id.as_str()));
        assert!(ids.contains(&r2.run_id.as_str()));
        assert!(!ids.contains(&r3.run_id.as_str()));
    }
}
