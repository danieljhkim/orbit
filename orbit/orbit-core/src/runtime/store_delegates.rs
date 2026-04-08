use orbit_store::{
    ActivityCreateParams, ActivityUpdateParams, AuditEventFilter, AuditEventInsertParams,
    JobCreateParams, JobRunQuery, JobRunStepParams, JobUpdateParams, TaskCreateParams,
    TaskUpdateParams as StoreTaskUpdateParams,
};
use orbit_types::{
    Activity, AuditEvent, Job, JobRun, JobRunState, OrbitError, StoredTool, Task, TaskPriority,
    TaskStatus,
};

use super::OrbitRuntime;

impl OrbitRuntime {
    // ---- Task store ----

    pub(crate) fn create_task_record(&self, params: TaskCreateParams) -> Result<Task, OrbitError> {
        self.context.task_store().create_task(params)
    }

    pub(crate) fn get_task_record(&self, id: &str) -> Result<Option<Task>, OrbitError> {
        self.context.task_store().get_task(id)
    }

    pub(crate) fn list_task_records(&self) -> Result<Vec<Task>, OrbitError> {
        self.context.task_store().list_tasks()
    }

    pub(crate) fn list_task_records_filtered(
        &self,
        status: Option<TaskStatus>,
        priority: Option<TaskPriority>,
        parent_id: Option<&str>,
        batch_id: Option<&str>,
    ) -> Result<Vec<Task>, OrbitError> {
        self.context
            .task_store()
            .list_tasks_filtered(status, priority, parent_id, batch_id)
    }

    pub(crate) fn update_task_record(
        &self,
        id: &str,
        params: StoreTaskUpdateParams,
    ) -> Result<Task, OrbitError> {
        self.context.task_store().update_task(id, params)
    }

    pub(crate) fn delete_task_record(&self, id: &str) -> Result<bool, OrbitError> {
        self.context.task_store().delete_task(id)
    }

    pub(crate) fn search_task_records(&self, query: &str) -> Result<Vec<Task>, OrbitError> {
        self.context.task_store().search_tasks(query)
    }

    // ---- Activity store ----

    pub(crate) fn add_activity_record(
        &self,
        params: ActivityCreateParams,
    ) -> Result<Activity, OrbitError> {
        self.context.activity_store().add_activity(params)
    }

    pub(crate) fn list_activity_records(
        &self,
        include_inactive: bool,
    ) -> Result<Vec<Activity>, OrbitError> {
        self.context
            .activity_store()
            .list_activities(include_inactive)
    }

    pub(crate) fn get_activity_record(&self, id: &str) -> Result<Option<Activity>, OrbitError> {
        self.context.activity_store().get_activity(id)
    }

    pub(crate) fn update_activity_record(
        &self,
        id: &str,
        params: ActivityUpdateParams,
    ) -> Result<Activity, OrbitError> {
        self.context.activity_store().update_activity(id, params)
    }

    pub(crate) fn disable_activity_record(&self, id: &str) -> Result<bool, OrbitError> {
        self.context.activity_store().disable_activity(id)
    }

    // ---- Job store ----

    pub(crate) fn add_job_record(&self, params: JobCreateParams) -> Result<Job, OrbitError> {
        self.context.job_store().add_job(params)
    }

    pub(crate) fn update_job_record(
        &self,
        job_id: &str,
        params: JobUpdateParams,
    ) -> Result<Job, OrbitError> {
        self.context.job_store().update_job(job_id, params)
    }

    pub(crate) fn mark_job_disabled_record(&self, job_id: &str) -> Result<bool, OrbitError> {
        self.context.job_store().mark_job_disabled(job_id)
    }

    pub(crate) fn list_job_records(&self, include_disabled: bool) -> Result<Vec<Job>, OrbitError> {
        self.context.job_store().list_jobs(include_disabled)
    }

    pub(crate) fn get_job_record(&self, job_id: &str) -> Result<Option<Job>, OrbitError> {
        self.context.job_store().get_job(job_id)
    }

    // ---- Job run store ----

    pub(crate) fn list_job_runs_filtered_record(
        &self,
        query: &JobRunQuery,
    ) -> Result<Vec<JobRun>, OrbitError> {
        self.context.job_store().list_job_runs_filtered(query)
    }

    pub(crate) fn list_pending_or_running_job_runs_record(
        &self,
        job_id: &str,
    ) -> Result<Vec<JobRun>, OrbitError> {
        self.context
            .job_store()
            .list_pending_or_running_job_runs(job_id)
    }

    pub(crate) fn insert_job_run_record(
        &self,
        job_id: &str,
        attempt: u32,
        scheduled_at: chrono::DateTime<chrono::Utc>,
        input: Option<serde_json::Value>,
        retry_source_run_id: Option<String>,
    ) -> Result<JobRun, OrbitError> {
        self.context.job_store().insert_job_run(
            job_id,
            attempt,
            scheduled_at,
            input,
            retry_source_run_id,
        )
    }

    pub(crate) fn mark_job_run_running_record(
        &self,
        run_id: &str,
        started_at: chrono::DateTime<chrono::Utc>,
        pid: u32,
    ) -> Result<bool, OrbitError> {
        self.context
            .job_store()
            .mark_job_run_running(run_id, started_at, pid)
    }

    pub(crate) fn abandon_job_run_record(
        &self,
        run_id: &str,
        finished_at: chrono::DateTime<chrono::Utc>,
    ) -> Result<bool, OrbitError> {
        self.context
            .job_store()
            .abandon_job_run(run_id, finished_at)
    }

    pub(crate) fn complete_job_run_step_record(
        &self,
        run_id: &str,
        params: &JobRunStepParams,
    ) -> Result<bool, OrbitError> {
        self.context
            .job_store()
            .complete_job_run_step(run_id, params)
    }

    pub(crate) fn finalize_job_run_record(
        &self,
        run_id: &str,
        state: JobRunState,
        finished_at: chrono::DateTime<chrono::Utc>,
        duration_ms: Option<u64>,
    ) -> Result<bool, OrbitError> {
        self.context
            .job_store()
            .finalize_job_run(run_id, state, finished_at, duration_ms)
    }

    pub(crate) fn get_job_run_record(&self, run_id: &str) -> Result<Option<JobRun>, OrbitError> {
        self.context.job_store().get_job_run(run_id)
    }

    pub(crate) fn list_job_run_records(&self, job_id: &str) -> Result<Vec<JobRun>, OrbitError> {
        self.context.job_store().list_job_runs(job_id)
    }

    pub(crate) fn archive_job_run_record(&self, run_id: &str) -> Result<String, OrbitError> {
        self.context.job_store().archive_job_run(run_id)
    }

    pub(crate) fn delete_job_run_record(&self, run_id: &str) -> Result<String, OrbitError> {
        self.context.job_store().delete_job_run(run_id)
    }

    // ---- Tool store ----

    pub(crate) fn list_tool_records(&self) -> Result<Vec<StoredTool>, OrbitError> {
        self.context.tool_store().list_tools()
    }

    pub(crate) fn get_tool_record(&self, name: &str) -> Result<Option<StoredTool>, OrbitError> {
        self.context.tool_store().get_tool(name)
    }

    pub(crate) fn insert_tool_record(&self, tool: &StoredTool) -> Result<(), OrbitError> {
        self.context.tool_store().insert_tool(tool)
    }

    pub(crate) fn delete_tool_record(&self, name: &str) -> Result<bool, OrbitError> {
        self.context.tool_store().delete_tool(name)
    }

    pub(crate) fn set_tool_enabled_record(
        &self,
        name: &str,
        enabled: bool,
    ) -> Result<bool, OrbitError> {
        self.context.tool_store().set_tool_enabled(name, enabled)
    }

    // ---- Audit event store ----

    pub(crate) fn list_audit_event_records(
        &self,
        filter: &AuditEventFilter,
    ) -> Result<Vec<AuditEvent>, OrbitError> {
        self.context.audit_event_store().list_audit_events(filter)
    }

    pub(crate) fn get_audit_event_record(&self, id: i64) -> Result<Option<AuditEvent>, OrbitError> {
        self.context.audit_event_store().get_audit_event(id)
    }

    pub(crate) fn prune_audit_event_records(
        &self,
        older_than: &chrono::DateTime<chrono::Utc>,
    ) -> Result<usize, OrbitError> {
        self.context
            .audit_event_store()
            .prune_audit_events(older_than)
    }

    pub(crate) fn audit_event_stats_record(
        &self,
        since: Option<&chrono::DateTime<chrono::Utc>>,
        tool: Option<&str>,
    ) -> Result<(i64, i64, i64, i64, f64, i64), OrbitError> {
        self.context
            .audit_event_store()
            .get_audit_event_stats(since, tool)
    }

    pub(crate) fn audit_event_durations_record(
        &self,
        since: Option<&chrono::DateTime<chrono::Utc>>,
        tool: Option<&str>,
    ) -> Result<Vec<i64>, OrbitError> {
        self.context
            .audit_event_store()
            .get_audit_event_durations(since, tool)
    }

    pub(crate) fn insert_audit_event_record(
        &self,
        params: &AuditEventInsertParams,
    ) -> Result<(), OrbitError> {
        self.context
            .audit_event_store()
            .insert_audit_event_record(params)
    }
}
