use chrono::{DateTime, Utc};
use orbit_types::{
    Activity, AgentSession, AgentSessionStatus, AgentToolCall, Audit, AuditEvent, Job, JobRun,
    JobScheduleState, OrbitError, OrbitEvent, StoredTool,
};

use super::contracts::{
    ActivityCreateParams, ActivityStoreBackend, AgentSessionStoreBackend, AuditEventStoreBackend,
    AuditStoreBackend, JobCreateParams, JobRunCompletionParams, JobStoreBackend, LockStoreBackend,
    ToolStoreBackend,
};
use crate::sqlite::audit_event_store::{AuditEventFilter, AuditEventInsertParams};
use crate::sqlite::job_store::DueJobsClaim;
use crate::{ActivityInsertParams, Store};

#[derive(Clone)]
pub(crate) struct SqliteActivityStoreBackend {
    pub(crate) store: Store,
}

impl ActivityStoreBackend for SqliteActivityStoreBackend {
    fn add_activity(&self, params: ActivityCreateParams) -> Result<Activity, OrbitError> {
        self.store.with_transaction(|tx| {
            tx.insert_work(&ActivityInsertParams {
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

    fn list_activities(&self, include_inactive: bool) -> Result<Vec<Activity>, OrbitError> {
        self.store.list_activities(include_inactive)
    }

    fn get_activity(&self, id: &str) -> Result<Option<Activity>, OrbitError> {
        self.store.get_activity(id)
    }

    fn disable_activity(&self, id: &str) -> Result<bool, OrbitError> {
        self.store.with_transaction(|tx| tx.disable_activity(id))
    }
}

#[derive(Clone)]
pub(crate) struct SqliteJobStoreBackend {
    pub(crate) store: Store,
}

impl JobStoreBackend for SqliteJobStoreBackend {
    fn add_job(&self, params: JobCreateParams) -> Result<Job, OrbitError> {
        self.store.with_transaction(|tx| {
            tx.insert_activity_v2(
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

    fn list_jobs(&self, include_disabled: bool) -> Result<Vec<Job>, OrbitError> {
        self.store.list_jobs(include_disabled)
    }

    fn get_job(&self, job_id: &str) -> Result<Option<Job>, OrbitError> {
        self.store.get_job(job_id)
    }

    fn due_jobs(&self, now: DateTime<Utc>) -> Result<Vec<Job>, OrbitError> {
        self.store.due_jobs(now)
    }

    fn next_due_job_time(&self) -> Result<Option<DateTime<Utc>>, OrbitError> {
        self.store.next_due_job_time()
    }

    fn list_job_runs(&self, job_id: &str) -> Result<Vec<JobRun>, OrbitError> {
        self.store.list_job_runs(job_id)
    }

    fn get_pending_or_running_job_run(&self, job_id: &str) -> Result<Option<JobRun>, OrbitError> {
        self.store.get_pending_or_running_job_run(job_id)
    }

    fn set_job_state(&self, job_id: &str, state: JobScheduleState) -> Result<bool, OrbitError> {
        self.store
            .with_transaction(|tx| tx.set_job_state(job_id, state))
    }

    fn mark_job_disabled(&self, job_id: &str) -> Result<bool, OrbitError> {
        self.store
            .with_transaction(|tx| tx.mark_job_disabled(job_id))
    }

    fn update_job_next_run(
        &self,
        job_id: &str,
        next_run_at: DateTime<Utc>,
    ) -> Result<bool, OrbitError> {
        self.store
            .with_transaction(|tx| tx.update_job_next_run(job_id, next_run_at))
    }

    fn insert_job_run(
        &self,
        job_id: &str,
        attempt: u32,
        scheduled_at: DateTime<Utc>,
    ) -> Result<JobRun, OrbitError> {
        self.store
            .with_transaction(|tx| tx.insert_job_run(job_id, attempt, scheduled_at))
    }

    fn mark_job_run_running(
        &self,
        run_id: &str,
        started_at: DateTime<Utc>,
    ) -> Result<bool, OrbitError> {
        self.store
            .with_transaction(|tx| tx.mark_job_run_running(run_id, started_at))
    }

    fn complete_job_run(&self, params: &JobRunCompletionParams) -> Result<bool, OrbitError> {
        self.store.with_transaction(|tx| {
            tx.complete_job_run(
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

    fn claim_due_jobs(&self, now: DateTime<Utc>) -> Result<DueJobsClaim, OrbitError> {
        self.store.with_transaction(|tx| tx.claim_due_jobs(now))
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
