//! Job catalog and job-run listing handlers.

use crate::state::Ws;
use axum::extract::{Path, Query};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Json, Response};
use chrono::{DateTime, Utc};
use orbit_core::JobRunState;
use orbit_core::command::job::JobRunListParams;
use serde::Deserialize;
use serde_json::{Value, json};

use super::{bad_request, bounded_limit, map_runtime_error, server_error, validate_id};
use crate::projections::{job_catalog_to_json_with_last_run, job_run_to_json};

const JOB_RUN_DEFAULT_LIMIT: usize = 25;

#[derive(Deserialize, Default)]
pub(super) struct JobRunListQuery {
    #[serde(default)]
    limit: Option<usize>,
    #[serde(default)]
    job_id: Option<String>,
    #[serde(default)]
    state: Option<String>,
    #[serde(default)]
    since: Option<DateTime<Utc>>,
}

enum JobRunListState {
    Concrete(JobRunState),
    Terminal,
}

impl JobRunListState {
    fn parse(value: &str) -> Result<Self, String> {
        match value {
            "pending" => Ok(Self::Concrete(JobRunState::Pending)),
            "running" => Ok(Self::Concrete(JobRunState::Running)),
            "terminal" => Ok(Self::Terminal),
            _ => Err("invalid state; expected one of: pending, running, terminal".to_string()),
        }
    }
}

pub(super) async fn list_jobs(Ws(runtime): Ws) -> Response {
    use orbit_core::command::job::JobCatalogFilter;
    match runtime.list_job_catalog_with_last_run(true, JobCatalogFilter::All) {
        Ok(rows) => {
            let values: Vec<Value> = rows
                .iter()
                .map(|(entry, last_run)| {
                    job_catalog_to_json_with_last_run(entry, last_run.as_ref())
                })
                .collect();
            Json(Value::Array(values)).into_response()
        }
        Err(e) => server_error(e),
    }
}

pub(super) async fn list_job_runs(Ws(runtime): Ws, Query(q): Query<JobRunListQuery>) -> Response {
    let limit = bounded_limit(q.limit, JOB_RUN_DEFAULT_LIMIT);
    let state = match q.state.as_deref().map(JobRunListState::parse).transpose() {
        Ok(state) => state,
        Err(message) => return bad_request(message),
    };
    let params = JobRunListParams {
        job_id: q.job_id,
        state: state.as_ref().and_then(|state| match state {
            JobRunListState::Concrete(state) => Some(*state),
            JobRunListState::Terminal => None,
        }),
        terminal_only: matches!(state, Some(JobRunListState::Terminal)),
        since: q.since,
        limit: Some(limit),
    };
    match runtime.list_job_runs(params) {
        Ok(runs) => {
            let values: Vec<Value> = runs.iter().map(job_run_to_json).collect();
            Json(Value::Array(values)).into_response()
        }
        Err(e) => server_error(e),
    }
}

/// Resume a terminal resumable run as a new linked run.
///
/// Resume re-runs the first non-successful step and every subsequent step; it
/// succeeds only when the underlying cause of the source failure is resolved.
pub(super) async fn resume_job_run_action(Ws(runtime): Ws, Path(id): Path<String>) -> Response {
    let id = match validate_id(&id) {
        Ok(id) => id,
        Err(message) => return bad_request(message),
    };
    match runtime.resume_job_run(id) {
        Ok(result) => match runtime.show_job_run(&result.run_id) {
            Ok(run) => Json(job_run_to_json(&run)).into_response(),
            Err(e) => map_runtime_error(e),
        },
        Err(orbit_core::OrbitError::JobValidation(message)) => {
            (StatusCode::CONFLICT, Json(json!({ "error": message }))).into_response()
        }
        Err(e) => map_runtime_error(e),
    }
}
