use std::collections::HashMap;
use std::fmt::{Display, Formatter};
use std::str::FromStr;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::OrbitId;

pub const fn default_job_max_active_runs() -> u32 {
    1
}

pub const fn default_max_iterations() -> u32 {
    1
}

pub const fn default_retry_backoff_seconds() -> u64 {
    10
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[cfg_attr(feature = "clap", derive(clap::ValueEnum))]
#[serde(rename_all = "snake_case")]
pub enum JobTargetType {
    #[default]
    #[cfg_attr(feature = "clap", value(name = "activity", alias = "activity"))]
    Activity,
    #[cfg_attr(feature = "clap", value(name = "job", alias = "job"))]
    Job,
}

impl Display for JobTargetType {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            JobTargetType::Activity => write!(f, "activity"),
            JobTargetType::Job => write!(f, "job"),
        }
    }
}

impl FromStr for JobTargetType {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "activity" => Ok(JobTargetType::Activity),
            "job" => Ok(JobTargetType::Job),
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
    Skipped,
    /// Transient state emitted while the engine is sleeping between retry attempts.
    Retrying,
    /// Run was explicitly cancelled by the user before it completed.
    Cancelled,
}

/// Events that drive job run state transitions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunEvent {
    Start,
    Complete,
    Fail,
    Timeout,
    Cancel,
    Abandon,
}

impl Display for RunEvent {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            RunEvent::Start => write!(f, "start"),
            RunEvent::Complete => write!(f, "complete"),
            RunEvent::Fail => write!(f, "fail"),
            RunEvent::Timeout => write!(f, "timeout"),
            RunEvent::Cancel => write!(f, "cancel"),
            RunEvent::Abandon => write!(f, "abandon"),
        }
    }
}

impl JobRunState {
    /// Returns true if this state cannot be overwritten by a later finalization.
    pub fn is_terminal(self) -> bool {
        matches!(
            self,
            Self::Success | Self::Failed | Self::Timeout | Self::Cancelled
        )
    }

    /// Validate and compute the next state for a given event.
    pub fn try_transition(self, event: RunEvent) -> Result<JobRunState, String> {
        // Terminal states reject all events
        if self.is_terminal() {
            return Err(format!(
                "invalid job run state transition: {} + {:?} (state is terminal)",
                self, event
            ));
        }

        match (self, event) {
            (Self::Pending, RunEvent::Start) => Ok(Self::Running),
            (Self::Pending, RunEvent::Cancel) => Ok(Self::Cancelled),
            (Self::Running, RunEvent::Complete) => Ok(Self::Success),
            (Self::Running, RunEvent::Fail) => Ok(Self::Failed),
            (Self::Running, RunEvent::Timeout) => Ok(Self::Timeout),
            (Self::Running, RunEvent::Cancel) => Ok(Self::Cancelled),
            (Self::Running, RunEvent::Abandon) => Ok(Self::Failed),
            _ => Err(format!(
                "invalid job run state transition: {} + {:?}",
                self, event
            )),
        }
    }

    /// Validates that a step result state is one of the allowed write-once values.
    pub fn validate_step_state(self) -> Result<(), String> {
        match self {
            Self::Success | Self::Failed | Self::Timeout | Self::Skipped => Ok(()),
            other => Err(format!(
                "invalid step result state: {} (must be success, failed, timeout, or skipped)",
                other
            )),
        }
    }
}

impl Display for JobRunState {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            JobRunState::Pending => write!(f, "pending"),
            JobRunState::Running => write!(f, "running"),
            JobRunState::Success => write!(f, "success"),
            JobRunState::Failed => write!(f, "failed"),
            JobRunState::Timeout => write!(f, "timeout"),
            JobRunState::Skipped => write!(f, "skipped"),
            JobRunState::Retrying => write!(f, "retrying"),
            JobRunState::Cancelled => write!(f, "cancelled"),
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
            "skipped" => Ok(JobRunState::Skipped),
            "retrying" => Ok(JobRunState::Retrying),
            "cancelled" => Ok(JobRunState::Cancelled),
            other => Err(format!("unknown job run state: {other}")),
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[cfg_attr(feature = "clap", derive(clap::ValueEnum))]
#[serde(rename_all = "snake_case")]
pub enum StepCondition {
    #[default]
    Always,
    OnSuccess,
    OnFailure,
    OnTimeout,
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
    #[serde(default)]
    #[serde(rename = "durationMs")]
    pub duration_ms: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AgentCommitRequest {
    pub message: String,
    pub files: Vec<String>,
}

/// A single step within a job definition.
///
/// `Default::default()` matches serde defaults for all fields:
/// - `retry_backoff_seconds` defaults to 10 (matching `#[serde(default = "default_retry_backoff_seconds")]`)
/// - `timeout_seconds` defaults to 0; callers must set this explicitly
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct JobStep {
    pub target_type: JobTargetType,
    pub target_id: OrbitId,
    #[serde(default)]
    pub agent_cli: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    pub timeout_seconds: u64,
    /// Additional env var names to pass through in hermetic mode, on top of the global allowlist.
    #[serde(default)]
    pub env_extra: Vec<String>,
    /// Explicit env var key-value pairs injected into the step's environment.
    /// Unlike `env_extra` (which passes names from the parent env), these set
    /// fixed values regardless of what the parent env contains.  Entries here
    /// override same-named vars from `env_extra` or the global allowlist.
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub env_set: HashMap<String, String>,
    /// Maximum number of total attempts (including the first). Zero means no retry.
    #[serde(default)]
    pub retry_max_attempts: u32,
    /// Initial backoff delay in seconds before the first retry; doubles with each attempt.
    #[serde(default = "default_retry_backoff_seconds")]
    pub retry_backoff_seconds: u64,
    #[serde(default)]
    pub condition: StepCondition,
    /// Rename output keys before merging into the next step's input.
    /// Each entry maps `source_key -> target_key`. Unmapped keys pass through unchanged.
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub output_map: HashMap<String, String>,
}

impl Default for JobStep {
    fn default() -> Self {
        Self {
            target_type: JobTargetType::default(),
            target_id: OrbitId::default(),
            agent_cli: String::new(),
            model: None,
            timeout_seconds: 0,
            env_extra: Vec::new(),
            env_set: HashMap::new(),
            retry_max_attempts: 0,
            retry_backoff_seconds: default_retry_backoff_seconds(),
            condition: StepCondition::Always,
            output_map: HashMap::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Job {
    pub job_id: OrbitId,
    pub state: JobScheduleState,
    #[serde(default)]
    pub default_input: Option<Value>,
    #[serde(default = "default_job_max_active_runs")]
    pub max_active_runs: u32,
    /// Maximum number of times the step sequence is executed. Defaults to 1
    /// (single pass). Values > 1 enable loop semantics: after all steps
    /// complete successfully, the sequence restarts from step 0 until
    /// `max_iterations` is reached or a step outputs `loop_exit: true`.
    #[serde(default = "default_max_iterations")]
    pub max_iterations: u32,
    pub steps: Vec<JobStep>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
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
    /// OS PID of the process that owns this run; set when the run transitions to `running`.
    /// Used to detect abandoned runs when the owning process has died.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pid: Option<u32>,
    /// Process start-time token captured alongside `pid` so reused PIDs are not
    /// mistaken for the original run owner.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pid_start_time: Option<String>,
    /// The original input passed to this run, persisted so retries can reconstruct state.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input: Option<Value>,
    /// When this run is a retry, links back to the source run ID.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retry_source_run_id: Option<String>,
    /// Step execution results; populated in-memory from step files, not stored in jrun.yaml.
    #[serde(skip)]
    pub steps: Vec<JobRunStep>,
}
