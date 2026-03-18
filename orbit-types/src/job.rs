use std::fmt::{Display, Formatter};
use std::str::FromStr;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::OrbitId;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "clap", derive(clap::ValueEnum))]
#[serde(rename_all = "snake_case")]
pub enum JobTargetType {
    #[cfg_attr(feature = "clap", value(name = "activity", alias = "activity"))]
    Activity,
}

impl Display for JobTargetType {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            JobTargetType::Activity => write!(f, "activity"),
        }
    }
}

impl FromStr for JobTargetType {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "activity" => Ok(JobTargetType::Activity),
            other => Err(format!("unknown job target type: {other}")),
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "clap", derive(clap::ValueEnum))]
#[serde(rename_all = "snake_case")]
pub enum JobScheduleState {
    Enabled,
    Disabled,
}

impl Display for JobScheduleState {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            JobScheduleState::Enabled => write!(f, "enabled"),
            JobScheduleState::Disabled => write!(f, "disabled"),
        }
    }
}

impl FromStr for JobScheduleState {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "enabled" | "active" => Ok(JobScheduleState::Enabled),
            "disabled" | "deleted" | "paused" => Ok(JobScheduleState::Disabled),
            other => Err(format!("unknown job state: {other}")),
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "clap", derive(clap::ValueEnum))]
#[serde(rename_all = "snake_case")]
pub enum JobRunState {
    Pending,
    Running,
    Success,
    Failed,
    Timeout,
}

impl Display for JobRunState {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            JobRunState::Pending => write!(f, "pending"),
            JobRunState::Running => write!(f, "running"),
            JobRunState::Success => write!(f, "success"),
            JobRunState::Failed => write!(f, "failed"),
            JobRunState::Timeout => write!(f, "timeout"),
        }
    }
}

impl FromStr for JobRunState {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "pending" => Ok(JobRunState::Pending),
            "running" => Ok(JobRunState::Running),
            "success" => Ok(JobRunState::Success),
            "failed" => Ok(JobRunState::Failed),
            "timeout" => Ok(JobRunState::Timeout),
            other => Err(format!("unknown job run state: {other}")),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AgentRunError {
    pub code: String,
    pub message: String,
    #[serde(default)]
    pub details: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AgentResponseEnvelope {
    #[serde(rename = "schemaVersion")]
    pub schema_version: u32,
    pub status: String,
    #[serde(default)]
    pub result: Option<Value>,
    #[serde(default)]
    pub error: Option<AgentRunError>,
    #[serde(rename = "durationMs")]
    pub duration_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AgentCommitRequest {
    pub message: String,
    pub files: Vec<String>,
}

/// Precondition check for a job step.
/// If the command exits non-zero and `skip_job_on_failure` is true, the pipeline
/// stops cleanly with success state. If false, the step is treated as a failure.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct JobStepPrecondition {
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    pub skip_job_on_failure: bool,
}

/// A single step within a job definition.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct JobStep {
    pub target_type: JobTargetType,
    pub target_id: OrbitId,
    #[serde(default)]
    pub agent_cli: String,
    pub timeout_seconds: u64,
    /// Additional env var names to pass through in hermetic mode, on top of the global allowlist.
    #[serde(default)]
    pub env_extra: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub precondition: Option<JobStepPrecondition>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Job {
    pub job_id: OrbitId,
    pub state: JobScheduleState,
    #[serde(default)]
    pub default_input: Option<Value>,
    pub steps: Vec<JobStep>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn job_step_precondition_round_trips_json() {
        let step = JobStep {
            target_type: JobTargetType::Activity,
            target_id: "dispatch_task".to_string(),
            agent_cli: "claude".to_string(),
            timeout_seconds: 1000,
            env_extra: vec![],
            precondition: Some(JobStepPrecondition {
                command: "orbit".to_string(),
                args: vec![
                    "activity".to_string(),
                    "run".to_string(),
                    "check_dispatch_needed".to_string(),
                ],
                skip_job_on_failure: true,
            }),
        };
        let json = serde_json::to_string(&step).expect("serialize");
        let back: JobStep = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(step, back);
        let precondition = back.precondition.expect("has precondition");
        assert_eq!(precondition.command, "orbit");
        assert_eq!(
            precondition.args,
            vec!["activity", "run", "check_dispatch_needed"]
        );
        assert!(precondition.skip_job_on_failure);
    }

    #[test]
    fn job_step_without_precondition_omits_field_in_json() {
        let step = JobStep {
            target_type: JobTargetType::Activity,
            target_id: "some_step".to_string(),
            agent_cli: String::new(),
            timeout_seconds: 60,
            env_extra: vec![],
            precondition: None,
        };
        let json = serde_json::to_string(&step).expect("serialize");
        assert!(!json.contains("precondition"), "precondition must be omitted when None");
    }
}

/// Per-step execution record stored in a step file inside the run bundle directory.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct JobRunStep {
    pub step_index: u32,
    pub target_type: JobTargetType,
    pub target_id: OrbitId,
    pub started_at: Option<DateTime<Utc>>,
    pub finished_at: Option<DateTime<Utc>>,
    pub duration_ms: Option<u64>,
    pub exit_code: Option<i32>,
    pub agent_response_json: Option<Value>,
    pub state: JobRunState,
    pub error_code: Option<String>,
    pub error_message: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct JobRun {
    pub run_id: OrbitId,
    pub job_id: OrbitId,
    pub attempt: u32,
    pub state: JobRunState,
    pub scheduled_at: DateTime<Utc>,
    pub started_at: Option<DateTime<Utc>>,
    pub finished_at: Option<DateTime<Utc>>,
    pub duration_ms: Option<u64>,
    pub created_at: DateTime<Utc>,
    /// Step execution results; populated in-memory from step files, not stored in jrun.yaml.
    #[serde(skip)]
    pub steps: Vec<JobRunStep>,
}
