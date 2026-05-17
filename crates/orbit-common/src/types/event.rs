use serde::{Deserialize, Serialize};

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
    JobRunCancelled {
        job_id: String,
        run_id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        previous_state: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        final_state: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        actor: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        source: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        signal_attempted: Option<bool>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        signal_outcome: Option<String>,
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
    FrictionAutoResolved {
        task_id: String,
        friction_id: String,
    },
    TaskRelationDangling {
        task_id: String,
        target: String,
        relation: String,
    },
    TaskRelationSideEffectFailed {
        task_id: String,
        target: String,
        relation: String,
        reason: String,
    },
    TaskStarted {
        id: String,
        started_by: String,
        approved_from_proposed: bool,
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
    /// Recorded when [planning-duel writeback] drops an entry from the winning
    /// plan's "Context Files" section because it could not be canonicalized
    /// into a `file:` / `dir:` / `symbol:` selector. The writeback continues
    /// without the entry; consumers can use this event to debug stale or
    /// malformed plan markdown.
    PlanningDuelContextFileSkipped {
        task_id: String,
        raw_entry: String,
        reason: String,
    },
}
