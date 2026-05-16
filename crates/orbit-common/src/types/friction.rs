use std::fmt::{Display, Formatter};
use std::str::FromStr;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::types::ActorIdentity;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum FrictionStatus {
    Open,
    Triaged,
    Resolved,
}

impl Default for FrictionStatus {
    fn default() -> Self {
        Self::Open
    }
}

impl Display for FrictionStatus {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FrictionStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            FrictionStatus::Open => "open",
            FrictionStatus::Triaged => "triaged",
            FrictionStatus::Resolved => "resolved",
        }
    }
}

impl FromStr for FrictionStatus {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "open" => Ok(FrictionStatus::Open),
            "triaged" => Ok(FrictionStatus::Triaged),
            "resolved" => Ok(FrictionStatus::Resolved),
            other => Err(format!("unknown friction status: {other}")),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FrictionRecord {
    pub id: String,
    pub model: String,
    pub created_at: DateTime<Utc>,
    #[serde(default)]
    pub status: FrictionStatus,
    pub tags: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolved_at: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub during_task: Option<String>,
    #[serde(default)]
    pub body: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FrictionFrontmatter {
    pub id: String,
    pub model: String,
    pub created_at: DateTime<Utc>,
    #[serde(default)]
    pub status: FrictionStatus,
    pub tags: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolved_at: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub during_task: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FrictionEntry {
    pub ts: DateTime<Utc>,
    pub job_run: String,
    pub step: String,
    #[serde(default)]
    pub task_id: Option<String>,
    pub command: String,
    pub input: String,
    #[serde(default)]
    pub exit_code: Option<i32>,
    pub stderr: String,
    /// Typed identity of the actor that triggered this friction event.
    #[serde(default)]
    pub actor_identity: ActorIdentity,
}
