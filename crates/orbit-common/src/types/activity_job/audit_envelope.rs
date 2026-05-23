use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Schema version for the §7 v2 audit envelope. Per §12 Q10 resolution,
/// versioning is PER EVENT TYPE — each variant of `V2AuditEventKind` can be
/// versioned independently. This constant is the envelope schema itself.
pub const AUDIT_ENVELOPE_SCHEMA_VERSION: u32 = 1;

pub const V2_EVENT_TYPE_FS_CALL_DENIED: &str = "fs.call.denied";
pub const V2_EVENT_TYPE_TOOL_DENIED: &str = "tool.denied";
pub const V2_EVENT_TYPE_STEP_DENIED: &str = "step.denied";
pub const V2_DENIAL_EVENT_TYPES: &[&str] = &[
    V2_EVENT_TYPE_FS_CALL_DENIED,
    V2_EVENT_TYPE_TOOL_DENIED,
    V2_EVENT_TYPE_STEP_DENIED,
];

/// Common envelope fields wrapping every v2 audit event (§7).
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct V2AuditEnvelope {
    #[serde(rename = "schemaVersion")]
    pub schema_version: u32,
    pub event_type: String,
    pub event_id: String,
    pub ts: DateTime<Utc>,
    pub run_id: String,
    pub agent_identity: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent_event_id: Option<String>,
    /// Absolute filesystem path of the workspace that produced this event.
    /// Populated by CLI entry points so persisted v2 audit rows can be
    /// filtered by origin repo.
    /// Absent for smokes and stub hosts that don't carry a workspace identity.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub workspace_path: Option<String>,
}

/// §7 v2 audit event — the envelope plus a type-specific body.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct V2AuditEvent {
    #[serde(flatten)]
    pub envelope: V2AuditEnvelope,
    #[serde(flatten)]
    pub kind: V2AuditEventKind,
}

/// Event-type discriminator (§7). The v2 layer emits run.*, step.*,
/// activity.*, construct-level (parallel / fan_out / loop), and tool.denied
/// events. Loop-engine http.* and tool.call.* events continue to be emitted
/// by the loop engine and are referenced via `parent_event_id` from Activity
/// events.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(tag = "body_kind", rename_all = "snake_case")]
pub enum V2AuditEventKind {
    RunStarted {
        job_name: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        retry_source_run_id: Option<String>,
    },
    RunFinished {
        outcome: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        error_message: Option<String>,
    },
    StepStarted {
        step_id: String,
    },
    StepFinished {
        step_id: String,
        outcome: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        error_message: Option<String>,
    },
    StepSkipped {
        step_id: String,
        reason: String,
    },
    StepRetry {
        step_id: String,
        attempt: u32,
        next_backoff_ms: u64,
    },
    StepRecoveryAttempted {
        step_id: String,
        recovery_activity: String,
        recovery_succeeded: bool,
    },
    StepDenied {
        step_id: String,
        reason: String,
    },
    StepJoin {
        step_id: String,
        mode: String,
        branch_outcomes: Vec<BranchOutcome>,
    },
    FanoutDispatched {
        step_id: String,
        worker_count: u32,
    },
    WorkerState {
        step_id: String,
        worker_index: u32,
        state: String,
    },
    FaninJoined {
        step_id: String,
        collected: u32,
        failed: u32,
    },
    LoopIterationStart {
        step_id: String,
        iteration: u32,
    },
    LoopIterationEnd {
        step_id: String,
        iteration: u32,
        broke: bool,
    },
    LoopDidNotConverge {
        step_id: String,
        max_iterations: u32,
    },
    ActivityStarted {
        activity_name: String,
        activity_type: String,
    },
    ActivityFinished {
        activity_name: String,
        outcome: String,
    },
    FsCallRequest {
        profile: String,
        op: String,
        path: String,
        allowed: bool,
        matched_rule: String,
    },
    FsCallResult {
        profile: String,
        op: String,
        path: String,
        allowed: bool,
        matched_rule: String,
    },
    FsCallDenied {
        profile: String,
        op: String,
        path: String,
        allowed: bool,
        matched_rule: String,
    },
    ToolDenied {
        tool_name: String,
        reason: String,
    },
    /// §6 harness-delegated allowlist advisory. Emitted once per CLI backend
    /// invocation when the declared `tools:` list is passed through to the
    /// provider harness (Orbit does not enforce it in CLI mode).
    ToolAllowlistHarnessDelegated {
        provider: String,
        tools: Vec<String>,
    },
    /// §7.6 — CLI backend subprocess starting. Emitted after redaction has been
    /// applied to `argv`; the stdin blob is already written and hashed by the
    /// time this event fires.
    CliInvocationStarted {
        provider: String,
        argv_redacted: Vec<String>,
        stdin_blob_ref: Option<String>,
        model: Option<String>,
        cwd: Option<String>,
        wall_clock_timeout_ms: u64,
    },
    /// §7.6 — CLI backend subprocess finished (either naturally or by
    /// wall-clock timeout). `timed_out == true` iff the subprocess was killed
    /// because it exceeded `wall_clock_timeout_ms`.
    CliInvocationFinished {
        provider: String,
        exit_code: Option<i32>,
        duration_ms: u64,
        stdout_blob_ref: Option<String>,
        stderr_blob_ref: Option<String>,
        harness_version: Option<String>,
        timed_out: bool,
    },
}

impl V2AuditEventKind {
    pub fn event_type(&self) -> &'static str {
        match self {
            V2AuditEventKind::RunStarted { .. } => "run.started",
            V2AuditEventKind::RunFinished { .. } => "run.finished",
            V2AuditEventKind::StepStarted { .. } => "step.started",
            V2AuditEventKind::StepFinished { .. } => "step.finished",
            V2AuditEventKind::StepSkipped { .. } => "step.skipped",
            V2AuditEventKind::StepRetry { .. } => "step.retry",
            V2AuditEventKind::StepRecoveryAttempted { .. } => "step.recovery_attempted",
            V2AuditEventKind::StepDenied { .. } => V2_EVENT_TYPE_STEP_DENIED,
            V2AuditEventKind::StepJoin { .. } => "step.join",
            V2AuditEventKind::FanoutDispatched { .. } => "fanout.dispatched",
            V2AuditEventKind::WorkerState { .. } => "worker.state",
            V2AuditEventKind::FaninJoined { .. } => "fanin.joined",
            V2AuditEventKind::LoopIterationStart { .. } => "loop.iteration.start",
            V2AuditEventKind::LoopIterationEnd { .. } => "loop.iteration.end",
            V2AuditEventKind::LoopDidNotConverge { .. } => "loop.did_not_converge",
            V2AuditEventKind::ActivityStarted { .. } => "activity.started",
            V2AuditEventKind::ActivityFinished { .. } => "activity.finished",
            V2AuditEventKind::FsCallRequest { .. } => "fs.call.request",
            V2AuditEventKind::FsCallResult { .. } => "fs.call.result",
            V2AuditEventKind::FsCallDenied { .. } => V2_EVENT_TYPE_FS_CALL_DENIED,
            V2AuditEventKind::ToolDenied { .. } => V2_EVENT_TYPE_TOOL_DENIED,
            V2AuditEventKind::ToolAllowlistHarnessDelegated { .. } => {
                "tool_allowlist.harness_delegated"
            }
            V2AuditEventKind::CliInvocationStarted { .. } => "cli.invocation.started",
            V2AuditEventKind::CliInvocationFinished { .. } => "cli.invocation.finished",
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct BranchOutcome {
    pub branch_id: String,
    pub outcome: String,
}
