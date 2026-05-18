//! Job catalog and job-run listing handlers.

use std::sync::Arc;

use axum::extract::{Query, State};
use axum::response::{IntoResponse, Json, Response};
use orbit_core::OrbitRuntime;
use orbit_core::command::job::JobRunListParams;
use serde_json::Value;

use super::{LimitQuery, bounded_limit, server_error};
use crate::projections::{job_catalog_to_json_with_last_run, job_run_to_json};

const JOB_RUN_DEFAULT_LIMIT: usize = 25;

pub(super) async fn list_jobs(State(runtime): State<Arc<OrbitRuntime>>) -> Response {
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

pub(super) async fn list_job_runs(
    State(runtime): State<Arc<OrbitRuntime>>,
    Query(q): Query<LimitQuery>,
) -> Response {
    let limit = bounded_limit(q.limit, JOB_RUN_DEFAULT_LIMIT);
    let params = JobRunListParams {
        job_id: None,
        state: None,
        since: None,
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
