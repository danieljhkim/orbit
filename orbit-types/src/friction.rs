use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

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
    #[serde(default)]
    pub agent: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
}
