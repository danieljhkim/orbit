use std::fs;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use orbit_types::{
    JobRun, JobRunState, JobRunStep, KnowledgeRunMetrics, OrbitError, PipelineState,
};

use crate::backend::JobRunStepParams;
use crate::file::fs_utils::write_atomic;

use super::{
    JobFileStore,
    doc::{JobRunFileDocument, JobRunStepFileDocument},
    resource::process_start_time_token,
};

impl JobFileStore {
    pub(crate) fn insert_job_run(
        &self,
        job_id: &str,
        attempt: u32,
        scheduled_at: DateTime<Utc>,
        input: Option<serde_json::Value>,
        retry_source_run_id: Option<String>,
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
            pid: None,
            pid_start_time: None,
            input,
            retry_source_run_id,
            created_at: Utc::now(),
            steps: vec![],
            knowledge_metrics: None,
        };
        self.write_run(job_id, &run)?;
        Ok(run)
    }

    pub(crate) fn mark_job_run_running(
        &self,
        run_id: &str,
        started_at: DateTime<Utc>,
        pid: u32,
    ) -> Result<bool, OrbitError> {
        let Some((job_id, run_dir)) = self.find_run_path(run_id)? else {
            return Ok(false);
        };
        let mut run = self.read_run_at(&run_dir)?;
        run.state = run
            .state
            .try_transition(orbit_types::RunEvent::Start)
            .map_err(OrbitError::JobRunStateTransition)?;
        run.started_at = Some(started_at);
        run.pid = Some(pid);
        run.pid_start_time = process_start_time_token(pid);
        self.write_run(&job_id, &run)?;
        Ok(true)
    }

    pub(crate) fn take_over_running_job_run(
        &self,
        run_id: &str,
        expected_pid: Option<u32>,
        expected_pid_start_time: Option<String>,
        started_at: DateTime<Utc>,
        pid: u32,
    ) -> Result<bool, OrbitError> {
        let Some((job_id, run_dir)) = self.find_run_path(run_id)? else {
            return Ok(false);
        };
        let mut run = self.read_run_at(&run_dir)?;
        if run.state != JobRunState::Running {
            return Ok(false);
        }
        if run.pid != expected_pid || run.pid_start_time != expected_pid_start_time {
            return Ok(false);
        }
        run.started_at = run.started_at.or(Some(started_at));
        run.pid = Some(pid);
        run.pid_start_time = process_start_time_token(pid);
        self.write_run(&job_id, &run)?;
        Ok(true)
    }

    pub(crate) fn abandon_job_run(
        &self,
        run_id: &str,
        finished_at: DateTime<Utc>,
    ) -> Result<bool, OrbitError> {
        let Some((job_id, run_dir)) = self.find_run_path(run_id)? else {
            return Ok(false);
        };
        let mut run = self.read_run_at(&run_dir)?;
        if run.state.is_terminal() {
            return Ok(true);
        }
        run.state = run
            .state
            .try_transition(orbit_types::RunEvent::Abandon)
            .map_err(OrbitError::JobRunStateTransition)?;
        run.finished_at = Some(finished_at);
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
        params
            .state
            .validate_step_state()
            .map_err(OrbitError::JobRunStateTransition)?;
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

    pub(crate) fn record_job_run_knowledge_metrics(
        &self,
        run_id: &str,
        metrics: KnowledgeRunMetrics,
    ) -> Result<bool, OrbitError> {
        let Some((job_id, run_dir)) = self.find_run_path(run_id)? else {
            return Ok(false);
        };
        let mut run = self.read_run_at(&run_dir)?;
        run.knowledge_metrics = Some(metrics);
        self.write_run(&job_id, &run)?;
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
        // Preserve existing no-op behavior for terminal states.
        if run.state.is_terminal() {
            return Ok(true);
        }
        let event = match state {
            JobRunState::Success => orbit_types::RunEvent::Complete,
            JobRunState::Failed => orbit_types::RunEvent::Fail,
            JobRunState::Timeout => orbit_types::RunEvent::Timeout,
            JobRunState::Cancelled => orbit_types::RunEvent::Cancel,
            other => {
                return Err(OrbitError::JobRunStateTransition(format!(
                    "cannot finalize to non-terminal state: {}",
                    other
                )));
            }
        };
        run.state = run
            .state
            .try_transition(event)
            .map_err(OrbitError::JobRunStateTransition)?;
        run.finished_at = Some(finished_at);
        run.duration_ms = duration_ms;
        self.write_run(&job_id, &run)?;
        Ok(true)
    }

    pub(crate) fn read_runs_for_activity(&self, job_id: &str) -> Result<Vec<JobRun>, OrbitError> {
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

    pub(crate) fn read_all_runs(&self) -> Result<Vec<JobRun>, OrbitError> {
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
    pub(crate) fn find_run_path(
        &self,
        run_id: &str,
    ) -> Result<Option<(String, PathBuf)>, OrbitError> {
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
    pub(crate) fn find_archived_run_path(
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

    /// Read a run bundle directory: parses `jrun.yaml` then populates the
    /// convenience fields (`exit_code`, `agent_response_json`, etc.) from
    /// any step files found in `steps/`.
    pub(crate) fn read_run_at(&self, run_dir: &Path) -> Result<JobRun, OrbitError> {
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

    /// Write the run-level `jrun.yaml` inside the run bundle directory.
    pub(crate) fn write_run(&self, job_id: &str, run: &JobRun) -> Result<(), OrbitError> {
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
    pub(crate) fn write_run_step(
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

    pub(crate) fn read_run_state(&self, run_id: &str) -> Result<Option<PipelineState>, OrbitError> {
        let Some((_job_id, run_dir)) = self.find_run_path(run_id)? else {
            return Ok(None);
        };
        let state_path = run_dir.join("state.json");
        if !state_path.exists() {
            return Ok(None);
        }
        let raw = fs::read_to_string(&state_path).map_err(|e| OrbitError::Io(e.to_string()))?;
        let state: PipelineState = serde_json::from_str(&raw).map_err(|e| {
            OrbitError::Store(format!(
                "invalid state.json '{}': {e}",
                state_path.display()
            ))
        })?;
        Ok(Some(state))
    }

    pub(crate) fn write_run_state(
        &self,
        run_id: &str,
        state: &PipelineState,
    ) -> Result<(), OrbitError> {
        let Some((_job_id, run_dir)) = self.find_run_path(run_id)? else {
            return Err(OrbitError::JobRunNotFound(run_id.to_string()));
        };
        let content =
            serde_json::to_string_pretty(state).map_err(|e| OrbitError::Store(e.to_string()))?;
        write_atomic(&run_dir.join("state.json"), &content)
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

fn is_yaml(path: &Path) -> bool {
    path.extension()
        .and_then(|value| value.to_str())
        .is_some_and(|ext| ext.eq_ignore_ascii_case("yaml") || ext.eq_ignore_ascii_case("yml"))
}
