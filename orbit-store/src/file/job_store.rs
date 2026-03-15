use std::fs;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use orbit_types::{Job, JobRun, JobRunState, JobRunStep, JobScheduleState, JobStep, OrbitError};
use serde::{Deserialize, Serialize};

use crate::backend::JobRunStepParams;

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

/// Serialized to jrun.yaml — contains run-level fields only.
/// Step-level fields (exit_code, agent_response_json, etc.) live in steps/*.yaml.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct JobRunFileDocument {
    schema_version: u8,
    run: JobRun,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct JobRunStepFileDocument {
    schema_version: u8,
    step: JobRunStep,
}

impl JobFileStore {
    pub(crate) fn new(root: PathBuf) -> Self {
        Self { root }
    }

    pub(crate) fn ensure_layout(&self) -> Result<(), OrbitError> {
        fs::create_dir_all(self.activities_dir()).map_err(|e| OrbitError::Io(e.to_string()))?;
        fs::create_dir_all(self.disabled_jobs_dir()).map_err(|e| OrbitError::Io(e.to_string()))?;
        fs::create_dir_all(self.runs_dir()).map_err(|e| OrbitError::Io(e.to_string()))?;
        Ok(())
    }

    pub(crate) fn insert_activity_v2(
        &self,
        job_id: Option<String>,
        steps: Vec<JobStep>,
        initial_state: JobScheduleState,
    ) -> Result<Job, OrbitError> {
        self.ensure_layout()?;
        let resolved_id = match job_id {
            Some(id) => {
                if self.job_path(&id).exists() {
                    return Err(OrbitError::JobValidation(format!(
                        "job id already exists: {id}"
                    )));
                }
                id
            }
            None => self.next_job_id(),
        };
        let now = Utc::now();
        let job = Job {
            job_id: resolved_id,
            state: initial_state,
            steps,
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
        if path.exists() {
            return Ok(Some(self.read_activity_at(&path)?));
        }
        let disabled_path = self.disabled_job_path(job_id);
        if disabled_path.exists() {
            return Ok(Some(self.read_activity_at(&disabled_path)?));
        }
        Ok(None)
    }

    pub(crate) fn update_job(
        &self,
        job_id: &str,
        steps: Option<Vec<JobStep>>,
        state: Option<JobScheduleState>,
    ) -> Result<Job, OrbitError> {
        self.ensure_layout()?;
        let Some(mut job) = self.get_job(job_id)? else {
            return Err(OrbitError::JobNotFound(job_id.to_string()));
        };

        if let Some(steps) = steps {
            job.steps = steps;
        }
        if let Some(state) = state {
            job.state = state;
        }
        job.updated_at = Utc::now();

        self.write_activity(&job)?;
        let disabled_path = self.disabled_job_path(job_id);
        let active_path = self.job_path(job_id);
        match job.state {
            JobScheduleState::Enabled => {
                if disabled_path.exists() {
                    fs::remove_file(&disabled_path).map_err(|e| OrbitError::Io(e.to_string()))?;
                }
            }
            JobScheduleState::Disabled => {
                if active_path.exists() {
                    fs::remove_file(&active_path).map_err(|e| OrbitError::Io(e.to_string()))?;
                }
            }
        }

        Ok(job)
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

    pub(crate) fn list_job_runs_filtered(
        &self,
        query: &crate::backend::JobRunQuery,
    ) -> Result<Vec<JobRun>, OrbitError> {
        let mut runs = if let Some(job_id) = query.job_id.as_deref() {
            self.read_runs_for_activity(job_id)?
        } else {
            self.read_all_runs()?
        };

        if let Some(state) = query.state {
            runs.retain(|run| run.state == state);
        }
        if let Some(created_since) = query.created_since {
            runs.retain(|run| run.created_at >= created_since);
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

    pub(crate) fn get_job_run(&self, run_id: &str) -> Result<Option<JobRun>, OrbitError> {
        let Some((_job_id, run_dir)) = self.find_run_path(run_id)? else {
            return Ok(None);
        };
        Ok(Some(self.read_run_at(&run_dir)?))
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
        if state == JobScheduleState::Disabled {
            return self.mark_job_disabled(job_id);
        }
        job.state = state;
        job.updated_at = Utc::now();
        self.write_activity(&job)?;
        // If the job was previously in disabled/, remove that stale copy.
        let disabled_path = self.disabled_job_path(job_id);
        if disabled_path.exists() {
            fs::remove_file(&disabled_path).map_err(|e| OrbitError::Io(e.to_string()))?;
        }
        Ok(true)
    }

    pub(crate) fn mark_job_disabled(&self, job_id: &str) -> Result<bool, OrbitError> {
        let Some(mut job) = self.get_job(job_id)? else {
            return Ok(false);
        };
        // If already in disabled/, nothing to move.
        let disabled_path = self.disabled_job_path(job_id);
        if disabled_path.exists() {
            return Ok(true);
        }
        job.state = JobScheduleState::Disabled;
        job.updated_at = Utc::now();
        // Write updated state to disabled/ then remove the active file.
        let content = serde_yaml::to_string(&JobFileDocument {
            schema_version: 1,
            job: job.clone(),
        })
        .map_err(|e| OrbitError::Store(e.to_string()))?;
        write_atomic(&disabled_path, &content)?;
        let active_path = self.job_path(job_id);
        if active_path.exists() {
            fs::remove_file(&active_path).map_err(|e| OrbitError::Io(e.to_string()))?;
        }
        Ok(true)
    }

    pub(crate) fn insert_job_run(
        &self,
        job_id: &str,
        attempt: u32,
        scheduled_at: DateTime<Utc>,
    ) -> Result<JobRun, OrbitError> {
        let run = JobRun {
            run_id: self.next_run_id(job_id),
            job_id: job_id.to_string(),
            attempt,
            state: JobRunState::Pending,
            scheduled_at,
            started_at: None,
            finished_at: None,
            duration_ms: None,
            created_at: Utc::now(),
            steps: vec![],
        };
        self.write_run(job_id, &run)?;
        Ok(run)
    }

    pub(crate) fn mark_job_run_running(
        &self,
        run_id: &str,
        started_at: DateTime<Utc>,
    ) -> Result<bool, OrbitError> {
        let Some((job_id, run_dir)) = self.find_run_path(run_id)? else {
            return Ok(false);
        };
        let mut run = self.read_run_at(&run_dir)?;
        run.state = JobRunState::Running;
        run.started_at = Some(started_at);
        self.write_run(&job_id, &run)?;
        Ok(true)
    }

    pub(crate) fn complete_job_run_step(
        &self,
        run_id: &str,
        params: &JobRunStepParams,
    ) -> Result<bool, OrbitError> {
        let Some((job_id, _run_dir)) = self.find_run_path(run_id)? else {
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
        self.write_run_step(&job_id, run_id, params.step_index, &params.target_id, &step)?;
        Ok(true)
    }

    pub(crate) fn finalize_job_run(
        &self,
        run_id: &str,
        state: JobRunState,
        finished_at: DateTime<Utc>,
        duration_ms: Option<u64>,
    ) -> Result<bool, OrbitError> {
        let Some((job_id, run_dir)) = self.find_run_path(run_id)? else {
            return Ok(false);
        };
        let mut run = self.read_run_at(&run_dir)?;
        run.state = state;
        run.finished_at = Some(finished_at);
        run.duration_ms = duration_ms;
        self.write_run(&job_id, &run)?;
        Ok(true)
    }

    fn read_all_activities(&self) -> Result<Vec<Job>, OrbitError> {
        self.ensure_layout()?;
        let mut paths: Vec<PathBuf> = fs::read_dir(self.activities_dir())
            .map_err(|e| OrbitError::Io(e.to_string()))?
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| is_yaml(path))
            .collect();
        // Also include disabled jobs.
        if self.disabled_jobs_dir().exists() {
            let disabled: Vec<PathBuf> = fs::read_dir(self.disabled_jobs_dir())
                .map_err(|e| OrbitError::Io(e.to_string()))?
                .filter_map(Result::ok)
                .map(|entry| entry.path())
                .filter(|path| is_yaml(path))
                .collect();
            paths.extend(disabled);
        }
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
        let mut run_dirs: Vec<PathBuf> = fs::read_dir(&dir)
            .map_err(|e| OrbitError::Io(e.to_string()))?
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|p| p.is_dir())
            .collect();
        run_dirs.sort();
        let mut runs = Vec::new();
        for run_dir in run_dirs {
            runs.push(self.read_run_at(&run_dir)?);
        }
        Ok(runs)
    }

    fn read_all_runs(&self) -> Result<Vec<JobRun>, OrbitError> {
        self.ensure_layout()?;
        let runs_root = self.runs_dir();
        if !runs_root.exists() {
            return Ok(Vec::new());
        }

        let mut runs = Vec::new();
        for entry in fs::read_dir(runs_root).map_err(|e| OrbitError::Io(e.to_string()))? {
            let entry = entry.map_err(|e| OrbitError::Io(e.to_string()))?;
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            if path.file_name().and_then(|value| value.to_str()) == Some("archived") {
                continue;
            }
            let Some(job_id) = path.file_name().and_then(|value| value.to_str()) else {
                continue;
            };
            runs.extend(self.read_runs_for_activity(job_id)?);
        }

        Ok(runs)
    }

    /// Returns `(job_id, run_bundle_dir)` for an active run.
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
            if job_id == "archived" {
                continue;
            }
            let run_dir = path.join(run_id);
            if run_dir.is_dir() {
                return Ok(Some((job_id.to_string(), run_dir)));
            }
        }
        Ok(None)
    }

    /// Returns `(job_id, run_bundle_dir)` for an archived run.
    fn find_archived_run_path(
        &self,
        run_id: &str,
    ) -> Result<Option<(String, PathBuf)>, OrbitError> {
        let runs_root = self.archived_runs_dir();
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
            let run_dir = path.join(run_id);
            if run_dir.is_dir() {
                return Ok(Some((job_id.to_string(), run_dir)));
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

    /// Read a run bundle directory: parses `jrun.yaml` then populates the
    /// convenience fields (`exit_code`, `agent_response_json`, etc.) from
    /// any step files found in `steps/`.
    fn read_run_at(&self, run_dir: &Path) -> Result<JobRun, OrbitError> {
        let jrun_path = run_dir.join("jrun.yaml");
        let raw = fs::read_to_string(&jrun_path).map_err(|e| OrbitError::Io(e.to_string()))?;
        let doc = serde_yaml::from_str::<JobRunFileDocument>(&raw).map_err(|e| {
            OrbitError::Store(format!("invalid jrun.yaml '{}': {e}", jrun_path.display()))
        })?;
        let mut run = doc.run;

        // Read step files and populate in-memory convenience fields.
        let steps_dir = run_dir.join("steps");
        if steps_dir.exists() {
            let mut step_files: Vec<PathBuf> = fs::read_dir(&steps_dir)
                .map_err(|e| OrbitError::Io(e.to_string()))?
                .filter_map(Result::ok)
                .map(|e| e.path())
                .filter(|p| is_yaml(p))
                .collect();
            step_files.sort(); // lexicographic = index order (01-, 02-, …)
            for step_path in &step_files {
                let step_raw =
                    fs::read_to_string(step_path).map_err(|e| OrbitError::Io(e.to_string()))?;
                let step_doc =
                    serde_yaml::from_str::<JobRunStepFileDocument>(&step_raw).map_err(|e| {
                        OrbitError::Store(format!(
                            "invalid step file '{}': {e}",
                            step_path.display()
                        ))
                    })?;
                run.steps.push(step_doc.step);
            }
        }

        Ok(run)
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

    /// Write the run-level `jrun.yaml` inside the run bundle directory.
    fn write_run(&self, job_id: &str, run: &JobRun) -> Result<(), OrbitError> {
        self.ensure_layout()?;
        let run_dir = self.run_bundle_dir(job_id, &run.run_id);
        fs::create_dir_all(&run_dir).map_err(|e| OrbitError::Io(e.to_string()))?;
        let doc = JobRunFileDocument {
            schema_version: 1,
            run: run.clone(),
        };
        let content = serde_yaml::to_string(&doc).map_err(|e| OrbitError::Store(e.to_string()))?;
        write_atomic(&run_dir.join("jrun.yaml"), &content)
    }

    /// Write a step result file inside `<run_bundle_dir>/steps/`.
    fn write_run_step(
        &self,
        job_id: &str,
        run_id: &str,
        step_index: usize,
        target_id: &str,
        step: &JobRunStep,
    ) -> Result<(), OrbitError> {
        let steps_dir = self.run_bundle_dir(job_id, run_id).join("steps");
        fs::create_dir_all(&steps_dir).map_err(|e| OrbitError::Io(e.to_string()))?;
        // Index-prefixed filename preserves order and avoids collisions.
        let filename = format!("{:02}-{target_id}.yaml", step_index + 1);
        let doc = JobRunStepFileDocument {
            schema_version: 1,
            step: step.clone(),
        };
        let content = serde_yaml::to_string(&doc).map_err(|e| OrbitError::Store(e.to_string()))?;
        write_atomic(&steps_dir.join(filename), &content)
    }

    fn next_job_id(&self) -> String {
        let now = Utc::now();
        let base = format!("job-{}", now.format("%Y%m%d-%H%M%S"));
        if !self.job_path(&base).exists() && !self.disabled_job_path(&base).exists() {
            return base;
        }
        for suffix in 2..1024_u32 {
            let candidate = format!("{base}-{suffix}");
            if !self.job_path(&candidate).exists() && !self.disabled_job_path(&candidate).exists() {
                return candidate;
            }
        }
        base
    }

    fn next_run_id(&self, job_id: &str) -> String {
        let now = Utc::now();
        let base = format!("jrun-{}", now.format("%Y%m%d-%H%M%S"));
        if !self.run_id_exists_globally(job_id, &base) {
            return base;
        }
        for suffix in 2..1024_u32 {
            let candidate = format!("{base}-{suffix}");
            if !self.run_id_exists_globally(job_id, &candidate) {
                return candidate;
            }
        }
        base
    }

    fn run_id_exists_globally(&self, job_id: &str, run_id: &str) -> bool {
        self.run_bundle_dir(job_id, run_id).exists()
            || self.archived_run_bundle_dir(job_id, run_id).exists()
            || self.find_run_path(run_id).ok().flatten().is_some()
            || self.find_archived_run_path(run_id).ok().flatten().is_some()
    }

    fn activities_dir(&self) -> PathBuf {
        self.root.join("jobs")
    }

    fn disabled_jobs_dir(&self) -> PathBuf {
        self.activities_dir().join("disabled")
    }

    fn runs_dir(&self) -> PathBuf {
        self.root.join("runs")
    }

    fn job_path(&self, job_id: &str) -> PathBuf {
        self.activities_dir().join(format!("{job_id}.yaml"))
    }

    fn disabled_job_path(&self, job_id: &str) -> PathBuf {
        self.disabled_jobs_dir().join(format!("{job_id}.yaml"))
    }

    fn run_dir(&self, job_id: &str) -> PathBuf {
        self.runs_dir().join(job_id)
    }

    /// Path to the run bundle directory: `<runs_dir>/<job_id>/<run_id>/`
    fn run_bundle_dir(&self, job_id: &str, run_id: &str) -> PathBuf {
        self.run_dir(job_id).join(run_id)
    }

    fn archived_runs_dir(&self) -> PathBuf {
        self.runs_dir().join("archived")
    }

    fn archived_run_dir(&self, job_id: &str) -> PathBuf {
        self.archived_runs_dir().join(job_id)
    }

    /// Path to the archived run bundle directory: `<archived_runs_dir>/<job_id>/<run_id>/`
    fn archived_run_bundle_dir(&self, job_id: &str, run_id: &str) -> PathBuf {
        self.archived_run_dir(job_id).join(run_id)
    }

    pub(crate) fn archive_run(&self, run_id: &str) -> Result<String, OrbitError> {
        let Some((job_id, src)) = self.find_run_path(run_id)? else {
            return Err(OrbitError::JobRunNotFound(run_id.to_string()));
        };
        let dst = self.archived_run_bundle_dir(&job_id, run_id);
        let parent = dst.parent().ok_or_else(|| {
            OrbitError::Io(format!("cannot determine parent for '{}'", dst.display()))
        })?;
        fs::create_dir_all(parent).map_err(|e| OrbitError::Io(e.to_string()))?;
        fs::rename(&src, &dst).map_err(|e| OrbitError::Io(e.to_string()))?;
        Ok(job_id)
    }

    pub(crate) fn delete_run(&self, run_id: &str) -> Result<String, OrbitError> {
        if let Some((job_id, dir)) = self.find_run_path(run_id)? {
            fs::remove_dir_all(&dir).map_err(|e| OrbitError::Io(e.to_string()))?;
            return Ok(job_id);
        }
        if let Some((job_id, dir)) = self.find_archived_run_path(run_id)? {
            fs::remove_dir_all(&dir).map_err(|e| OrbitError::Io(e.to_string()))?;
            return Ok(job_id);
        }
        Err(OrbitError::JobRunNotFound(run_id.to_string()))
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

#[cfg(test)]
mod tests {
    use chrono::Utc;
    use orbit_types::{JobRunState, JobScheduleState, JobStep, JobTargetType, OrbitError};

    use super::JobFileStore;

    fn make_store() -> (tempfile::TempDir, JobFileStore) {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = JobFileStore::new(dir.path().to_path_buf());
        (dir, store)
    }

    fn make_step(target_id: &str) -> JobStep {
        JobStep {
            target_type: JobTargetType::Activity,
            target_id: target_id.to_string(),
            agent_cli: "mock-agent".to_string(),
            timeout_seconds: 300,
            env_extra: vec![],
        }
    }

    fn insert_test_job(store: &JobFileStore, target_id: &str) -> orbit_types::Job {
        store
            .insert_activity_v2(None, vec![make_step(target_id)], JobScheduleState::Enabled)
            .expect("insert job")
    }

    #[test]
    fn archive_run_moves_dir_to_archived_dir() {
        let (_dir, store) = make_store();
        let job = insert_test_job(&store, "target-1");
        let run = store
            .insert_job_run(&job.job_id, 1, Utc::now())
            .expect("insert run");

        let src = store.run_bundle_dir(&job.job_id, &run.run_id);
        assert!(src.exists(), "run dir must exist before archive");

        store.archive_run(&run.run_id).expect("archive run");

        assert!(!src.exists(), "run dir must be gone after archive");
        let dst = store.archived_run_bundle_dir(&job.job_id, &run.run_id);
        assert!(dst.exists(), "archived run dir must exist");
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

    #[test]
    fn delete_run_removes_active_and_archived_dirs() {
        let (_dir, store) = make_store();
        let now = Utc::now();
        let job = insert_test_job(&store, "target-delete");

        let active_run = store
            .insert_job_run(&job.job_id, 1, now)
            .expect("insert active run");
        let archived_run = store
            .insert_job_run(&job.job_id, 2, now)
            .expect("insert archived run");
        store
            .archive_run(&archived_run.run_id)
            .expect("archive run");

        store
            .delete_run(&active_run.run_id)
            .expect("delete active run");
        assert!(
            !store
                .run_bundle_dir(&job.job_id, &active_run.run_id)
                .exists(),
            "active run dir removed"
        );

        store
            .delete_run(&archived_run.run_id)
            .expect("delete archived run");
        assert!(
            !store
                .archived_run_bundle_dir(&job.job_id, &archived_run.run_id)
                .exists(),
            "archived run dir removed"
        );
    }

    #[test]
    fn job_id_uses_datetime_format_without_nanosecond_suffix() {
        let (_dir, store) = make_store();
        let job = insert_test_job(&store, "target-id-format");

        // Must be job-<YYYYMMDD>-<HHMMSS> with no nanosecond component.
        assert!(job.job_id.starts_with("job-"), "must start with 'job-'");
        let rest = &job.job_id["job-".len()..];
        let (date, time) = rest.split_once('-').expect("has dash after prefix");
        assert_eq!(date.len(), 8, "date part must be 8 digits, got '{date}'");
        assert!(
            date.chars().all(|c| c.is_ascii_digit()),
            "date must be digits"
        );
        assert_eq!(time.len(), 6, "time part must be 6 digits, got '{time}'");
        assert!(
            time.chars().all(|c| c.is_ascii_digit()),
            "time must be digits"
        );
    }

    #[test]
    fn job_run_id_uses_datetime_format_without_nanosecond_suffix() {
        let (_dir, store) = make_store();
        let job = insert_test_job(&store, "target-run-id-format");
        let run = store
            .insert_job_run(&job.job_id, 1, Utc::now())
            .expect("insert run");

        assert!(run.run_id.starts_with("jrun-"), "must start with 'jrun-'");
        let rest = &run.run_id["jrun-".len()..];
        let (date, time) = rest.split_once('-').expect("has dash after prefix");
        assert_eq!(date.len(), 8, "date part must be 8 digits, got '{date}'");
        assert!(
            date.chars().all(|c| c.is_ascii_digit()),
            "date must be digits"
        );
        assert_eq!(time.len(), 6, "time part must be 6 digits, got '{time}'");
        assert!(
            time.chars().all(|c| c.is_ascii_digit()),
            "time must be digits"
        );
    }

    #[test]
    fn job_run_ids_are_unique_across_jobs_in_same_second() {
        let (_dir, store) = make_store();
        let job_a = insert_test_job(&store, "target-run-a");
        let job_b = insert_test_job(&store, "target-run-b");

        let current_second = Utc::now().timestamp();
        while Utc::now().timestamp() == current_second {
            std::thread::sleep(std::time::Duration::from_millis(1));
        }

        let first = store
            .insert_job_run(&job_a.job_id, 1, Utc::now())
            .expect("insert first run");
        let second = store
            .insert_job_run(&job_b.job_id, 1, Utc::now())
            .expect("insert second run");

        assert_ne!(
            first.run_id, second.run_id,
            "run ids must be globally unique"
        );

        let resolved = store
            .get_job_run(&second.run_id)
            .expect("lookup run")
            .expect("run exists");
        assert_eq!(resolved.job_id, job_b.job_id);
    }

    #[test]
    fn job_write_read_roundtrip_preserves_all_fields() {
        let (_dir, store) = make_store();
        let step = JobStep {
            target_type: JobTargetType::Activity,
            target_id: "target-roundtrip".to_string(),
            agent_cli: "my-agent-cli".to_string(),
            timeout_seconds: 600,
            env_extra: vec!["MY_VAR".to_string(), "OTHER_VAR".to_string()],
        };
        let written = store
            .insert_activity_v2(
                Some("job-roundtrip-test".to_string()),
                vec![step],
                JobScheduleState::Disabled,
            )
            .expect("insert job");

        // Read the raw YAML to assert correct nesting.
        let yaml_path = store.job_path("job-roundtrip-test");
        let raw = std::fs::read_to_string(&yaml_path).expect("read yaml");
        // steps must be nested under job: (2-space indent)
        assert!(
            raw.contains("  steps:"),
            "steps must be nested under job: but raw yaml was:\n{raw}"
        );
        // state, created_at, updated_at must also be nested
        for field in &["state:", "created_at:", "updated_at:"] {
            assert!(
                raw.contains(&format!("  {field}")),
                "field '{field}' must be nested under job: but raw yaml was:\n{raw}"
            );
        }

        // Read back via the store and assert round-trip fidelity.
        let read_back = store
            .get_job("job-roundtrip-test")
            .expect("get_job ok")
            .expect("job exists");

        assert_eq!(read_back.job_id, written.job_id);
        assert_eq!(read_back.state, JobScheduleState::Disabled);
        assert_eq!(read_back.steps.len(), 1);
        assert_eq!(read_back.steps[0].target_id, "target-roundtrip");
        assert_eq!(read_back.steps[0].agent_cli, "my-agent-cli");
        assert_eq!(read_back.steps[0].timeout_seconds, 600);
        assert_eq!(read_back.steps[0].env_extra, vec!["MY_VAR", "OTHER_VAR"]);
        assert_eq!(
            read_back.created_at.timestamp(),
            written.created_at.timestamp()
        );
        assert_eq!(
            read_back.updated_at.timestamp(),
            written.updated_at.timestamp()
        );
    }

    #[test]
    fn run_bundle_directory_structure_and_step_roundtrip() {
        use crate::backend::JobRunStepParams;
        use orbit_types::JobTargetType;

        let (_dir, store) = make_store();
        let job = insert_test_job(&store, "target-bundle");
        let run = store
            .insert_job_run(&job.job_id, 1, Utc::now())
            .expect("insert run");

        // jrun.yaml must exist inside the run directory
        let run_dir = store.run_bundle_dir(&job.job_id, &run.run_id);
        assert!(run_dir.is_dir(), "run bundle dir must be a directory");
        assert!(run_dir.join("jrun.yaml").exists(), "jrun.yaml must exist");

        // Write a step file
        let step_params = JobRunStepParams {
            step_index: 0,
            target_type: JobTargetType::Activity,
            target_id: "target-bundle".to_string(),
            started_at: Utc::now(),
            finished_at: Utc::now(),
            duration_ms: Some(42),
            exit_code: Some(0),
            agent_response_json: Some(serde_json::json!({"status": "success"})),
            state: JobRunState::Success,
            error_code: None,
            error_message: None,
        };
        store
            .complete_job_run_step(&run.run_id, &step_params)
            .expect("write step");

        // Step file must exist in steps/ subdir
        let steps_dir = run_dir.join("steps");
        assert!(steps_dir.is_dir(), "steps dir must exist");
        let step_files: Vec<_> = std::fs::read_dir(&steps_dir)
            .expect("read steps dir")
            .filter_map(|e| e.ok())
            .filter(|e| {
                e.path()
                    .extension()
                    .and_then(|x| x.to_str())
                    .is_some_and(|x| x == "yaml")
            })
            .collect();
        assert_eq!(step_files.len(), 1, "exactly one step file expected");

        // Finalize jrun.yaml
        store
            .finalize_job_run(&run.run_id, JobRunState::Success, Utc::now(), Some(42))
            .expect("finalize run");

        let read = store
            .get_job_run(&run.run_id)
            .expect("get run")
            .expect("run exists");
        assert_eq!(read.state, JobRunState::Success);
        assert_eq!(read.steps.len(), 1);
        assert_eq!(read.steps[0].exit_code, Some(0));
        assert_eq!(read.steps[0].duration_ms, Some(42));
    }
}
