use chrono::{DateTime, Utc};
use orbit_types::{
    Job, OrbitError, Scheduler, SchedulerRun, SchedulerRunState, SchedulerScheduleState, Task,
    TaskPriority, TaskStatus,
};
use serde_json::Value;

use super::contracts::{
    JobCreateParams, JobStoreBackend, SchedulerCreateParams, SchedulerStoreBackend,
    TaskCreateParams, TaskStoreBackend, TaskUpdateParams,
};
use crate::file::job_store::{FileWorkInsert, JobFileStore};
use crate::file::scheduler_store::SchedulerFileStore;
use crate::file::task_store::{FileTaskInsert, FileTaskUpdate, TaskFileStore};
use crate::sqlite::scheduler_store::DueJobsClaim;

impl TaskStoreBackend for TaskFileStore {
    fn create_task(&self, params: TaskCreateParams) -> Result<Task, OrbitError> {
        self.create_task(FileTaskInsert {
            title: params.title,
            description: params.description,
            instructions: params.instructions,
            context_files: params.context_files,
            workspace_path: params.workspace_path,
            identity_id: params.identity_id,
            assigned_to: params.assigned_to,
            created_by: params.created_by,
            approved_at: params.approved_at,
            approved_by: params.approved_by,
            approval_note: params.approval_note,
            priority: params.priority,
            task_type: params.task_type,
            owner: params.owner,
            parent_id: params.parent_id,
        })
    }

    fn list_tasks(&self) -> Result<Vec<Task>, OrbitError> {
        self.list_tasks()
    }

    fn list_tasks_filtered(
        &self,
        status: Option<TaskStatus>,
        priority: Option<TaskPriority>,
    ) -> Result<Vec<Task>, OrbitError> {
        self.list_tasks_filtered(status, priority)
    }

    fn get_task(&self, id: &str) -> Result<Option<Task>, OrbitError> {
        self.get_task(id)
    }

    fn search_tasks(&self, query: &str) -> Result<Vec<Task>, OrbitError> {
        self.search_tasks(query)
    }

    fn update_task(&self, id: &str, params: TaskUpdateParams) -> Result<Task, OrbitError> {
        self.update_task(
            id,
            &FileTaskUpdate {
                title: params.title,
                description: params.description,
                instructions: params.instructions,
                context_files: params.context_files,
                workspace_path: params.workspace_path,
                identity_id: params.identity_id,
                assigned_to: params.assigned_to,
                created_by: params.created_by,
                approved_at: params.approved_at,
                approved_by: params.approved_by,
                approval_note: params.approval_note,
                status: params.status,
                priority: params.priority,
                task_type: params.task_type,
                owner: params.owner,
                parent_id: params.parent_id,
            },
        )
    }

    fn delete_task(&self, id: &str) -> Result<bool, OrbitError> {
        self.delete_task(id)
    }
}

impl JobStoreBackend for JobFileStore {
    fn add_job(&self, params: JobCreateParams) -> Result<Job, OrbitError> {
        self.insert_work(&FileWorkInsert {
            id: params.id,
            spec_type: params.spec_type,
            description: params.description,
            input_schema_json: params.input_schema_json,
            output_schema_json: params.output_schema_json,
            artifact_path_template: params.artifact_path_template,
            skill_refs: params.skill_refs,
            identity_id: params.identity_id,
            assigned_to: params.assigned_to,
            created_by: params.created_by,
        })
    }

    fn list_jobs(&self, include_inactive: bool) -> Result<Vec<Job>, OrbitError> {
        self.list_jobs(include_inactive)
    }

    fn get_job(&self, id: &str) -> Result<Option<Job>, OrbitError> {
        self.get_job(id)
    }

    fn disable_job(&self, id: &str) -> Result<bool, OrbitError> {
        self.disable_job(id)
    }
}

impl SchedulerStoreBackend for SchedulerFileStore {
    fn add_scheduler(&self, params: SchedulerCreateParams) -> Result<Scheduler, OrbitError> {
        self.insert_job_v2(
            params.target_type,
            &params.target_id,
            &params.schedule,
            &params.agent_cli,
            params.timeout_seconds,
            params.retry_max_attempts,
            params.retry_backoff_strategy,
            params.retry_initial_delay_seconds,
            params.next_run_at,
        )
    }

    fn list_schedulers(&self, include_disabled: bool) -> Result<Vec<Scheduler>, OrbitError> {
        self.list_schedulers(include_disabled)
    }

    fn get_scheduler(&self, scheduler_id: &str) -> Result<Option<Scheduler>, OrbitError> {
        self.get_scheduler(scheduler_id)
    }

    fn due_schedulers(&self, now: DateTime<Utc>) -> Result<Vec<Scheduler>, OrbitError> {
        self.due_schedulers(now)
    }

    fn list_scheduler_runs(&self, scheduler_id: &str) -> Result<Vec<SchedulerRun>, OrbitError> {
        self.list_scheduler_runs(scheduler_id)
    }

    fn get_pending_or_running_scheduler_run(
        &self,
        scheduler_id: &str,
    ) -> Result<Option<SchedulerRun>, OrbitError> {
        self.get_pending_or_running_scheduler_run(scheduler_id)
    }

    fn set_scheduler_state(
        &self,
        scheduler_id: &str,
        state: SchedulerScheduleState,
    ) -> Result<bool, OrbitError> {
        self.set_scheduler_state(scheduler_id, state)
    }

    fn mark_scheduler_disabled(&self, scheduler_id: &str) -> Result<bool, OrbitError> {
        self.mark_scheduler_disabled(scheduler_id)
    }

    fn update_scheduler_next_run(
        &self,
        scheduler_id: &str,
        next_run_at: DateTime<Utc>,
    ) -> Result<bool, OrbitError> {
        self.update_scheduler_next_run(scheduler_id, next_run_at)
    }

    fn insert_scheduler_run(
        &self,
        scheduler_id: &str,
        attempt: u32,
        scheduled_at: DateTime<Utc>,
    ) -> Result<SchedulerRun, OrbitError> {
        self.insert_scheduler_run(scheduler_id, attempt, scheduled_at)
    }

    fn mark_scheduler_run_running(
        &self,
        run_id: &str,
        started_at: DateTime<Utc>,
    ) -> Result<bool, OrbitError> {
        self.mark_scheduler_run_running(run_id, started_at)
    }

    fn complete_scheduler_run(
        &self,
        run_id: &str,
        state: SchedulerRunState,
        finished_at: DateTime<Utc>,
        duration_ms: Option<u64>,
        exit_code: Option<i32>,
        agent_response_json: Option<&Value>,
        error_code: Option<&str>,
        error_message: Option<&str>,
    ) -> Result<bool, OrbitError> {
        self.complete_scheduler_run(
            run_id,
            state,
            finished_at,
            duration_ms,
            exit_code,
            agent_response_json,
            error_code,
            error_message,
        )
    }

    fn claim_due_schedulers(&self, now: DateTime<Utc>) -> Result<DueJobsClaim, OrbitError> {
        self.claim_due_schedulers(now)
    }
}
