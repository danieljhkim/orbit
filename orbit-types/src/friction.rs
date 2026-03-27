use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::ActorIdentity;

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
