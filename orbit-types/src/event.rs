use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", content = "data")]
pub enum OrbitEvent {
    ToolExecuted {
        name: String,
    },
    ToolAdded {
        name: String,
    },
    ToolRemoved {
        name: String,
    },
    ToolEnabled {
        name: String,
    },
    ToolDisabled {
        name: String,
    },
    JobAdded {
        job_id: String,
    },
    JobPaused {
        job_id: String,
    },
    JobResumed {
        job_id: String,
    },
    JobDeleted {
        job_id: String,
    },
    JobSessionStarted {
        job_id: String,
        session_id: String,
        trigger: String,
    },
    JobSessionCompleted {
        job_id: String,
        session_id: String,
        status: String,
    },
    JobSessionCancelled {
        job_id: String,
        session_id: String,
    },
    JobSkipped {
        job_id: String,
        reason: String,
    },
    JobStarted {
        id: String,
    },
    JobCompleted {
        id: String,
        success: bool,
    },
    WatchTriggered {
        path: String,
    },
    PolicyDenied {
        tool: String,
    },
    TaskAdded {
        id: String,
    },
    TaskUpdated {
        id: String,
    },
    TaskClosed {
        id: String,
    },
    TaskReopened {
        id: String,
    },
    TaskDeleted {
        id: String,
    },
    SkillAdded {
        name: String,
    },
    SkillUpdated {
        name: String,
    },
    SkillDeleted {
        name: String,
    },
    SkillAttached {
        task_id: String,
        skill_name: String,
    },
    SkillDetached {
        task_id: String,
        skill_name: String,
    },
    AgentSessionStarted {
        session_id: String,
        task_id: String,
        skill_names: Vec<String>,
        composed_context_hash: String,
        effective_allowed_tools: Vec<String>,
    },
    AgentToolCall {
        session_id: String,
        task_id: String,
        skill_names: Vec<String>,
        tool_name: String,
        input: Value,
        output: Option<Value>,
        success: bool,
    },
    AgentSessionCompleted {
        session_id: String,
        task_id: String,
        status: String,
    },
    EntryCreated {
        id: String,
        entity_type: String,
        entity_id: String,
        sequence_number: i64,
    },
}
