use chrono::{DateTime, Utc};
use orbit_common::types::JobRunState;
use serde::Serialize;

/// Parameters for filtering and paging job run listings.
#[derive(Debug, Clone, Default)]
pub struct JobRunListParams {
    pub job_id: Option<String>,
    pub state: Option<JobRunState>,
    pub since: Option<DateTime<Utc>>,
    pub limit: Option<usize>,
}

/// Result of a job run cancellation attempt.
#[derive(Debug, Clone, Serialize)]
pub struct JobRunCancelResult {
    pub run_id: String,
    pub previous_state: String,
    pub final_state: String,
    pub actor: String,
    pub source: String,
    pub signal_attempted: bool,
    pub signal_outcome: Option<String>,
}
