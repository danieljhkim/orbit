use orbit_store::{
    ActivityCreateParams, ActivityStoreBackend, ActivityUpdateParams, AuditEventFilter,
    AuditEventInsertParams, AuditEventStoreBackend, ExecutorDefStoreBackend, JobCreateParams,
    JobRunQuery, JobRunStepParams, JobStoreBackend, JobUpdateParams, PolicyDefStoreBackend,
    TaskCreateParams, TaskStoreBackend, TaskUpdateParams as StoreTaskUpdateParams,
    ToolStoreBackend,
};
use orbit_types::{
    Activity, AuditEvent, ExecutorDef, Job, JobRun, JobRunState, KnowledgeRunMetrics, OrbitError,
    PolicyDef, StoredTool, Task, TaskArtifact, TaskPriority, TaskStatus,
};

use crate::context::OrbitStores;

impl OrbitStores {
    pub(crate) fn tasks(&self) -> TaskRecords<'_> {
        TaskRecords {
            store: self.task.as_ref(),
        }
    }

    pub(crate) fn activities(&self) -> ActivityRecords<'_> {
        ActivityRecords {
            store: self.activity.as_ref(),
        }
    }

    pub(crate) fn jobs(&self) -> JobRecords<'_> {
        JobRecords {
            store: self.job.as_ref(),
        }
    }

    pub(crate) fn tools(&self) -> ToolRecords<'_> {
        ToolRecords {
            store: self.tool.as_ref(),
        }
    }

    pub(crate) fn audit_events(&self) -> AuditEventRecords<'_> {
        AuditEventRecords {
            store: self.audit_event.as_ref(),
        }
    }

    pub(crate) fn executors(&self) -> ExecutorDefRecords<'_> {
        ExecutorDefRecords {
            store: self.executor_def.as_ref(),
        }
    }

    pub(crate) fn policies(&self) -> PolicyDefRecords<'_> {
        PolicyDefRecords {
            store: self.policy_def.as_ref(),
        }
    }
}

pub(crate) struct TaskRecords<'a> {
    store: &'a dyn TaskStoreBackend,
}

impl TaskRecords<'_> {
    pub(crate) fn create(&self, params: TaskCreateParams) -> Result<Task, OrbitError> {
        self.store.create_task(params)
    }

    pub(crate) fn get(&self, id: &str) -> Result<Option<Task>, OrbitError> {
        self.store.get_task(id)
    }

    pub(crate) fn get_artifacts(&self, id: &str) -> Result<Option<Vec<TaskArtifact>>, OrbitError> {
        self.store.get_task_artifacts(id)
    }

    pub(crate) fn list(&self) -> Result<Vec<Task>, OrbitError> {
        self.store.list_tasks()
    }

    pub(crate) fn list_filtered(
        &self,
        status: Option<TaskStatus>,
        priority: Option<TaskPriority>,
        parent_id: Option<&str>,
        batch_id: Option<&str>,
    ) -> Result<Vec<Task>, OrbitError> {
        self.store
            .list_tasks_filtered(status, priority, parent_id, batch_id)
    }

    pub(crate) fn update(
        &self,
        id: &str,
        params: StoreTaskUpdateParams,
    ) -> Result<Task, OrbitError> {
        self.store.update_task(id, params)
    }

    pub(crate) fn delete(&self, id: &str) -> Result<bool, OrbitError> {
        self.store.delete_task(id)
    }

    pub(crate) fn search(&self, query: &str) -> Result<Vec<Task>, OrbitError> {
        self.store.search_tasks(query)
    }
}

pub(crate) struct ActivityRecords<'a> {
    store: &'a dyn ActivityStoreBackend,
}

impl ActivityRecords<'_> {
    pub(crate) fn add(&self, params: ActivityCreateParams) -> Result<Activity, OrbitError> {
        self.store.add_activity(params)
    }

    pub(crate) fn list(&self, include_inactive: bool) -> Result<Vec<Activity>, OrbitError> {
        self.store.list_activities(include_inactive)
    }

    pub(crate) fn get(&self, id: &str) -> Result<Option<Activity>, OrbitError> {
        self.store.get_activity(id)
    }

    pub(crate) fn update(
        &self,
        id: &str,
        params: ActivityUpdateParams,
    ) -> Result<Activity, OrbitError> {
        self.store.update_activity(id, params)
    }

    pub(crate) fn disable(&self, id: &str) -> Result<bool, OrbitError> {
        self.store.disable_activity(id)
    }
}

pub(crate) struct JobRecords<'a> {
    store: &'a dyn JobStoreBackend,
}

impl JobRecords<'_> {
    pub(crate) fn add(&self, params: JobCreateParams) -> Result<Job, OrbitError> {
        self.store.add_job(params)
    }

    pub(crate) fn update(&self, job_id: &str, params: JobUpdateParams) -> Result<Job, OrbitError> {
        self.store.update_job(job_id, params)
    }

    pub(crate) fn mark_disabled(&self, job_id: &str) -> Result<bool, OrbitError> {
        self.store.mark_job_disabled(job_id)
    }

    pub(crate) fn list(&self, include_disabled: bool) -> Result<Vec<Job>, OrbitError> {
        self.store.list_jobs(include_disabled)
    }

    pub(crate) fn get(&self, job_id: &str) -> Result<Option<Job>, OrbitError> {
        self.store.get_job(job_id)
    }

    pub(crate) fn list_runs_filtered(
        &self,
        query: &JobRunQuery,
    ) -> Result<Vec<JobRun>, OrbitError> {
        self.store.list_job_runs_filtered(query)
    }

    pub(crate) fn list_all_pending_or_running(&self) -> Result<Vec<JobRun>, OrbitError> {
        self.store.list_all_pending_or_running_runs()
    }

    pub(crate) fn list_pending_or_running(&self, job_id: &str) -> Result<Vec<JobRun>, OrbitError> {
        self.store.list_pending_or_running_job_runs(job_id)
    }

    pub(crate) fn insert_run(
        &self,
        job_id: &str,
        attempt: u32,
        scheduled_at: chrono::DateTime<chrono::Utc>,
        input: Option<serde_json::Value>,
        retry_source_run_id: Option<String>,
    ) -> Result<JobRun, OrbitError> {
        self.store
            .insert_job_run(job_id, attempt, scheduled_at, input, retry_source_run_id)
    }

    pub(crate) fn mark_run_running(
        &self,
        run_id: &str,
        started_at: chrono::DateTime<chrono::Utc>,
        pid: u32,
    ) -> Result<bool, OrbitError> {
        self.store.mark_job_run_running(run_id, started_at, pid)
    }

    pub(crate) fn take_over_running_run(
        &self,
        run_id: &str,
        expected_pid: Option<u32>,
        expected_pid_start_time: Option<String>,
        started_at: chrono::DateTime<chrono::Utc>,
        pid: u32,
    ) -> Result<bool, OrbitError> {
        self.store.take_over_running_job_run(
            run_id,
            expected_pid,
            expected_pid_start_time,
            started_at,
            pid,
        )
    }

    pub(crate) fn abandon_run(
        &self,
        run_id: &str,
        finished_at: chrono::DateTime<chrono::Utc>,
    ) -> Result<bool, OrbitError> {
        self.store.abandon_job_run(run_id, finished_at)
    }

    pub(crate) fn complete_run_step(
        &self,
        run_id: &str,
        params: &JobRunStepParams,
    ) -> Result<bool, OrbitError> {
        self.store.complete_job_run_step(run_id, params)
    }

    pub(crate) fn record_run_knowledge_metrics(
        &self,
        run_id: &str,
        metrics: KnowledgeRunMetrics,
    ) -> Result<bool, OrbitError> {
        self.store.record_job_run_knowledge_metrics(run_id, metrics)
    }

    pub(crate) fn finalize_run(
        &self,
        run_id: &str,
        state: JobRunState,
        finished_at: chrono::DateTime<chrono::Utc>,
        duration_ms: Option<u64>,
    ) -> Result<bool, OrbitError> {
        self.store
            .finalize_job_run(run_id, state, finished_at, duration_ms)
    }

    pub(crate) fn get_run(&self, run_id: &str) -> Result<Option<JobRun>, OrbitError> {
        self.store.get_job_run(run_id)
    }

    pub(crate) fn read_run_state(
        &self,
        run_id: &str,
    ) -> Result<Option<orbit_types::PipelineState>, OrbitError> {
        self.store.read_run_state(run_id)
    }

    pub(crate) fn write_run_state(
        &self,
        run_id: &str,
        state: &orbit_types::PipelineState,
    ) -> Result<(), OrbitError> {
        self.store.write_run_state(run_id, state)
    }

    pub(crate) fn list_runs(&self, job_id: &str) -> Result<Vec<JobRun>, OrbitError> {
        self.store.list_job_runs(job_id)
    }

    pub(crate) fn archive_run(&self, run_id: &str) -> Result<String, OrbitError> {
        self.store.archive_job_run(run_id)
    }

    pub(crate) fn delete_run(&self, run_id: &str) -> Result<String, OrbitError> {
        self.store.delete_job_run(run_id)
    }
}

pub(crate) struct ToolRecords<'a> {
    store: &'a dyn ToolStoreBackend,
}

impl ToolRecords<'_> {
    pub(crate) fn list(&self) -> Result<Vec<StoredTool>, OrbitError> {
        self.store.list_tools()
    }

    pub(crate) fn get(&self, name: &str) -> Result<Option<StoredTool>, OrbitError> {
        self.store.get_tool(name)
    }

    pub(crate) fn insert(&self, tool: &StoredTool) -> Result<(), OrbitError> {
        self.store.insert_tool(tool)
    }

    pub(crate) fn delete(&self, name: &str) -> Result<bool, OrbitError> {
        self.store.delete_tool(name)
    }

    pub(crate) fn set_enabled(&self, name: &str, enabled: bool) -> Result<bool, OrbitError> {
        self.store.set_tool_enabled(name, enabled)
    }
}

pub(crate) struct AuditEventRecords<'a> {
    store: &'a dyn AuditEventStoreBackend,
}

impl AuditEventRecords<'_> {
    pub(crate) fn list(&self, filter: &AuditEventFilter) -> Result<Vec<AuditEvent>, OrbitError> {
        self.store.list_audit_events(filter)
    }

    pub(crate) fn get(&self, id: i64) -> Result<Option<AuditEvent>, OrbitError> {
        self.store.get_audit_event(id)
    }

    pub(crate) fn prune(
        &self,
        older_than: &chrono::DateTime<chrono::Utc>,
    ) -> Result<usize, OrbitError> {
        self.store.prune_audit_events(older_than)
    }

    pub(crate) fn stats(
        &self,
        since: Option<&chrono::DateTime<chrono::Utc>>,
        tool: Option<&str>,
    ) -> Result<(i64, i64, i64, i64, f64, i64), OrbitError> {
        self.store.get_audit_event_stats(since, tool)
    }

    pub(crate) fn durations(
        &self,
        since: Option<&chrono::DateTime<chrono::Utc>>,
        tool: Option<&str>,
    ) -> Result<Vec<i64>, OrbitError> {
        self.store.get_audit_event_durations(since, tool)
    }

    pub(crate) fn insert(&self, params: &AuditEventInsertParams) -> Result<(), OrbitError> {
        self.store.insert_audit_event_record(params)
    }
}

pub(crate) struct ExecutorDefRecords<'a> {
    store: &'a dyn ExecutorDefStoreBackend,
}

impl ExecutorDefRecords<'_> {
    pub(crate) fn list(&self) -> Result<Vec<ExecutorDef>, OrbitError> {
        self.store.list_executor_defs()
    }

    pub(crate) fn get(&self, name: &str) -> Result<Option<ExecutorDef>, OrbitError> {
        self.store.get_executor_def(name)
    }

    pub(crate) fn upsert(&self, def: &ExecutorDef) -> Result<(), OrbitError> {
        self.store.upsert_executor_def(def)
    }
}

pub(crate) struct PolicyDefRecords<'a> {
    store: &'a dyn PolicyDefStoreBackend,
}

impl PolicyDefRecords<'_> {
    pub(crate) fn list(&self) -> Result<Vec<PolicyDef>, OrbitError> {
        self.store.list_policy_defs()
    }

    pub(crate) fn get(&self, name: &str) -> Result<Option<PolicyDef>, OrbitError> {
        self.store.get_policy_def(name)
    }

    pub(crate) fn upsert(&self, def: &PolicyDef) -> Result<(), OrbitError> {
        self.store.upsert_policy_def(def)
    }
}
