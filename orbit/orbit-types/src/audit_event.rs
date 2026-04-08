use std::fmt::{Display, Formatter};
use std::str::FromStr;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "clap", derive(clap::ValueEnum))]
#[serde(rename_all = "snake_case")]
pub enum AuditEventStatus {
    Success,
    Failure,
    Denied,
}

impl Display for AuditEventStatus {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            AuditEventStatus::Success => write!(f, "success"),
            AuditEventStatus::Failure => write!(f, "failure"),
            AuditEventStatus::Denied => write!(f, "denied"),
        }
    }
}

impl FromStr for AuditEventStatus {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "success" => Ok(AuditEventStatus::Success),
            "failure" => Ok(AuditEventStatus::Failure),
            "denied" => Ok(AuditEventStatus::Denied),
            other => Err(format!("unknown audit event status: {other}")),
        }
    }
}

/// A comprehensive, persistent audit trail record for a CLI command execution.
/// Stored in SQLite and exposed via `orbit audit list` / `orbit audit show`.
/// Captures execution context including timing, exit code, role, tool name, and
/// truncated stdout/stderr for post-hoc review.
///
/// Contrast with [`orbit_types::Audit`], which is the lightweight in-memory
/// event log entry produced by the runtime for internal observability.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AuditEvent {
    pub id: i64,
    pub execution_id: String,
    pub timestamp: DateTime<Utc>,
    pub command: String,
    pub subcommand: Option<String>,
    pub tool_name: Option<String>,
    pub target_type: Option<String>,
    pub target_id: Option<String>,
    pub role: String,
    pub status: AuditEventStatus,
    pub exit_code: i32,
    pub duration_ms: i64,
    pub working_directory: String,
    pub arguments_json: Option<String>,
    pub stdout_truncated: Option<String>,
    pub stderr_truncated: Option<String>,
    pub error_message: Option<String>,
    pub host: Option<String>,
    pub pid: u32,
    pub session_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AuditStats {
    pub total: i64,
    pub success_count: i64,
    pub failure_count: i64,
    pub denied_count: i64,
    pub avg_duration_ms: f64,
    pub p95_duration_ms: i64,
    pub max_duration_ms: i64,
}
