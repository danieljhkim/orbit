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
    JobUpdated {
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
    JobTriggered {
        job_id: String,
    },
    JobRunStarted {
        job_id: String,
        run_id: String,
        attempt: u32,
    },
    JobRunCompleted {
        job_id: String,
        run_id: String,
        state: String,
    },
    JobProtocolViolation {
        job_id: String,
        run_id: String,
        message: String,
    },
    JobSkipped {
        job_id: String,
        reason: String,
    },
    JobRunArchived {
        job_id: String,
        run_id: String,
    },
    JobRunDeleted {
        job_id: String,
        run_id: String,
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
    TaskArchived {
        id: String,
    },
    TaskUnarchived {
        id: String,
    },
    TaskProposalApproved {
        id: String,
        approved_by: String,
    },
    TaskProposalRejected {
        id: String,
        rejected_by: String,
    },
    TaskReviewApproved {
        id: String,
        approved_by: String,
    },
    TaskReviewRejected {
        id: String,
        rejected_by: String,
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
    ActivityAdded {
        id: String,
    },
    ActivityUpdated {
        id: String,
    },
    ActivityDisabled {
        id: String,
    },
    ActivityRunStarted {
        id: String,
    },
    ActivityRunCompleted {
        id: String,
        state: String,
    },
    ActivityProtocolViolation {
        id: String,
        message: String,
    },
    AgentSessionStarted {
        session_id: String,
        task_id: String,
        identity_id: Option<String>,
        identity_name: Option<String>,
        identity_role: Option<String>,
        identity_block: Option<String>,
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
}
