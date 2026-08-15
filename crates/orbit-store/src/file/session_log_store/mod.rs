//! Workspace-scoped session-log file store.
//!
//! The log is JSON Lines at `<workspace>/.orbit/session-log.jsonl`. Every read
//! and mutation is serialized through one stable advisory-lock sidecar so ID
//! allocation, append, and resolution observe one ordered record stream.

mod persistence;

#[cfg(test)]
mod tests;

use std::path::PathBuf;

use chrono::{DateTime, Utc};
use orbit_common::types::OrbitError;
use serde::{Deserialize, Serialize};

/// Kind of durable session-log record.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionLogKind {
    Status,
    Note,
    CheckLater,
}

/// One durable session-log record.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SessionLogEntry {
    pub id: String,
    pub at: DateTime<Utc>,
    pub kind: SessionLogKind,
    pub body: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub related_task_ids: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub related_run_ids: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolved_at: Option<DateTime<Utc>>,
}

/// Values required to append one session-log record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionLogAppendParams {
    pub kind: SessionLogKind,
    pub body: String,
    pub related_task_ids: Vec<String>,
    pub related_run_ids: Vec<String>,
}

/// Optional filters applied while listing session-log records.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SessionLogFilter {
    pub kind: Option<SessionLogKind>,
    pub unresolved_only: bool,
    pub since: Option<DateTime<Utc>>,
}

/// File-backed session log rooted in one workspace's `.orbit` directory.
#[derive(Debug, Clone)]
pub struct SessionLogStore {
    orbit_dir: PathBuf,
}

impl SessionLogStore {
    pub fn new(orbit_dir: impl Into<PathBuf>) -> Self {
        Self {
            orbit_dir: orbit_dir.into(),
        }
    }

    /// Append one record and allocate its sequential ID atomically with respect
    /// to every other session-log operation.
    pub fn append(&self, params: SessionLogAppendParams) -> Result<SessionLogEntry, OrbitError> {
        persistence::append(&self.orbit_dir, params)
    }

    /// List records from one consistent, lock-protected snapshot.
    pub fn list(&self, filter: SessionLogFilter) -> Result<Vec<SessionLogEntry>, OrbitError> {
        persistence::list(&self.orbit_dir, &filter)
    }

    /// Mark one `check_later` record resolved using a crash-safe replacement.
    pub fn resolve(&self, id: &str) -> Result<SessionLogEntry, OrbitError> {
        persistence::resolve(&self.orbit_dir, id)
    }
}
