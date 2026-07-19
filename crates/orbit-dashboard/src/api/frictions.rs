//! Friction artifact scan and triage handlers.

use crate::state::Ws;
use axum::extract::rejection::JsonRejection;
use axum::extract::{Path, Query};
use axum::response::{IntoResponse, Json, Response};
use orbit_core::{OrbitError, OrbitRuntime};
use serde::Deserialize;
use serde_json::{Map, Value, json};

use super::{bad_request, bounded_limit, map_runtime_error, non_empty_string};

const FRICTIONS_DEFAULT_LIMIT: usize = 100;
const HUMAN_ACTOR_LABEL: &str = "human";

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

/// Create body for `POST /frictions`. Mirrors the `orbit.friction.add` tool
/// schema: `body` is required, `model` provenance and `tags`/`during_task`
/// are optional. Field parsing and validation stay in the shared runtime add
/// path so the wire shape and defaults match the native MCP tool exactly.
/// `model` falls back to the human actor label via [`run_friction_tool`] when
/// the caller omits it, matching the ADR routes. `orbit.friction.add` rejects
/// a separate `agent` field (attribution was consolidated to `model`-only),
/// so this body does not accept one.
#[derive(Deserialize, Default)]
pub(super) struct CreateFrictionBody {
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    body: Option<String>,
    #[serde(default)]
    tags: Option<Vec<String>>,
    #[serde(default)]
    during_task: Option<String>,
}

#[derive(Deserialize, Default)]
pub(super) struct FrictionPatchBody {
    #[serde(default)]
    status: Option<String>,
    #[serde(default)]
    tags: Option<Vec<String>>,
}

pub(super) async fn list_frictions(
    Ws(runtime): Ws,
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

/// `POST /frictions` — file a new friction, mirroring `orbit.friction.add`.
/// The request body is passed through the shared runtime add path so validation
/// and defaults stay identical to the native MCP tool; the created friction JSON
/// (same projection the list/get handlers emit) is returned on success.
/// Malformed JSON is rejected with a structured 400; deeper validation errors
/// (missing/empty `body`, bad fields) surface through [`map_runtime_error`] as
/// 4xx carrying Orbit's own error text.
pub(super) async fn create_friction_action(
    Ws(runtime): Ws,
    body: Result<Json<CreateFrictionBody>, JsonRejection>,
) -> Response {
    let Json(body) = match body {
        Ok(body) => body,
        Err(rejection) => {
            return bad_request(format!("malformed friction create payload: {rejection}"));
        }
    };

    let mut input = Map::new();
    if let Some(model) = body.model.as_deref().and_then(non_empty_string) {
        input.insert("model".to_string(), Value::String(model));
    }
    if let Some(friction_body) = body.body {
        input.insert("body".to_string(), Value::String(friction_body));
    }
    if let Some(tags) = body.tags {
        input.insert("tags".to_string(), json!(tags));
    }
    if let Some(during_task) = body.during_task.as_deref().and_then(non_empty_string) {
        input.insert("during_task".to_string(), Value::String(during_task));
    }

    match run_friction_tool(&runtime, "orbit.friction.add", Value::Object(input)) {
        Ok(friction) => Json(friction).into_response(),
        Err(e) => map_runtime_error(e),
    }
}

pub(super) async fn get_friction(Ws(runtime): Ws, Path(id): Path<String>) -> Response {
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

pub(super) async fn friction_stats(Ws(runtime): Ws) -> Response {
    match run_friction_tool(&runtime, "orbit.friction.stats", json!({})) {
        Ok(stats) => Json(stats).into_response(),
        Err(e) => map_runtime_error(e),
    }
}

pub(super) async fn update_friction_action(
    Ws(runtime): Ws,
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

pub(super) async fn resolve_friction_action(Ws(runtime): Ws, Path(id): Path<String>) -> Response {
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

/// `orbit.friction.add` requires a non-empty `model`; when the caller supplies
/// no attribution, default to the human actor label rather than a model
/// constant. No other friction tool consumes `model`, so this only touches
/// `orbit.friction.add`.
fn run_friction_tool(
    runtime: &OrbitRuntime,
    name: &str,
    mut input: Value,
) -> Result<Value, OrbitError> {
    if name == "orbit.friction.add"
        && let Some(object) = input.as_object_mut()
    {
        object
            .entry("model".to_string())
            .or_insert_with(|| Value::String(HUMAN_ACTOR_LABEL.to_string()));
    }
    runtime.run_tool(name, input)
}
