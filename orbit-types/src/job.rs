use std::fmt::{Display, Formatter};
use std::str::FromStr;

use chrono::{DateTime, Utc};
use clap::ValueEnum;
use serde::{Deserialize, Serialize};

use crate::{OrbitId, Role};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, ValueEnum)]
#[serde(rename_all = "snake_case")]
pub enum JobScheduleState {
    Active,
    Paused,
    Deleted,
}

impl Display for JobScheduleState {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            JobScheduleState::Active => write!(f, "active"),
            JobScheduleState::Paused => write!(f, "paused"),
            JobScheduleState::Deleted => write!(f, "deleted"),
        }
    }
}

impl FromStr for JobScheduleState {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "active" => Ok(JobScheduleState::Active),
            "paused" => Ok(JobScheduleState::Paused),
            "deleted" => Ok(JobScheduleState::Deleted),
            other => Err(format!("unknown job state: {other}")),
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, ValueEnum)]
#[serde(rename_all = "snake_case")]
pub enum JobSessionStatus {
    Running,
    Succeeded,
    Failed,
    Cancelled,
}

impl Display for JobSessionStatus {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            JobSessionStatus::Running => write!(f, "running"),
            JobSessionStatus::Succeeded => write!(f, "succeeded"),
            JobSessionStatus::Failed => write!(f, "failed"),
            JobSessionStatus::Cancelled => write!(f, "cancelled"),
        }
    }
}

impl FromStr for JobSessionStatus {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "running" => Ok(JobSessionStatus::Running),
            "succeeded" => Ok(JobSessionStatus::Succeeded),
            "failed" => Ok(JobSessionStatus::Failed),
            "cancelled" => Ok(JobSessionStatus::Cancelled),
            other => Err(format!("unknown job session status: {other}")),
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, ValueEnum)]
#[serde(rename_all = "snake_case")]
pub enum JobTrigger {
    Schedule,
    Manual,
}

impl Display for JobTrigger {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            JobTrigger::Schedule => write!(f, "schedule"),
            JobTrigger::Manual => write!(f, "manual"),
        }
    }
}

impl FromStr for JobTrigger {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "schedule" => Ok(JobTrigger::Schedule),
            "manual" => Ok(JobTrigger::Manual),
            other => Err(format!("unknown job trigger: {other}")),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Job {
    pub job_id: OrbitId,
    pub name: String,
    pub task_id: OrbitId,
    pub schedule_spec: String,
    pub timezone: String,
    pub state: JobScheduleState,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub paused_at: Option<DateTime<Utc>>,
    pub deleted_at: Option<DateTime<Utc>>,
    pub last_run_session_id: Option<OrbitId>,
    pub last_run_at: Option<DateTime<Utc>>,
    pub next_run_at: Option<DateTime<Utc>>,
    pub last_error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct JobSession {
    pub session_id: OrbitId,
    pub job_id: OrbitId,
    pub task_id: OrbitId,
    pub trigger: JobTrigger,
    pub trigger_time: DateTime<Utc>,
    pub started_at: Option<DateTime<Utc>>,
    pub finished_at: Option<DateTime<Utc>>,
    pub status: JobSessionStatus,
    pub exit_code: Option<i32>,
    pub error: Option<String>,
    pub composed_context_hash: Option<String>,
    pub effective_allowlist_hash: Option<String>,
    pub created_by_role: Role,
    pub created_at: DateTime<Utc>,
    pub cancel_requested_at: Option<DateTime<Utc>>,
}
