use chrono::{DateTime, Utc};
use orbit_types::{
    AgentSession, AgentSessionStatus, AgentToolCall, Audit, AuditEvent, Job, OrbitError,
    OrbitEvent, Scheduler, SchedulerRun, SchedulerScheduleState, StoredTool, Task, TaskPriority,
    TaskStatus, Watch,
};

use super::contracts::{
    AgentSessionStoreBackend, AuditEventStoreBackend, AuditStoreBackend, JobCreateParams,
    JobStoreBackend, LockStoreBackend, SchedulerCreateParams, SchedulerRunCompletionParams,
    SchedulerStoreBackend, TaskCreateParams, TaskStoreBackend, TaskUpdateParams, ToolStoreBackend,
    WatchStoreBackend,
};
use crate::sqlite::audit_event_store::{AuditEventFilter, AuditEventInsertParams};
use crate::sqlite::scheduler_store::DueJobsClaim;
use crate::sqlite::task_store::{TaskInsertParams, TaskUpdateFields};
use crate::{JobInsertParams, Store};

#[derive(Clone)]
pub(crate) struct SqliteTaskStoreBackend {
    pub(crate) store: Store,
}

impl TaskStoreBackend for SqliteTaskStoreBackend {
    fn create_task(&self, params: TaskCreateParams) -> Result<Task, OrbitError> {
        let task = self.store.with_transaction(|tx| {
            tx.insert_task(&TaskInsertParams {
                title: params.title.clone(),
                description: params.description.clone(),
                plan: params.plan.clone(),
                execution_summary: params.execution_summary.clone(),
                context_files: params.context_files.clone(),
                workspace_path: params.workspace_path.clone(),
                assigned_to: params.assigned_to.clone(),
                created_by: params.created_by.clone(),
                status: params.status,
                priority: params.priority,
                task_type: params.task_type,
                branch: params.branch.clone(),
                pr_number: params.pr_number.clone(),
                proposed_by: params.proposed_by.clone(),
            })
        })?;

        self.get_task(&task.id)?
            .ok_or(OrbitError::TaskNotFound(task.id))
    }

    fn list_tasks(&self) -> Result<Vec<Task>, OrbitError> {
        self.store.list_tasks()
    }

    fn list_tasks_filtered(
        &self,
        status: Option<TaskStatus>,
        priority: Option<TaskPriority>,
    ) -> Result<Vec<Task>, OrbitError> {
        self.store.list_tasks_filtered(status, priority)
    }

    fn get_task(&self, id: &str) -> Result<Option<Task>, OrbitError> {
        self.store.get_task(id)
    }

    fn search_tasks(&self, query: &str) -> Result<Vec<Task>, OrbitError> {
        self.store.search_tasks(query)
    }

    fn update_task(&self, id: &str, params: TaskUpdateParams) -> Result<Task, OrbitError> {
        let changed = self.store.with_transaction(|tx| {
            tx.update_task(
                id,
                &TaskUpdateFields {
                    title: params.title.clone(),
                    description: params.description.clone(),
                    plan: params.plan.clone(),
                    execution_summary: params.execution_summary.clone(),
                    context_files: params.context_files.clone(),
                    workspace_path: params.workspace_path,
                    assigned_to: params.assigned_to.clone(),
                    created_by: params.created_by.clone(),
                    status: params.status,
                    priority: params.priority,
                    task_type: params.task_type,
                    branch: params.branch.clone(),
                    pr_number: params.pr_number.clone(),
                    proposed_by: params.proposed_by.clone(),
                    proposal_approved_by: params.proposal_approved_by.clone(),
                    proposal_decision_note: params.proposal_decision_note.clone(),
                    review_approved_by: params.review_approved_by.clone(),
                    review_decision_note: params.review_decision_note.clone(),
                },
            )
        })?;

        if !changed {
            return Err(OrbitError::TaskNotFound(id.to_string()));
        }

        self.get_task(id)?
            .ok_or_else(|| OrbitError::TaskNotFound(id.to_string()))
    }

    fn delete_task(&self, id: &str) -> Result<bool, OrbitError> {
        self.store.with_transaction(|tx| tx.delete_task(id))
    }
}

#[derive(Clone)]
pub(crate) struct SqliteJobStoreBackend {
    pub(crate) store: Store,
}

impl JobStoreBackend for SqliteJobStoreBackend {
    fn add_job(&self, params: JobCreateParams) -> Result<Job, OrbitError> {
        self.store.with_transaction(|tx| {
            tx.insert_work(&JobInsertParams {
                id: params.id.clone(),
                spec_type: params.spec_type.clone(),
                description: params.description.clone(),
                instruction: params.instruction.clone(),
                input_schema_json: params.input_schema_json.clone(),
                output_schema_json: params.output_schema_json.clone(),
                artifact_path_template: params.artifact_path_template.clone(),
                skill_refs: params.skill_refs.clone(),
                identity_id: params.identity_id.clone(),
                assigned_to: params.assigned_to.clone(),
                created_by: params.created_by.clone(),
            })
        })
    }

    fn list_jobs(&self, include_inactive: bool) -> Result<Vec<Job>, OrbitError> {
        self.store.list_jobs(include_inactive)
    }

    fn get_job(&self, id: &str) -> Result<Option<Job>, OrbitError> {
        self.store.get_job(id)
    }

    fn disable_job(&self, id: &str) -> Result<bool, OrbitError> {
        self.store.with_transaction(|tx| tx.disable_job(id))
    }
}

#[derive(Clone)]
pub(crate) struct SqliteSchedulerStoreBackend {
    pub(crate) store: Store,
}

impl SchedulerStoreBackend for SqliteSchedulerStoreBackend {
    fn add_scheduler(&self, params: SchedulerCreateParams) -> Result<Scheduler, OrbitError> {
        self.store.with_transaction(|tx| {
            tx.insert_job_v2(
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
        })
    }

    fn list_schedulers(&self, include_disabled: bool) -> Result<Vec<Scheduler>, OrbitError> {
        self.store.list_schedulers(include_disabled)
    }

    fn get_scheduler(&self, scheduler_id: &str) -> Result<Option<Scheduler>, OrbitError> {
        self.store.get_scheduler(scheduler_id)
    }

    fn due_schedulers(&self, now: DateTime<Utc>) -> Result<Vec<Scheduler>, OrbitError> {
        self.store.due_schedulers(now)
    }

    fn list_scheduler_runs(&self, scheduler_id: &str) -> Result<Vec<SchedulerRun>, OrbitError> {
        self.store.list_scheduler_runs(scheduler_id)
    }

    fn get_pending_or_running_scheduler_run(
        &self,
        scheduler_id: &str,
    ) -> Result<Option<SchedulerRun>, OrbitError> {
        self.store
            .get_pending_or_running_scheduler_run(scheduler_id)
    }

    fn set_scheduler_state(
        &self,
        scheduler_id: &str,
        state: SchedulerScheduleState,
    ) -> Result<bool, OrbitError> {
        self.store
            .with_transaction(|tx| tx.set_scheduler_state(scheduler_id, state))
    }

    fn mark_scheduler_disabled(&self, scheduler_id: &str) -> Result<bool, OrbitError> {
        self.store
            .with_transaction(|tx| tx.mark_scheduler_disabled(scheduler_id))
    }

    fn update_scheduler_next_run(
        &self,
        scheduler_id: &str,
        next_run_at: DateTime<Utc>,
    ) -> Result<bool, OrbitError> {
        self.store
            .with_transaction(|tx| tx.update_scheduler_next_run(scheduler_id, next_run_at))
    }

    fn insert_scheduler_run(
        &self,
        scheduler_id: &str,
        attempt: u32,
        scheduled_at: DateTime<Utc>,
    ) -> Result<SchedulerRun, OrbitError> {
        self.store
            .with_transaction(|tx| tx.insert_scheduler_run(scheduler_id, attempt, scheduled_at))
    }

    fn mark_scheduler_run_running(
        &self,
        run_id: &str,
        started_at: DateTime<Utc>,
    ) -> Result<bool, OrbitError> {
        self.store
            .with_transaction(|tx| tx.mark_scheduler_run_running(run_id, started_at))
    }

    fn complete_scheduler_run(
        &self,
        params: &SchedulerRunCompletionParams,
    ) -> Result<bool, OrbitError> {
        self.store.with_transaction(|tx| {
            tx.complete_scheduler_run(
                params.run_id,
                params.state,
                params.finished_at,
                params.duration_ms,
                params.exit_code,
                params.agent_response_json,
                params.error_code,
                params.error_message,
            )
        })
    }

    fn claim_due_schedulers(&self, now: DateTime<Utc>) -> Result<DueJobsClaim, OrbitError> {
        self.store
            .with_transaction(|tx| tx.claim_due_schedulers(now))
    }
}

#[derive(Clone)]
pub(crate) struct SqliteToolStoreBackend {
    pub(crate) store: Store,
}

impl ToolStoreBackend for SqliteToolStoreBackend {
    fn list_tools(&self) -> Result<Vec<StoredTool>, OrbitError> {
        self.store.list_tools()
    }

    fn get_tool(&self, name: &str) -> Result<Option<StoredTool>, OrbitError> {
        self.store.get_tool(name)
    }

    fn insert_tool(&self, tool: &StoredTool) -> Result<(), OrbitError> {
        self.store.with_transaction(|tx| tx.insert_tool(tool))
    }

    fn delete_tool(&self, name: &str) -> Result<bool, OrbitError> {
        self.store.with_transaction(|tx| tx.delete_tool(name))
    }

    fn set_tool_enabled(&self, name: &str, enabled: bool) -> Result<bool, OrbitError> {
        self.store
            .with_transaction(|tx| tx.set_tool_enabled(name, enabled))
    }
}

#[derive(Clone)]
pub(crate) struct SqliteWatchStoreBackend {
    pub(crate) store: Store,
}

impl WatchStoreBackend for SqliteWatchStoreBackend {
    fn list_watches(&self) -> Result<Vec<Watch>, OrbitError> {
        self.store.list_watches()
    }

    fn get_watch(&self, id: &str) -> Result<Option<Watch>, OrbitError> {
        self.store.get_watch(id)
    }

    fn insert_watch(
        &self,
        path: &str,
        command: &str,
        debounce_ms: u64,
    ) -> Result<Watch, OrbitError> {
        self.store
            .with_transaction(|tx| tx.insert_watch(path, command, debounce_ms))
    }
}

#[derive(Clone)]
pub(crate) struct SqliteAuditStoreBackend {
    pub(crate) store: Store,
}

impl AuditStoreBackend for SqliteAuditStoreBackend {
    fn list_audits(&self, limit: usize) -> Result<Vec<Audit>, OrbitError> {
        self.store.list_audits(limit)
    }

    fn insert_audit_event(&self, event: &OrbitEvent) -> Result<(), OrbitError> {
        self.store
            .with_transaction(|tx| tx.insert_audit_event(event))
    }
}

#[derive(Clone)]
pub(crate) struct SqliteAuditEventStoreBackend {
    pub(crate) store: Store,
}

impl AuditEventStoreBackend for SqliteAuditEventStoreBackend {
    fn insert_audit_event_record(&self, params: &AuditEventInsertParams) -> Result<(), OrbitError> {
        self.store.insert_audit_event_record(params)
    }

    fn list_audit_events(&self, filter: &AuditEventFilter) -> Result<Vec<AuditEvent>, OrbitError> {
        self.store.list_audit_events(filter)
    }

    fn get_audit_event(&self, id: i64) -> Result<Option<AuditEvent>, OrbitError> {
        self.store.get_audit_event(id)
    }

    fn get_audit_event_stats(
        &self,
        since: Option<&DateTime<Utc>>,
        tool: Option<&str>,
    ) -> Result<(i64, i64, i64, i64, f64, i64), OrbitError> {
        self.store.get_audit_event_stats(since, tool)
    }

    fn get_audit_event_durations(
        &self,
        since: Option<&DateTime<Utc>>,
        tool: Option<&str>,
    ) -> Result<Vec<i64>, OrbitError> {
        self.store.get_audit_event_durations(since, tool)
    }

    fn prune_audit_events(&self, older_than: &DateTime<Utc>) -> Result<usize, OrbitError> {
        self.store.prune_audit_events(older_than)
    }
}

#[derive(Clone)]
pub(crate) struct SqliteAgentSessionStoreBackend {
    pub(crate) store: Store,
}

impl AgentSessionStoreBackend for SqliteAgentSessionStoreBackend {
    fn get_agent_session(&self, session_id: &str) -> Result<Option<AgentSession>, OrbitError> {
        self.store.get_agent_session(session_id)
    }

    fn insert_agent_session(&self, session: &AgentSession) -> Result<(), OrbitError> {
        self.store
            .with_transaction(|tx| tx.insert_agent_session(session))
    }

    fn update_agent_session(
        &self,
        session_id: &str,
        tool_calls: &[AgentToolCall],
        outcome: &str,
        status: AgentSessionStatus,
    ) -> Result<bool, OrbitError> {
        self.store
            .with_transaction(|tx| tx.update_agent_session(session_id, tool_calls, outcome, status))
    }
}

#[derive(Clone)]
pub(crate) struct SqliteLockStoreBackend {
    pub(crate) store: Store,
}

impl LockStoreBackend for SqliteLockStoreBackend {
    fn try_lock(&self, name: &str) -> Result<bool, OrbitError> {
        self.store.try_lock(name)
    }

    fn unlock(&self, name: &str) -> Result<bool, OrbitError> {
        self.store.unlock(name)
    }

    fn global_job_lock_name(&self) -> &'static str {
        Store::global_job_lock_name()
    }
}
