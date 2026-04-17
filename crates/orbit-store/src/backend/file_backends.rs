use chrono::{DateTime, Utc};
use orbit_types::{
    Activity, ExecutorDef, Job, JobRun, JobScheduleState, KnowledgeRunMetrics, OrbitError,
    PipelineState, PolicyDef, Task, TaskArtifact, TaskPriority, TaskStatus,
};

use super::contracts::{
    ActivityCreateParams, ActivityStoreBackend, ActivityUpdateParams, ExecutorDefStoreBackend,
    JobCreateParams, JobDefinitionStoreBackend, JobRunQuery, JobRunStepParams, JobRunStoreBackend,
    JobUpdateParams, PolicyDefStoreBackend, TaskArtifactStoreBackend, TaskArtifactUpdateParams,
    TaskCreateParams, TaskDocumentStoreBackend, TaskDocumentUpdateParams, TaskHistoryStoreBackend,
    TaskHistoryUpdateParams, TaskReviewStoreBackend, TaskReviewUpdateParams, TaskStoreBackend,
};
use crate::file::activity_store::ActivityFileStore;
use crate::file::executor_def_store::ExecutorDefFileStore;
use crate::file::job_store::JobFileStore;
use crate::file::policy_def_store::PolicyDefFileStore;
use crate::file::task_store::TaskFileStore;

impl TaskStoreBackend for TaskFileStore {
    fn create_task(&self, params: TaskCreateParams) -> Result<Task, OrbitError> {
        self.create_task(params)
    }

    fn list_tasks(&self) -> Result<Vec<Task>, OrbitError> {
        self.list_tasks()
    }

    fn list_tasks_filtered(
        &self,
        status: Option<TaskStatus>,
        priority: Option<TaskPriority>,
        parent_id: Option<&str>,
        batch_id: Option<&str>,
    ) -> Result<Vec<Task>, OrbitError> {
        self.list_tasks_filtered(status, priority, parent_id, batch_id)
    }

    fn get_task(&self, id: &str) -> Result<Option<Task>, OrbitError> {
        self.get_task(id)
    }

    fn search_tasks(&self, query: &str) -> Result<Vec<Task>, OrbitError> {
        self.search_tasks(query)
    }

    fn delete_task(&self, id: &str) -> Result<bool, OrbitError> {
        self.delete_task(id)
    }
}

impl TaskDocumentStoreBackend for TaskFileStore {
    fn update_task_document(
        &self,
        id: &str,
        params: TaskDocumentUpdateParams,
    ) -> Result<(), OrbitError> {
        self.update_task_document(id, &params)
    }
}

impl TaskHistoryStoreBackend for TaskFileStore {
    fn update_task_history(
        &self,
        id: &str,
        params: TaskHistoryUpdateParams,
    ) -> Result<(), OrbitError> {
        self.update_task_history(id, &params)
    }
}

impl TaskReviewStoreBackend for TaskFileStore {
    fn update_task_reviews(
        &self,
        id: &str,
        params: TaskReviewUpdateParams,
    ) -> Result<(), OrbitError> {
        self.update_task_reviews(id, &params)
    }
}

impl TaskArtifactStoreBackend for TaskFileStore {
    fn get_task_artifacts(&self, id: &str) -> Result<Option<Vec<TaskArtifact>>, OrbitError> {
        self.get_task_artifacts(id)
    }

    fn upsert_task_artifacts(
        &self,
        id: &str,
        params: TaskArtifactUpdateParams,
    ) -> Result<(), OrbitError> {
        self.upsert_task_artifacts(id, &params)
    }
}

impl ActivityStoreBackend for ActivityFileStore {
    fn add_activity(&self, params: ActivityCreateParams) -> Result<Activity, OrbitError> {
        self.insert_work(&params)
    }

    fn list_activities(&self, include_inactive: bool) -> Result<Vec<Activity>, OrbitError> {
        self.list_activities(include_inactive)
    }

    fn get_activity(&self, id: &str) -> Result<Option<Activity>, OrbitError> {
        self.get_activity(id)
    }

    fn update_activity(
        &self,
        id: &str,
        params: ActivityUpdateParams,
    ) -> Result<Activity, OrbitError> {
        self.update_activity(id, &params)
    }

    fn disable_activity(&self, id: &str) -> Result<bool, OrbitError> {
        self.disable_activity(id)
    }
}

impl JobDefinitionStoreBackend for JobFileStore {
    fn add_job(&self, params: JobCreateParams) -> Result<Job, OrbitError> {
        self.add_job(params)
    }

    fn update_job(&self, job_id: &str, params: JobUpdateParams) -> Result<Job, OrbitError> {
        self.update_job(job_id, &params)
    }

    fn list_jobs(&self, include_disabled: bool) -> Result<Vec<Job>, OrbitError> {
        self.list_jobs(include_disabled)
    }

    fn get_job(&self, job_id: &str) -> Result<Option<Job>, OrbitError> {
        self.get_job(job_id)
    }

    fn set_job_state(&self, job_id: &str, state: JobScheduleState) -> Result<bool, OrbitError> {
        self.set_job_state(job_id, state)
    }

    fn mark_job_disabled(&self, job_id: &str) -> Result<bool, OrbitError> {
        self.mark_job_disabled(job_id)
    }
}

impl JobRunStoreBackend for JobFileStore {
    fn list_job_runs(&self, job_id: &str) -> Result<Vec<JobRun>, OrbitError> {
        self.list_job_runs(job_id)
    }

    fn list_job_runs_filtered(&self, query: &JobRunQuery) -> Result<Vec<JobRun>, OrbitError> {
        self.list_job_runs_filtered(query)
    }

    fn get_job_run(&self, run_id: &str) -> Result<Option<JobRun>, OrbitError> {
        self.get_job_run(run_id)
    }

    fn list_pending_or_running_job_runs(&self, job_id: &str) -> Result<Vec<JobRun>, OrbitError> {
        self.list_pending_or_running_job_runs(job_id)
    }

    fn insert_job_run(
        &self,
        job_id: &str,
        attempt: u32,
        scheduled_at: DateTime<Utc>,
        input: Option<serde_json::Value>,
        retry_source_run_id: Option<String>,
    ) -> Result<JobRun, OrbitError> {
        self.insert_job_run(job_id, attempt, scheduled_at, input, retry_source_run_id)
    }

    fn mark_job_run_running(
        &self,
        run_id: &str,
        started_at: DateTime<Utc>,
        pid: u32,
    ) -> Result<bool, OrbitError> {
        self.mark_job_run_running(run_id, started_at, pid)
    }

    fn take_over_running_job_run(
        &self,
        run_id: &str,
        expected_pid: Option<u32>,
        expected_pid_start_time: Option<String>,
        started_at: DateTime<Utc>,
        pid: u32,
    ) -> Result<bool, OrbitError> {
        self.take_over_running_job_run(
            run_id,
            expected_pid,
            expected_pid_start_time,
            started_at,
            pid,
        )
    }

    fn abandon_job_run(
        &self,
        run_id: &str,
        finished_at: DateTime<Utc>,
    ) -> Result<bool, OrbitError> {
        self.abandon_job_run(run_id, finished_at)
    }

    fn complete_job_run_step(
        &self,
        run_id: &str,
        params: &JobRunStepParams,
    ) -> Result<bool, OrbitError> {
        self.complete_job_run_step(run_id, params)
    }

    fn record_job_run_knowledge_metrics(
        &self,
        run_id: &str,
        metrics: KnowledgeRunMetrics,
    ) -> Result<bool, OrbitError> {
        self.record_job_run_knowledge_metrics(run_id, metrics)
    }

    fn finalize_job_run(
        &self,
        run_id: &str,
        state: orbit_types::JobRunState,
        finished_at: chrono::DateTime<chrono::Utc>,
        duration_ms: Option<u64>,
    ) -> Result<bool, OrbitError> {
        self.finalize_job_run(run_id, state, finished_at, duration_ms)
    }

    fn archive_job_run(&self, run_id: &str) -> Result<String, OrbitError> {
        self.archive_run(run_id)
    }

    fn delete_job_run(&self, run_id: &str) -> Result<String, OrbitError> {
        self.delete_run(run_id)
    }

    fn read_run_state(&self, run_id: &str) -> Result<Option<PipelineState>, OrbitError> {
        self.read_run_state(run_id)
    }

    fn write_run_state(&self, run_id: &str, state: &PipelineState) -> Result<(), OrbitError> {
        self.write_run_state(run_id, state)
    }

    fn list_all_pending_or_running_runs(&self) -> Result<Vec<JobRun>, OrbitError> {
        self.list_all_pending_or_running_runs()
    }
}

impl ExecutorDefStoreBackend for ExecutorDefFileStore {
    fn list_executor_defs(&self) -> Result<Vec<ExecutorDef>, OrbitError> {
        self.list_executor_defs()
    }

    fn get_executor_def(&self, name: &str) -> Result<Option<ExecutorDef>, OrbitError> {
        self.get_executor_def(name)
    }

    fn upsert_executor_def(&self, def: &ExecutorDef) -> Result<(), OrbitError> {
        self.upsert_executor_def(def)
    }
}

impl PolicyDefStoreBackend for PolicyDefFileStore {
    fn list_policy_defs(&self) -> Result<Vec<PolicyDef>, OrbitError> {
        self.list_policy_defs()
    }

    fn get_policy_def(&self, name: &str) -> Result<Option<PolicyDef>, OrbitError> {
        self.get_policy_def(name)
    }

    fn upsert_policy_def(&self, def: &PolicyDef) -> Result<(), OrbitError> {
        self.upsert_policy_def(def)
    }
}
