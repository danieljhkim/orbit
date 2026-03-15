use chrono::{DateTime, Utc};
use orbit_types::{
    Activity, Job, JobRun, JobScheduleState, OrbitError, Task, TaskPriority, TaskStatus,
};

use super::contracts::{
    ActivityCreateParams, ActivityStoreBackend, ActivityUpdateParams, JobCreateParams, JobRunQuery,
    JobRunStepParams, JobStoreBackend, TaskCreateParams, TaskStoreBackend, TaskUpdateParams,
};
use crate::file::activity_store::{ActivityFileStore, FileWorkInsert};
use crate::file::job_store::JobFileStore;
use crate::file::task_store::{FileTaskInsert, FileTaskUpdate, TaskFileStore};

impl TaskStoreBackend for TaskFileStore {
    fn create_task(&self, params: TaskCreateParams) -> Result<Task, OrbitError> {
        self.create_task(FileTaskInsert {
            title: params.title,
            description: params.description,
            plan: params.plan,
            execution_summary: params.execution_summary,
            context_files: params.context_files,
            workspace_path: params.workspace_path,
            assigned_to: params.assigned_to,
            created_by: params.created_by,
            status: params.status,
            priority: params.priority,
            task_type: params.task_type,
            branch: params.branch,
            pr_number: params.pr_number,
            proposed_by: params.proposed_by,
            comments: params.comments,
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
                plan: params.plan,
                execution_summary: params.execution_summary,
                context_files: params.context_files,
                workspace_path: params.workspace_path,
                assigned_to: params.assigned_to,
                created_by: params.created_by,
                status: params.status,
                priority: params.priority,
                task_type: params.task_type,
                branch: params.branch,
                pr_number: params.pr_number,
                proposed_by: params.proposed_by,
                proposal_approved_by: params.proposal_approved_by,
                proposal_rejected_by: params.proposal_rejected_by,
                proposal_decision_note: params.proposal_decision_note,
                review_approved_by: params.review_approved_by,
                review_rejected_by: params.review_rejected_by,
                review_decision_note: params.review_decision_note,
                append_comments: params.append_comments,
            },
        )
    }

    fn delete_task(&self, id: &str) -> Result<bool, OrbitError> {
        self.delete_task(id)
    }
}

impl ActivityStoreBackend for ActivityFileStore {
    fn add_activity(&self, params: ActivityCreateParams) -> Result<Activity, OrbitError> {
        self.insert_work(&FileWorkInsert {
            id: params.id,
            spec_type: params.spec_type,
            description: params.description,
            input_schema_json: params.input_schema_json,
            output_schema_json: params.output_schema_json,
            spec_config: params.spec_config,
            workspace_path: params.workspace_path,
            identity_id: params.identity_id,
            created_by: params.created_by,
        })
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
        self.update_activity(
            id,
            params.description,
            params.input_schema_json,
            params.output_schema_json,
            params.spec_config,
            params.workspace_path,
            params.identity_id,
            params.is_active,
        )
    }

    fn disable_activity(&self, id: &str) -> Result<bool, OrbitError> {
        self.disable_activity(id)
    }
}

impl JobStoreBackend for JobFileStore {
    fn add_job(&self, params: JobCreateParams) -> Result<Job, OrbitError> {
        self.insert_activity_v2(params.job_id, params.steps, params.initial_state)
    }

    fn list_jobs(&self, include_disabled: bool) -> Result<Vec<Job>, OrbitError> {
        self.list_jobs(include_disabled)
    }

    fn get_job(&self, job_id: &str) -> Result<Option<Job>, OrbitError> {
        self.get_job(job_id)
    }

    fn list_job_runs(&self, job_id: &str) -> Result<Vec<JobRun>, OrbitError> {
        self.list_job_runs(job_id)
    }

    fn list_job_runs_filtered(&self, query: &JobRunQuery) -> Result<Vec<JobRun>, OrbitError> {
        self.list_job_runs_filtered(query)
    }

    fn get_job_run(&self, run_id: &str) -> Result<Option<JobRun>, OrbitError> {
        self.get_job_run(run_id)
    }

    fn get_pending_or_running_job_run(&self, job_id: &str) -> Result<Option<JobRun>, OrbitError> {
        self.get_pending_or_running_job_run(job_id)
    }

    fn set_job_state(&self, job_id: &str, state: JobScheduleState) -> Result<bool, OrbitError> {
        self.set_job_state(job_id, state)
    }

    fn mark_job_disabled(&self, job_id: &str) -> Result<bool, OrbitError> {
        self.mark_job_disabled(job_id)
    }

    fn insert_job_run(
        &self,
        job_id: &str,
        attempt: u32,
        scheduled_at: DateTime<Utc>,
    ) -> Result<JobRun, OrbitError> {
        self.insert_job_run(job_id, attempt, scheduled_at)
    }

    fn mark_job_run_running(
        &self,
        run_id: &str,
        started_at: DateTime<Utc>,
    ) -> Result<bool, OrbitError> {
        self.mark_job_run_running(run_id, started_at)
    }

    fn complete_job_run_step(
        &self,
        run_id: &str,
        params: &JobRunStepParams,
    ) -> Result<bool, OrbitError> {
        self.complete_job_run_step(run_id, params)
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
}
