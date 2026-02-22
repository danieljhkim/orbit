use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fmt::{Display, Formatter};
use thiserror::Error;

pub type OrbitId = String;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Task {
    pub id: OrbitId,
    pub title: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Memo {
    pub id: OrbitId,
    pub body: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum JobStatus {
    Scheduled,
    Running,
    Complete,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Job {
    pub id: OrbitId,
    pub name: String,
    pub command: String,
    pub next_run_at: DateTime<Utc>,
    pub last_run_at: Option<DateTime<Utc>>,
    pub status: JobStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Watch {
    pub id: OrbitId,
    pub path: String,
    pub command: String,
    pub debounce_ms: u64,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Audit {
    pub id: i64,
    pub event_type: String,
    pub payload: Value,
    pub message: String,
    pub created_at: DateTime<Utc>,
}

impl Display for Task {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}\t{}\t{}",
            self.id,
            self.created_at.to_rfc3339(),
            self.title
        )
    }
}

impl Display for Audit {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}\t{}\t{}\t{}",
            self.id,
            self.created_at.to_rfc3339(),
            self.event_type,
            self.message
        )
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", content = "data")]
pub enum OrbitEvent {
    ToolExecuted { name: String },
    JobStarted { id: String },
    JobCompleted { id: String, success: bool },
    WatchTriggered { path: String },
    PolicyDenied { tool: String },
    TaskAdded { id: String },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ToolSchema {
    pub name: String,
    pub description: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ExecutionResult {
    pub success: bool,
    pub stdout: String,
    pub stderr: String,
    pub exit_code: Option<i32>,
    pub duration_ms: u64,
    pub output: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum PolicyDecision {
    Allow,
    Deny { reason: String },
}

#[derive(Debug, Error)]
pub enum OrbitError {
    #[error("policy denied: {0}")]
    PolicyDenied(String),
    #[error("tool not found: {0}")]
    ToolNotFound(String),
    #[error("invalid input: {0}")]
    InvalidInput(String),
    #[error("execution failed: {0}")]
    Execution(String),
    #[error("store error: {0}")]
    Store(String),
    #[error("io error: {0}")]
    Io(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn orbit_event_shape_is_stable() {
        let event = OrbitEvent::ToolExecuted {
            name: "fs.read".to_string(),
        };
        let json = serde_json::to_value(event).expect("serialize event");

        assert_eq!(json["type"], "ToolExecuted");
        assert_eq!(json["data"]["name"], "fs.read");
    }

    #[test]
    fn execution_result_round_trips() {
        let result = ExecutionResult {
            success: true,
            stdout: "ok".to_string(),
            stderr: String::new(),
            exit_code: Some(0),
            duration_ms: 12,
            output: Some(serde_json::json!({"k": "v"})),
        };

        let json = serde_json::to_string(&result).expect("serialize result");
        let decoded: ExecutionResult = serde_json::from_str(&json).expect("deserialize result");

        assert_eq!(decoded, result);
    }
}
