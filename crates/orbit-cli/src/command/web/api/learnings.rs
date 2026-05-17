//! Learning scan and curation handlers.

use std::sync::Arc;

use axum::extract::{Path, Query, State};
use axum::response::{IntoResponse, Json, Response};
use orbit_core::{Learning, LearningSearchParams, OrbitRuntime};
use serde::Deserialize;
use serde_json::{Value, json};

use super::{bad_request, bounded_limit, map_runtime_error, non_empty_string, server_error};
use crate::command::learning::output::learning_to_json;

const LEARNINGS_DEFAULT_LIMIT: usize = 100;

#[derive(Deserialize, Default)]
pub(super) struct LearningsQuery {
    #[serde(default)]
    q: Option<String>,
    #[serde(default)]
    scope: Option<String>,
    #[serde(default)]
    tag: Option<String>,
    #[serde(default)]
    limit: Option<usize>,
    #[serde(default)]
    offset: Option<usize>,
}

#[derive(Deserialize, Default)]
pub(super) struct SupersedeLearningBody {
    #[serde(default)]
    by: Option<String>,
    #[serde(default)]
    reason: Option<String>,
}

pub(super) async fn list_learnings(
    State(runtime): State<Arc<OrbitRuntime>>,
    Query(query): Query<LearningsQuery>,
) -> Response {
    let all = match runtime.list_learnings(None) {
        Ok(learnings) => learnings,
        Err(e) => return server_error(e),
    };

    let stats = learning_stats_to_json(&all);
    let limit = bounded_limit(query.limit, LEARNINGS_DEFAULT_LIMIT);
    let offset = query.offset.unwrap_or(0);
    let q = query.q.as_deref().and_then(non_empty_string);
    let scope = query.scope.as_deref().and_then(non_empty_string);
    let tag = query.tag.as_deref().and_then(non_empty_string);

    let rows = if q.is_some() || scope.is_some() || tag.is_some() {
        let search_limit = offset.saturating_add(limit);
        let results = match runtime.search_learnings(LearningSearchParams {
            path: scope,
            tag,
            query: q,
            limit: Some(search_limit),
        }) {
            Ok(results) => results,
            Err(e) => return map_runtime_error(e),
        };
        let mut rows = Vec::with_capacity(results.len());
        for result in results {
            match runtime.get_learning(&result.learning.id) {
                Ok(learning) => rows.push(learning),
                Err(e) => return map_runtime_error(e),
            }
        }
        rows
    } else {
        all.clone()
    };

    let items = rows
        .iter()
        .skip(offset)
        .take(limit)
        .map(learning_to_json)
        .collect::<Vec<_>>();

    Json(json!({
        "stats": stats,
        "items": items,
    }))
    .into_response()
}

pub(super) async fn get_learning(
    State(runtime): State<Arc<OrbitRuntime>>,
    Path(id): Path<String>,
) -> Response {
    match runtime.get_learning(&id) {
        Ok(learning) => Json(learning_to_json(&learning)).into_response(),
        Err(e) => map_runtime_error(e),
    }
}

pub(super) async fn supersede_learning_action(
    State(runtime): State<Arc<OrbitRuntime>>,
    Path(id): Path<String>,
    body: Option<Json<SupersedeLearningBody>>,
) -> Response {
    let Some(Json(body)) = body else {
        return bad_request("request body must include `by`".to_string());
    };
    let Some(by) = body.by.as_deref().and_then(non_empty_string) else {
        return bad_request("request body must include non-empty `by`".to_string());
    };
    let _reason = body.reason.as_deref().and_then(non_empty_string);

    match runtime.supersede_learning(&id, &by) {
        Ok(()) => {
            let old = match runtime.get_learning(&id) {
                Ok(learning) => learning,
                Err(e) => return map_runtime_error(e),
            };
            let new = match runtime.get_learning(&by) {
                Ok(learning) => learning,
                Err(e) => return map_runtime_error(e),
            };
            Json(json!({
                "old": learning_to_json(&old),
                "new": learning_to_json(&new),
            }))
            .into_response()
        }
        Err(e) => map_runtime_error(e),
    }
}

fn learning_stats_to_json(learnings: &[Learning]) -> Value {
    let superseded = learnings
        .iter()
        .filter(|learning| learning.status.as_str() == "superseded")
        .count();
    let last_indexed = learnings
        .iter()
        .map(|learning| learning.updated_at)
        .max()
        .map(|ts| ts.to_rfc3339());

    json!({
        "total": learnings.len(),
        "superseded": superseded,
        "last_indexed": last_indexed,
    })
}

#[cfg(test)]
#[path = "learnings_tests.rs"]
mod tests;
