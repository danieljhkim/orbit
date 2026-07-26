use crate::OrbitRuntime;
use orbit_common::types::OrbitError;
use orbit_store::{
    ActivityInvocationMetrics, AgentInvocationMetrics, InvocationInsertParams, InvocationQuery,
    InvocationRecord, Store, TaskInvocationMetrics, ToolInvocationMetrics,
};

pub(super) fn open_invocation_store(runtime: &OrbitRuntime) -> Result<Store, OrbitError> {
    Store::open(&runtime.context.persistence().audit_db)
}

impl OrbitRuntime {
    pub fn activity_invocation_metrics(
        &self,
    ) -> Result<Vec<ActivityInvocationMetrics>, OrbitError> {
        open_invocation_store(self)?.list_activity_invocation_metrics()
    }

    pub fn agent_invocation_metrics(&self) -> Result<Vec<AgentInvocationMetrics>, OrbitError> {
        open_invocation_store(self)?.list_agent_invocation_metrics()
    }

    pub fn task_invocation_metrics(
        &self,
        task_id: &str,
    ) -> Result<TaskInvocationMetrics, OrbitError> {
        open_invocation_store(self)?.get_task_invocation_metrics(task_id)
    }

    pub fn tool_invocation_metrics(&self) -> Result<Vec<ToolInvocationMetrics>, OrbitError> {
        open_invocation_store(self)?.list_tool_invocation_metrics()
    }

    pub fn invocation_records(
        &self,
        query: InvocationQuery,
    ) -> Result<Vec<InvocationRecord>, OrbitError> {
        open_invocation_store(self)?.list_invocation_records(&query)
    }

    pub fn insert_invocation_trace_record(
        &self,
        params: &InvocationInsertParams,
    ) -> Result<(), OrbitError> {
        open_invocation_store(self)?.insert_invocation_trace_record(params)
    }
}
