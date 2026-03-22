use std::sync::Arc;

use chrono::{DateTime, Utc};
use orbit_types::{Job, JobRun, JobRunState, JobScheduleState, OrbitError};

use super::contracts::{
    JobCreateParams, JobRunQuery, JobRunStepParams, JobStoreBackend, JobUpdateParams,
};

/// A layered job store that merges a workspace store with a global store.
///
/// Read semantics: workspace entries shadow global entries by job ID.
/// Write semantics: writes go to workspace store if present, otherwise global.
/// Mutations target whichever store owns the entry.
pub struct LayeredJobStore {
    workspace: Arc<dyn JobStoreBackend>,
    global: Arc<dyn JobStoreBackend>,
}

impl LayeredJobStore {
    pub fn new(workspace: Arc<dyn JobStoreBackend>, global: Arc<dyn JobStoreBackend>) -> Self {
        Self { workspace, global }
    }

    /// Returns (store_that_owns_it, is_workspace) for mutation routing.
    fn owning_store(&self, job_id: &str) -> Result<&dyn JobStoreBackend, OrbitError> {
        if self.workspace.get_job(job_id)?.is_some() {
            Ok(self.workspace.as_ref())
        } else {
            Ok(self.global.as_ref())
        }
    }
}

impl JobStoreBackend for LayeredJobStore {
    fn add_job(&self, params: JobCreateParams) -> Result<Job, OrbitError> {
        self.workspace.add_job(params)
    }

    fn update_job(&self, job_id: &str, params: JobUpdateParams) -> Result<Job, OrbitError> {
        self.owning_store(job_id)?.update_job(job_id, params)
    }

    fn list_jobs(&self, include_disabled: bool) -> Result<Vec<Job>, OrbitError> {
        let workspace_jobs = self.workspace.list_jobs(include_disabled)?;
        let global_jobs = self.global.list_jobs(include_disabled)?;

        let workspace_ids: std::collections::HashSet<String> =
            workspace_jobs.iter().map(|j| j.job_id.clone()).collect();

        let mut merged = workspace_jobs;
        for job in global_jobs {
            if !workspace_ids.contains(&job.job_id) {
                merged.push(job);
            }
        }
        merged.sort_by(|a, b| a.job_id.cmp(&b.job_id));
        Ok(merged)
    }

    fn get_job(&self, job_id: &str) -> Result<Option<Job>, OrbitError> {
        if let Some(job) = self.workspace.get_job(job_id)? {
            return Ok(Some(job));
        }
        self.global.get_job(job_id)
    }

    fn list_job_runs(&self, job_id: &str) -> Result<Vec<JobRun>, OrbitError> {
        self.workspace.list_job_runs(job_id)
    }

    fn list_job_runs_filtered(&self, query: &JobRunQuery) -> Result<Vec<JobRun>, OrbitError> {
        self.workspace.list_job_runs_filtered(query)
    }

    fn get_job_run(&self, run_id: &str) -> Result<Option<JobRun>, OrbitError> {
        self.workspace.get_job_run(run_id)
    }

    fn list_pending_or_running_job_runs(&self, job_id: &str) -> Result<Vec<JobRun>, OrbitError> {
        self.workspace.list_pending_or_running_job_runs(job_id)
    }

    fn set_job_state(&self, job_id: &str, state: JobScheduleState) -> Result<bool, OrbitError> {
        self.owning_store(job_id)?.set_job_state(job_id, state)
    }

    fn mark_job_disabled(&self, job_id: &str) -> Result<bool, OrbitError> {
        self.owning_store(job_id)?.mark_job_disabled(job_id)
    }

    fn insert_job_run(
        &self,
        job_id: &str,
        attempt: u32,
        scheduled_at: DateTime<Utc>,
    ) -> Result<JobRun, OrbitError> {
        // Runs are always created in the workspace store, regardless of
        // where the job definition lives.
        self.workspace.insert_job_run(job_id, attempt, scheduled_at)
    }

    fn mark_job_run_running(
        &self,
        run_id: &str,
        started_at: DateTime<Utc>,
        pid: u32,
    ) -> Result<bool, OrbitError> {
        self.workspace.mark_job_run_running(run_id, started_at, pid)
    }

    fn abandon_job_run(
        &self,
        run_id: &str,
        finished_at: DateTime<Utc>,
    ) -> Result<bool, OrbitError> {
        self.workspace.abandon_job_run(run_id, finished_at)
    }

    fn complete_job_run_step(
        &self,
        run_id: &str,
        params: &JobRunStepParams,
    ) -> Result<bool, OrbitError> {
        self.workspace.complete_job_run_step(run_id, params)
    }

    fn finalize_job_run(
        &self,
        run_id: &str,
        state: JobRunState,
        finished_at: DateTime<Utc>,
        duration_ms: Option<u64>,
    ) -> Result<bool, OrbitError> {
        self.workspace
            .finalize_job_run(run_id, state, finished_at, duration_ms)
    }

    fn archive_job_run(&self, run_id: &str) -> Result<String, OrbitError> {
        self.workspace.archive_job_run(run_id)
    }

    fn delete_job_run(&self, run_id: &str) -> Result<String, OrbitError> {
        self.workspace.delete_job_run(run_id)
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};

    use super::*;
    use crate::backend::job_store_file;
    use orbit_types::JobStep;

    static COUNTER: AtomicU64 = AtomicU64::new(0);

    struct TempDir(PathBuf);

    impl TempDir {
        fn new(prefix: &str) -> Self {
            let n = COUNTER.fetch_add(1, Ordering::Relaxed);
            let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .parent()
                .unwrap()
                .join("tmp")
                .join(format!("{prefix}-{n}"));
            std::fs::create_dir_all(&dir).unwrap();
            Self(dir)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn make_params(id: &str) -> JobCreateParams {
        JobCreateParams {
            job_id: Some(id.to_string()),
            default_input: None,
            max_active_runs: 1,
            steps: vec![JobStep {
                target_type: orbit_types::JobTargetType::Activity,
                target_id: "test-activity".to_string(),
                ..Default::default()
            }],
            initial_state: JobScheduleState::Enabled,
        }
    }

    fn make_layered() -> (
        Arc<dyn JobStoreBackend>,
        Arc<dyn JobStoreBackend>,
        LayeredJobStore,
        TempDir,
    ) {
        let dir = TempDir::new("layered-job");
        let ws = job_store_file(dir.0.join("ws")).unwrap();
        let global = job_store_file(dir.0.join("global")).unwrap();
        let layered = LayeredJobStore::new(ws.clone(), global.clone());
        (ws, global, layered, dir)
    }

    #[test]
    fn workspace_shadows_global_by_job_id() {
        let (ws, global, layered, _dir) = make_layered();
        global.add_job(make_params("shared")).unwrap();
        ws.add_job(make_params("shared")).unwrap();

        let jobs = layered.list_jobs(true).unwrap();
        assert_eq!(jobs.len(), 1);
        assert_eq!(jobs[0].job_id, "shared");
    }

    #[test]
    fn global_only_jobs_visible() {
        let (_ws, global, layered, _dir) = make_layered();
        global.add_job(make_params("global-only")).unwrap();

        assert!(layered.get_job("global-only").unwrap().is_some());
        let jobs = layered.list_jobs(true).unwrap();
        assert_eq!(jobs.len(), 1);
    }

    #[test]
    fn merge_returns_union() {
        let (ws, global, layered, _dir) = make_layered();
        global.add_job(make_params("g1")).unwrap();
        global.add_job(make_params("g2")).unwrap();
        ws.add_job(make_params("w1")).unwrap();
        ws.add_job(make_params("g2")).unwrap(); // shadows global g2

        let jobs = layered.list_jobs(true).unwrap();
        assert_eq!(jobs.len(), 3);
    }

    #[test]
    fn writes_go_to_workspace() {
        let (ws, global, layered, _dir) = make_layered();
        layered.add_job(make_params("new")).unwrap();

        assert!(ws.get_job("new").unwrap().is_some());
        assert!(global.get_job("new").unwrap().is_none());
    }

    #[test]
    fn update_targets_owning_store() {
        let (_ws, global, layered, _dir) = make_layered();
        global.add_job(make_params("gj")).unwrap();

        let update = JobUpdateParams {
            max_active_runs: Some(5),
            ..Default::default()
        };
        let updated = layered.update_job("gj", update).unwrap();
        assert_eq!(updated.max_active_runs, 5);
    }

    #[test]
    fn job_runs_are_workspace_scoped_only() {
        // A run created via a global-only job must land in workspace,
        // and must NOT be visible when querying the global store directly.
        let (_ws, global, layered, _dir) = make_layered();
        global.add_job(make_params("global-job")).unwrap();

        let now = chrono::Utc::now();
        let run = layered
            .insert_job_run("global-job", 1, now)
            .expect("insert run");

        // Visible via layered (workspace-scoped).
        let runs = layered.list_job_runs("global-job").unwrap();
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].run_id, run.run_id);

        // NOT visible directly from global store.
        let global_runs = global.list_job_runs("global-job").unwrap();
        assert!(
            global_runs.is_empty(),
            "runs must not leak into global store"
        );

        // get_job_run must resolve from workspace only.
        assert!(layered.get_job_run(&run.run_id).unwrap().is_some());
        assert!(
            global.get_job_run(&run.run_id).unwrap().is_none(),
            "get_job_run must not cross into global store"
        );
    }
}
