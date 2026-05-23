//! `/metrics/*` parity endpoints for the retired CLI metrics views.

use std::sync::Arc;

use axum::extract::{Path, Query, State};
use axum::response::{IntoResponse, Json, Response};
use chrono::{DateTime, Utc};
use orbit_core::command::job::JobRunListParams;
use orbit_core::{InvocationQuery, OrbitRuntime};
use orbit_knowledge::metrics::aggregate as aggregate_knowledge_stats;
use serde::Deserialize;

use super::{LimitQuery, map_runtime_error, non_empty_string};

const INVOCATIONS_DEFAULT_LIMIT: usize = 20;

#[derive(Debug, Default, Deserialize)]
pub(super) struct MetricsInvocationsQuery {
    #[serde(default)]
    since: Option<String>,
    #[serde(default)]
    until: Option<String>,
    #[serde(default)]
    job_run_id: Option<String>,
    #[serde(default)]
    activity_id: Option<String>,
    #[serde(default)]
    task_id: Option<String>,
    #[serde(default)]
    agent: Option<String>,
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    tool_name: Option<String>,
    #[serde(default)]
    limit: Option<usize>,
}

pub(super) async fn knowledge_metrics(
    State(runtime): State<Arc<OrbitRuntime>>,
    Query(q): Query<LimitQuery>,
) -> Response {
    match runtime.list_job_runs(JobRunListParams {
        limit: q.limit,
        ..Default::default()
    }) {
        Ok(runs) => Json(aggregate_knowledge_stats(&runs)).into_response(),
        Err(e) => map_runtime_error(e),
    }
}

pub(super) async fn activity_metrics(State(runtime): State<Arc<OrbitRuntime>>) -> Response {
    match runtime.activity_invocation_metrics() {
        Ok(rows) => Json(rows).into_response(),
        Err(e) => map_runtime_error(e),
    }
}

pub(super) async fn tool_metrics(State(runtime): State<Arc<OrbitRuntime>>) -> Response {
    match runtime.tool_invocation_metrics() {
        Ok(rows) => Json(rows).into_response(),
        Err(e) => map_runtime_error(e),
    }
}

pub(super) async fn task_metrics(
    State(runtime): State<Arc<OrbitRuntime>>,
    Path(id): Path<String>,
) -> Response {
    match runtime.task_invocation_metrics(&id) {
        Ok(row) => Json(row).into_response(),
        Err(e) => map_runtime_error(e),
    }
}

pub(super) async fn invocation_metrics(
    State(runtime): State<Arc<OrbitRuntime>>,
    Query(q): Query<MetricsInvocationsQuery>,
) -> Response {
    let query = match q.into_invocation_query() {
        Ok(query) => query,
        Err(e) => return map_runtime_error(e),
    };
    match runtime.invocation_records(query) {
        Ok(rows) => Json(rows).into_response(),
        Err(e) => map_runtime_error(e),
    }
}

impl MetricsInvocationsQuery {
    fn into_invocation_query(self) -> Result<InvocationQuery, orbit_core::OrbitError> {
        Ok(InvocationQuery {
            since: parse_rfc3339_opt(self.since, "since")?,
            until: parse_rfc3339_opt(self.until, "until")?,
            job_run_id: optional_query_string(self.job_run_id),
            activity_id: optional_query_string(self.activity_id),
            task_id: optional_query_string(self.task_id),
            agent: optional_query_string(self.agent),
            model: optional_query_string(self.model),
            slot: None,
            tool_name: optional_query_string(self.tool_name),
            limit: self.limit.unwrap_or(INVOCATIONS_DEFAULT_LIMIT),
        })
    }
}

fn parse_rfc3339_opt(
    value: Option<String>,
    field_name: &str,
) -> Result<Option<DateTime<Utc>>, orbit_core::OrbitError> {
    match optional_query_string(value) {
        Some(raw) => DateTime::parse_from_rfc3339(&raw)
            .map(|dt| Some(dt.with_timezone(&Utc)))
            .map_err(|error| {
                orbit_core::OrbitError::InvalidInput(format!("invalid {field_name}: {error}"))
            }),
        None => Ok(None),
    }
}

fn optional_query_string(value: Option<String>) -> Option<String> {
    value.as_deref().and_then(non_empty_string)
}
