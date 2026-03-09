use std::fs;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use orbit_types::{
    Job, JobRetryBackoffStrategy, JobRun, JobRunState, JobScheduleState, JobTargetType, OrbitError,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Clone)]
pub(crate) struct JobFileStore {
    root: PathBuf,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct JobFileDocument {
    schema_version: u8,
    job: Job,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct JobRunFileDocument {
    schema_version: u8,
    run: JobRun,
}

impl JobFileStore {
    pub(crate) fn new(root: PathBuf) -> Self {
        Self { root }
    }

    pub(crate) fn ensure_layout(&self) -> Result<(), OrbitError> {
        fs::create_dir_all(self.activities_dir()).map_err(|e| OrbitError::Io(e.to_string()))?;
        fs::create_dir_all(self.runs_dir()).map_err(|e| OrbitError::Io(e.to_string()))?;
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn insert_activity_v2(
        &self,
        target_type: JobTargetType,
        target_id: &str,
        schedule: &str,
        agent_cli: &str,
        timeout_seconds: u64,
        retry_max_attempts: u32,
        retry_backoff_strategy: JobRetryBackoffStrategy,
        retry_initial_delay_seconds: u64,
        next_run_at: DateTime<Utc>,
    ) -> Result<Job, OrbitError> {
        self.ensure_layout()?;
        let now = Utc::now();
        let job = Job {
            job_id: self.next_id("job"),
            target_type,
            target_id: target_id.to_string(),
            schedule: schedule.to_string(),
            agent_cli: agent_cli.to_string(),
            timeout_seconds,
            retry_max_attempts,
            retry_backoff_strategy,
            retry_initial_delay_seconds,
            state: JobScheduleState::Enabled,
            next_run_at,
            created_at: now,
            updated_at: now,
        };
        self.write_activity(&job)?;
        Ok(job)
    }

    pub(crate) fn list_jobs(&self, include_disabled: bool) -> Result<Vec<Job>, OrbitError> {
        let mut jobs = self.read_all_activities()?;
        if !include_disabled {
            jobs.retain(|job| job.state != JobScheduleState::Disabled);
        }
        jobs.sort_by(|a, b| {
            b.created_at
                .cmp(&a.created_at)
                .then_with(|| a.job_id.cmp(&b.job_id))
        });
        Ok(jobs)
    }

    pub(crate) fn get_job(&self, job_id: &str) -> Result<Option<Job>, OrbitError> {
        let path = self.job_path(job_id);
        if !path.exists() {
            return Ok(None);
        }
        Ok(Some(self.read_activity_at(&path)?))
    }

    pub(crate) fn due_jobs(&self, now: DateTime<Utc>) -> Result<Vec<Job>, OrbitError> {
        let mut jobs = self
            .read_all_activities()?
            .into_iter()
            .filter(|job| job.state == JobScheduleState::Enabled && job.next_run_at <= now)
            .collect::<Vec<_>>();
        jobs.sort_by(|a, b| a.next_run_at.cmp(&b.next_run_at));
        Ok(jobs)
    }

    pub(crate) fn next_due_job_time(&self) -> Result<Option<DateTime<Utc>>, OrbitError> {
        Ok(self
            .read_all_activities()?
            .into_iter()
            .filter(|job| job.state == JobScheduleState::Enabled)
            .map(|job| job.next_run_at)
            .min())
    }

    pub(crate) fn list_job_runs(&self, job_id: &str) -> Result<Vec<JobRun>, OrbitError> {
        let mut runs = self.read_runs_for_activity(job_id)?;
        runs.sort_by(|a, b| {
            b.created_at
                .cmp(&a.created_at)
                .then_with(|| a.run_id.cmp(&b.run_id))
        });
        Ok(runs)
    }

    pub(crate) fn get_pending_or_running_job_run(
        &self,
        job_id: &str,
    ) -> Result<Option<JobRun>, OrbitError> {
        let mut runs = self
            .read_runs_for_activity(job_id)?
            .into_iter()
            .filter(|run| run.state == JobRunState::Pending || run.state == JobRunState::Running)
            .collect::<Vec<_>>();
        runs.sort_by(|a, b| b.created_at.cmp(&a.created_at));
        Ok(runs.into_iter().next())
    }

    pub(crate) fn set_job_state(
        &self,
        job_id: &str,
        state: JobScheduleState,
    ) -> Result<bool, OrbitError> {
        let Some(mut job) = self.get_job(job_id)? else {
            return Ok(false);
        };
        job.state = state;
        job.updated_at = Utc::now();
        self.write_activity(&job)?;
        Ok(true)
    }

    pub(crate) fn mark_job_disabled(&self, job_id: &str) -> Result<bool, OrbitError> {
        self.set_job_state(job_id, JobScheduleState::Disabled)
    }

    pub(crate) fn update_job_next_run(
        &self,
        job_id: &str,
        next_run_at: DateTime<Utc>,
    ) -> Result<bool, OrbitError> {
        let Some(mut job) = self.get_job(job_id)? else {
            return Ok(false);
        };
        job.next_run_at = next_run_at;
        job.updated_at = Utc::now();
        self.write_activity(&job)?;
        Ok(true)
    }

    pub(crate) fn insert_job_run(
        &self,
        job_id: &str,
        attempt: u32,
        scheduled_at: DateTime<Utc>,
    ) -> Result<JobRun, OrbitError> {
        let run = JobRun {
            run_id: self.next_id("jrun"),
            job_id: job_id.to_string(),
            attempt,
            state: JobRunState::Pending,
            scheduled_at,
            started_at: None,
            finished_at: None,
            duration_ms: None,
            exit_code: None,
            agent_response_json: None,
            error_code: None,
            error_message: None,
            created_at: Utc::now(),
        };
        self.write_run(job_id, &run)?;
        Ok(run)
    }

    pub(crate) fn mark_job_run_running(
        &self,
        run_id: &str,
        started_at: DateTime<Utc>,
    ) -> Result<bool, OrbitError> {
        let Some((job_id, path)) = self.find_run_path(run_id)? else {
            return Ok(false);
        };
        let mut run = self.read_run_at(&path)?;
        run.state = JobRunState::Running;
        run.started_at = Some(started_at);
        self.write_run(&job_id, &run)?;
        Ok(true)
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn complete_job_run(
        &self,
        run_id: &str,
        state: JobRunState,
        finished_at: DateTime<Utc>,
        duration_ms: Option<u64>,
        exit_code: Option<i32>,
        agent_response_json: Option<&Value>,
        error_code: Option<&str>,
        error_message: Option<&str>,
    ) -> Result<bool, OrbitError> {
        let Some((job_id, path)) = self.find_run_path(run_id)? else {
            return Ok(false);
        };
        let mut run = self.read_run_at(&path)?;
        run.state = state;
        run.finished_at = Some(finished_at);
        run.duration_ms = duration_ms;
        run.exit_code = exit_code;
        run.agent_response_json = agent_response_json.cloned();
        run.error_code = error_code.map(ToString::to_string);
        run.error_message = error_message.map(ToString::to_string);
        self.write_run(&job_id, &run)?;
        Ok(true)
    }

    pub(crate) fn claim_due_jobs(&self, now: DateTime<Utc>) -> Result<DueJobsClaim, OrbitError> {
        let due_jobs = self.due_jobs(now)?;
        let mut result = DueJobsClaim::default();

        for job in due_jobs {
            if self.get_pending_or_running_job_run(&job.job_id)?.is_some() {
                result.skipped.push(job.job_id.clone());
                continue;
            }
            let run = self.insert_job_run(&job.job_id, 1, now)?;
            result.claimed.push(ClaimedJobRun { job, run });
        }
        Ok(result)
    }

    fn read_all_activities(&self) -> Result<Vec<Job>, OrbitError> {
        self.ensure_layout()?;
        let mut paths = fs::read_dir(self.activities_dir())
            .map_err(|e| OrbitError::Io(e.to_string()))?
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| is_yaml(path))
            .collect::<Vec<_>>();
        paths.sort();
        let mut jobs = Vec::new();
        for path in paths {
            jobs.push(self.read_activity_at(&path)?);
        }
        Ok(jobs)
    }

    fn read_runs_for_activity(&self, job_id: &str) -> Result<Vec<JobRun>, OrbitError> {
        let dir = self.run_dir(job_id);
        if !dir.exists() {
            return Ok(Vec::new());
        }
        let mut paths = fs::read_dir(dir)
            .map_err(|e| OrbitError::Io(e.to_string()))?
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| is_yaml(path))
            .collect::<Vec<_>>();
        paths.sort();
        let mut runs = Vec::new();
        for path in paths {
            runs.push(self.read_run_at(&path)?);
        }
        Ok(runs)
    }

    fn find_run_path(&self, run_id: &str) -> Result<Option<(String, PathBuf)>, OrbitError> {
        let runs_root = self.runs_dir();
        if !runs_root.exists() {
            return Ok(None);
        }
        for entry in fs::read_dir(runs_root).map_err(|e| OrbitError::Io(e.to_string()))? {
            let entry = entry.map_err(|e| OrbitError::Io(e.to_string()))?;
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            let Some(job_id) = path.file_name().and_then(|v| v.to_str()) else {
                continue;
            };
            let run_path = path.join(format!("{run_id}.yaml"));
            if run_path.exists() {
                return Ok(Some((job_id.to_string(), run_path)));
            }
        }
        Ok(None)
    }

    fn read_activity_at(&self, path: &Path) -> Result<Job, OrbitError> {
        let raw = fs::read_to_string(path).map_err(|e| OrbitError::Io(e.to_string()))?;
        let doc = serde_yaml::from_str::<JobFileDocument>(&raw).map_err(|e| {
            OrbitError::Store(format!("invalid job file '{}': {e}", path.display()))
        })?;
        Ok(doc.job)
    }

    fn read_run_at(&self, path: &Path) -> Result<JobRun, OrbitError> {
        let raw = fs::read_to_string(path).map_err(|e| OrbitError::Io(e.to_string()))?;
        let doc = serde_yaml::from_str::<JobRunFileDocument>(&raw).map_err(|e| {
            OrbitError::Store(format!("invalid job run file '{}': {e}", path.display()))
        })?;
        Ok(doc.run)
    }

    fn write_activity(&self, job: &Job) -> Result<(), OrbitError> {
        self.ensure_layout()?;
        let doc = JobFileDocument {
            schema_version: 1,
            job: job.clone(),
        };
        let content = serde_yaml::to_string(&doc).map_err(|e| OrbitError::Store(e.to_string()))?;
        write_atomic(&self.job_path(&job.job_id), &content)
    }

    fn write_run(&self, job_id: &str, run: &JobRun) -> Result<(), OrbitError> {
        self.ensure_layout()?;
        let doc = JobRunFileDocument {
            schema_version: 1,
            run: run.clone(),
        };
        let content = serde_yaml::to_string(&doc).map_err(|e| OrbitError::Store(e.to_string()))?;
        write_atomic(&self.run_path(job_id, &run.run_id), &content)
    }

    fn next_id(&self, prefix: &str) -> String {
        let nanos = Utc::now().timestamp_nanos_opt().unwrap_or_default();
        format!("{prefix}-{nanos}")
    }

    fn activities_dir(&self) -> PathBuf {
        self.root.join("jobs")
    }

    fn runs_dir(&self) -> PathBuf {
        self.root.join("runs")
    }

    fn job_path(&self, job_id: &str) -> PathBuf {
        self.activities_dir().join(format!("{job_id}.yaml"))
    }

    fn run_dir(&self, job_id: &str) -> PathBuf {
        self.runs_dir().join(job_id)
    }

    fn run_path(&self, job_id: &str, run_id: &str) -> PathBuf {
        self.run_dir(job_id).join(format!("{run_id}.yaml"))
    }

    fn archived_runs_dir(&self) -> PathBuf {
        self.runs_dir().join("archived")
    }

    fn archived_run_dir(&self, job_id: &str) -> PathBuf {
        self.archived_runs_dir().join(job_id)
    }

    fn archived_run_path(&self, job_id: &str, run_id: &str) -> PathBuf {
        self.archived_run_dir(job_id).join(format!("{run_id}.yaml"))
    }

    pub(crate) fn archive_run(&self, run_id: &str) -> Result<String, OrbitError> {
        let Some((job_id, src)) = self.find_run_path(run_id)? else {
            return Err(OrbitError::JobRunNotFound(run_id.to_string()));
        };
        let dst = self.archived_run_path(&job_id, run_id);
        let parent = dst.parent().ok_or_else(|| {
            OrbitError::Io(format!("cannot determine parent for '{}'", dst.display()))
        })?;
        fs::create_dir_all(parent).map_err(|e| OrbitError::Io(e.to_string()))?;
        fs::rename(&src, &dst).map_err(|e| OrbitError::Io(e.to_string()))?;
        Ok(job_id)
    }
}

fn write_atomic(path: &Path, content: &str) -> Result<(), OrbitError> {
    let parent = path.parent().ok_or_else(|| {
        OrbitError::Io(format!("cannot determine parent for '{}'", path.display()))
    })?;
    fs::create_dir_all(parent).map_err(|e| OrbitError::Io(e.to_string()))?;

    let mut tmp = path.to_path_buf();
    let nanos = Utc::now().timestamp_nanos_opt().unwrap_or_default();
    tmp.set_extension(format!("yaml.tmp.{nanos}"));
    fs::write(&tmp, content).map_err(|e| OrbitError::Io(e.to_string()))?;
    if let Err(err) = fs::rename(&tmp, path) {
        let _ = fs::remove_file(&tmp);
        return Err(OrbitError::Io(err.to_string()));
    }
    Ok(())
}

fn is_yaml(path: &Path) -> bool {
    path.extension()
        .and_then(|value| value.to_str())
        .is_some_and(|ext| ext.eq_ignore_ascii_case("yaml") || ext.eq_ignore_ascii_case("yml"))
}
use crate::{ClaimedJobRun, DueJobsClaim};

#[cfg(test)]
mod tests {
    use chrono::Utc;
    use orbit_types::{JobRetryBackoffStrategy, JobTargetType, OrbitError};

    use super::JobFileStore;

    fn make_store() -> (tempfile::TempDir, JobFileStore) {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = JobFileStore::new(dir.path().to_path_buf());
        (dir, store)
    }

    #[test]
    fn archive_run_moves_file_to_archived_dir() {
        let (_dir, store) = make_store();
        let now = Utc::now();
        let job = store
            .insert_activity_v2(
                JobTargetType::Activity,
                "target-1",
                "every 1h",
                "mock-agent",
                300,
                0,
                JobRetryBackoffStrategy::None,
                0,
                now,
            )
            .expect("insert job");
        let run = store
            .insert_job_run(&job.job_id, 1, now)
            .expect("insert run");

        let src = store.run_path(&job.job_id, &run.run_id);
        assert!(src.exists(), "run file must exist before archive");

        store.archive_run(&run.run_id).expect("archive run");

        assert!(!src.exists(), "run file must be gone after archive");
        let dst = store.archived_run_path(&job.job_id, &run.run_id);
        assert!(dst.exists(), "archived run file must exist");
    }

    #[test]
    fn archive_run_returns_error_for_unknown_run() {
        let (_dir, store) = make_store();
        let err = store.archive_run("jrun-does-not-exist").unwrap_err();
        assert!(
            matches!(err, OrbitError::JobRunNotFound(_)),
            "expected JobRunNotFound, got {err:?}"
        );
    }
}
