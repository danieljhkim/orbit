//! Friction artifact scan and triage handlers.

use std::sync::Arc;

use axum::extract::{Path, Query, State};
use axum::response::{IntoResponse, Json, Response};
use orbit_core::{OrbitError, OrbitRuntime};
use serde::Deserialize;
use serde_json::{Map, Value, json};

use super::{bad_request, bounded_limit, map_runtime_error, non_empty_string};

const FRICTIONS_DEFAULT_LIMIT: usize = 100;
const FRICTION_TOOL_MODEL: &str = "gpt-5.5";

#[derive(Deserialize, Default)]
pub(super) struct FrictionsQuery {
    #[serde(default)]
    status: Option<String>,
    #[serde(default)]
    tag: Option<String>,
    #[serde(default)]
    month: Option<String>,
    #[serde(default)]
    q: Option<String>,
    #[serde(default)]
    limit: Option<usize>,
    #[serde(default)]
    offset: Option<usize>,
}

#[derive(Deserialize, Default)]
pub(super) struct FrictionPatchBody {
    #[serde(default)]
    status: Option<String>,
    #[serde(default)]
    tags: Option<Vec<String>>,
}

pub(super) async fn list_frictions(
    State(runtime): State<Arc<OrbitRuntime>>,
    Query(query): Query<FrictionsQuery>,
) -> Response {
    let mut input = Map::new();
    insert_optional(&mut input, "status", query.status.as_deref());
    insert_optional(&mut input, "tag", query.tag.as_deref());
    insert_optional(&mut input, "month", query.month.as_deref());
    insert_optional(&mut input, "q", query.q.as_deref());
    input.insert(
        "limit".to_string(),
        Value::from(bounded_limit(query.limit, FRICTIONS_DEFAULT_LIMIT)),
    );
    input.insert("offset".to_string(), Value::from(query.offset.unwrap_or(0)));

    let items = match run_friction_tool(&runtime, "orbit.friction.list", Value::Object(input)) {
        Ok(Value::Array(items)) => items,
        Ok(other) => {
            return map_runtime_error(OrbitError::Execution(format!(
                "orbit.friction.list returned non-array JSON: {other}"
            )));
        }
        Err(e) => return map_runtime_error(e),
    };
    let stats = match run_friction_tool(&runtime, "orbit.friction.stats", json!({})) {
        Ok(stats) => stats,
        Err(e) => return map_runtime_error(e),
    };
    let tags = match run_friction_tool(&runtime, "orbit.friction.tags", json!({})) {
        Ok(tags) => tags,
        Err(e) => return map_runtime_error(e),
    };

    Json(json!({
        "stats": stats,
        "tags": tags,
        "items": items,
    }))
    .into_response()
}

pub(super) async fn get_friction(
    State(runtime): State<Arc<OrbitRuntime>>,
    Path(id): Path<String>,
) -> Response {
    let mut friction = match run_friction_tool(
        &runtime,
        "orbit.friction.show",
        json!({
            "id": id,
        }),
    ) {
        Ok(friction) => friction,
        Err(e) => return map_runtime_error(e),
    };
    let tags = match run_friction_tool(&runtime, "orbit.friction.tags", json!({})) {
        Ok(tags) => tags,
        Err(e) => return map_runtime_error(e),
    };
    if let Some(object) = friction.as_object_mut() {
        object.insert("tag_options".to_string(), tags);
    }
    Json(friction).into_response()
}

pub(super) async fn friction_stats(State(runtime): State<Arc<OrbitRuntime>>) -> Response {
    match run_friction_tool(&runtime, "orbit.friction.stats", json!({})) {
        Ok(stats) => Json(stats).into_response(),
        Err(e) => map_runtime_error(e),
    }
}

pub(super) async fn update_friction_action(
    State(runtime): State<Arc<OrbitRuntime>>,
    Path(id): Path<String>,
    body: Option<Json<FrictionPatchBody>>,
) -> Response {
    let Some(Json(body)) = body else {
        return bad_request("request body must include `status` or `tags`".to_string());
    };
    let status = body.status.as_deref().and_then(non_empty_string);
    if status.is_none() && body.tags.is_none() {
        return bad_request("request body must include `status` or `tags`".to_string());
    }
    let mut input = Map::new();
    input.insert("id".to_string(), Value::String(id));
    if let Some(status) = status {
        input.insert("status".to_string(), Value::String(status));
    }
    if let Some(tags) = body.tags {
        input.insert("tags".to_string(), json!(tags));
    }

    match run_friction_tool(&runtime, "orbit.friction.update", Value::Object(input)) {
        Ok(friction) => Json(friction).into_response(),
        Err(e) => map_runtime_error(e),
    }
}

pub(super) async fn resolve_friction_action(
    State(runtime): State<Arc<OrbitRuntime>>,
    Path(id): Path<String>,
) -> Response {
    match run_friction_tool(
        &runtime,
        "orbit.friction.resolve",
        json!({
            "id": id,
        }),
    ) {
        Ok(friction) => Json(friction).into_response(),
        Err(e) => map_runtime_error(e),
    }
}

fn insert_optional(input: &mut Map<String, Value>, key: &str, value: Option<&str>) {
    if let Some(value) = value.and_then(non_empty_string) {
        input.insert(key.to_string(), Value::String(value));
    }
}

fn run_friction_tool(
    runtime: &OrbitRuntime,
    name: &str,
    mut input: Value,
) -> Result<Value, OrbitError> {
    if name != "orbit.friction.list"
        && let Some(object) = input.as_object_mut()
    {
        object
            .entry("model".to_string())
            .or_insert_with(|| Value::String(FRICTION_TOOL_MODEL.to_string()));
    }
    runtime.run_tool(name, input)
}

#[cfg(test)]
#[path = "frictions_tests.rs"]
mod tests;
