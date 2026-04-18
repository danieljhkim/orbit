use chrono::{DateTime, Utc};
use serde::Serialize;

/// Schema version for the §7 v2 audit envelope. Per §12 Q10 resolution,
/// versioning is PER EVENT TYPE — each variant of `V2AuditEventKind` can be
/// versioned independently. This constant is the envelope schema itself.
pub const AUDIT_ENVELOPE_SCHEMA_VERSION: u32 = 1;

/// Common envelope fields wrapping every v2 audit event (§7).
#[derive(Debug, Clone, Serialize)]
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
}

/// §7 v2 audit event — the envelope plus a type-specific body.
#[derive(Debug, Clone, Serialize)]
pub struct V2AuditEvent {
    #[serde(flatten)]
    pub envelope: V2AuditEnvelope,
    #[serde(flatten)]
    pub kind: V2AuditEventKind,
}

/// Event-type discriminator (§7). The v2 layer emits run.*, step.*,
/// activity.*, and tool.denied events. Loop-level http.* and tool.call.*
/// events continue to be emitted by the loop engine and are referenced via
/// `parent_event_id` from Activity events.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "body_kind", rename_all = "snake_case")]
pub enum V2AuditEventKind {
    RunStarted {
        job_name: String,
    },
    RunFinished {
        outcome: String,
    },
    StepStarted {
        step_id: String,
    },
    StepFinished {
        step_id: String,
        outcome: String,
    },
    ActivityStarted {
        activity_name: String,
        activity_type: String,
    },
    ActivityFinished {
        activity_name: String,
        outcome: String,
    },
    ToolDenied {
        tool_name: String,
        reason: String,
    },
}
