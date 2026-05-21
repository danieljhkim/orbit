use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Schema version for the §7 v2 audit envelope. Per §12 Q10 resolution,
/// versioning is PER EVENT TYPE — each variant of `V2AuditEventKind` can be
/// versioned independently. This constant is the envelope schema itself.
pub const AUDIT_ENVELOPE_SCHEMA_VERSION: u32 = 1;

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
    /// Populated by CLI entry points so file-backed audit traces under
    /// `.orbit/state/audit/v2_loop/*.jsonl` can be filtered by origin repo.
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

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct BranchOutcome {
    pub branch_id: String,
    pub outcome: String,
}
