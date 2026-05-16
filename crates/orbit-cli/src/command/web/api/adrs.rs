//! ADR scan and lifecycle handlers.

use std::fs;
use std::io::ErrorKind;
use std::sync::Arc;

use axum::extract::{Path, Query, State};
use axum::response::{IntoResponse, Json, Response};
use orbit_core::{OrbitError, OrbitRuntime};
use serde::Deserialize;
use serde_json::{Map, Value, json};

use super::{bad_request, bounded_limit, map_runtime_error, non_empty_string};

const ADRS_DEFAULT_LIMIT: usize = 100;
const ADR_TOOL_MODEL: &str = "gpt-5.5";

#[derive(Deserialize, Default)]
pub(super) struct AdrsQuery {
    #[serde(default)]
    status: Option<String>,
    #[serde(default)]
    feature: Option<String>,
    #[serde(default)]
    q: Option<String>,
    #[serde(default)]
    limit: Option<usize>,
    #[serde(default)]
    offset: Option<usize>,
}

#[derive(Deserialize, Default)]
pub(super) struct SupersedeAdrBody {
    #[serde(default)]
    by: Option<String>,
    #[serde(default)]
    reason: Option<String>,
}

pub(super) async fn list_adrs(
    State(runtime): State<Arc<OrbitRuntime>>,
    Query(query): Query<AdrsQuery>,
) -> Response {
    let all = match adr_list(&runtime, Map::new()) {
        Ok(adrs) => adrs,
        Err(e) => return map_runtime_error(e),
    };
    let stats = adr_stats_to_json(&all);

    let mut input = Map::new();
    if let Some(status) = query.status.as_deref().and_then(non_empty_string) {
        input.insert("status".to_string(), Value::String(status));
    }
    if let Some(feature) = query.feature.as_deref().and_then(non_empty_string) {
        input.insert("feature".to_string(), Value::String(feature));
    }

    let mut rows = if input.is_empty() {
        all.clone()
    } else {
        match adr_list(&runtime, input) {
            Ok(adrs) => adrs,
            Err(e) => return map_runtime_error(e),
        }
    };

    if let Some(q) = query.q.as_deref().and_then(non_empty_string) {
        rows.retain(|adr| adr_matches_query(adr, &q));
    }

    let limit = bounded_limit(query.limit, ADRS_DEFAULT_LIMIT);
    let offset = query.offset.unwrap_or(0);
    let mut items = rows
        .into_iter()
        .skip(offset)
        .take(limit)
        .collect::<Vec<_>>();
    for adr in &mut items {
        if let Err(e) = attach_body(&runtime, adr) {
            return map_runtime_error(e);
        }
    }

    Json(json!({
        "stats": stats,
        "items": items,
    }))
    .into_response()
}

pub(super) async fn get_adr(
    State(runtime): State<Arc<OrbitRuntime>>,
    Path(id): Path<String>,
) -> Response {
    match adr_show(&runtime, &id) {
        Ok(adr) => Json(adr).into_response(),
        Err(e) => map_runtime_error(e),
    }
}

pub(super) async fn accept_adr_action(
    State(runtime): State<Arc<OrbitRuntime>>,
    Path(id): Path<String>,
) -> Response {
    let result = run_adr_tool(
        &runtime,
        "orbit.adr.update",
        json!({
            "id": id,
            "status": "accepted",
        }),
    );
    match result {
        Ok(mut adr) => match attach_body(&runtime, &mut adr) {
            Ok(()) => Json(adr).into_response(),
            Err(e) => map_runtime_error(e),
        },
        Err(e) => map_runtime_error(e),
    }
}

pub(super) async fn supersede_adr_action(
    State(runtime): State<Arc<OrbitRuntime>>,
    Path(id): Path<String>,
    body: Option<Json<SupersedeAdrBody>>,
) -> Response {
    let Some(Json(body)) = body else {
        return bad_request("request body must include `by`".to_string());
    };
    let Some(by) = body.by.as_deref().and_then(non_empty_string) else {
        return bad_request("request body must include non-empty `by`".to_string());
    };
    let _reason = body.reason.as_deref().and_then(non_empty_string);

    let result = run_adr_tool(
        &runtime,
        "orbit.adr.supersede",
        json!({
            "old_id": id,
            "new_id": by,
        }),
    );

    match result {
        Ok(mut old) => {
            if let Err(e) = attach_body(&runtime, &mut old) {
                return map_runtime_error(e);
            }
            let new_id = old
                .get("superseded_by")
                .and_then(Value::as_str)
                .unwrap_or(by.as_str());
            let new = match adr_show(&runtime, new_id) {
                Ok(adr) => adr,
                Err(e) => return map_runtime_error(e),
            };
            Json(json!({
                "old": old,
                "new": new,
            }))
            .into_response()
        }
        Err(e) => map_runtime_error(e),
    }
}

fn adr_list(runtime: &OrbitRuntime, input: Map<String, Value>) -> Result<Vec<Value>, OrbitError> {
    let value = run_adr_tool(runtime, "orbit.adr.list", Value::Object(input))?;
    match value {
        Value::Array(adrs) => Ok(adrs),
        other => Err(OrbitError::Execution(format!(
            "orbit.adr.list returned non-array JSON: {other}"
        ))),
    }
}

fn adr_show(runtime: &OrbitRuntime, id: &str) -> Result<Value, OrbitError> {
    let mut adr = run_adr_tool(runtime, "orbit.adr.show", json!({ "id": id }))?;
    attach_body(runtime, &mut adr)?;
    Ok(adr)
}

fn run_adr_tool(runtime: &OrbitRuntime, name: &str, mut input: Value) -> Result<Value, OrbitError> {
    if let Some(object) = input.as_object_mut() {
        object
            .entry("model".to_string())
            .or_insert_with(|| Value::String(ADR_TOOL_MODEL.to_string()));
    }
    runtime.run_tool(name, input)
}

fn attach_body(runtime: &OrbitRuntime, adr: &mut Value) -> Result<(), OrbitError> {
    let id = adr.get("id").and_then(Value::as_str).unwrap_or_default();
    let status = adr
        .get("status")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let body_path = runtime
        .data_root()
        .join("adrs")
        .join(status)
        .join(id)
        .join("body.md");
    let body = match fs::read_to_string(&body_path) {
        Ok(body) => body,
        Err(e) if e.kind() == ErrorKind::NotFound => String::new(),
        Err(e) => {
            return Err(OrbitError::Io(format!("read ADR body for {id}: {}", e)));
        }
    };
    if let Some(object) = adr.as_object_mut() {
        object.insert("body".to_string(), Value::String(body));
    }
    Ok(())
}

fn adr_stats_to_json(adrs: &[Value]) -> Value {
    let mut proposed = 0;
    let mut accepted = 0;
    let mut superseded = 0;
    for adr in adrs {
        match adr.get("status").and_then(Value::as_str) {
            Some("proposed") => proposed += 1,
            Some("accepted") => accepted += 1,
            Some("superseded") => superseded += 1,
            _ => {}
        }
    }

    json!({
        "total": adrs.len(),
        "proposed": proposed,
        "accepted": accepted,
        "superseded": superseded,
    })
}

fn adr_matches_query(adr: &Value, query: &str) -> bool {
    let query = query.to_lowercase();
    let fields = ["id", "title", "owner", "status"];
    if fields.iter().any(|field| {
        adr.get(*field)
            .and_then(Value::as_str)
            .is_some_and(|value| value.to_lowercase().contains(&query))
    }) {
        return true;
    }

    ["related_features", "related_tasks", "legacy_ids"]
        .iter()
        .any(|field| {
            adr.get(*field)
                .and_then(Value::as_array)
                .is_some_and(|values| {
                    values.iter().any(|value| {
                        value
                            .as_str()
                            .is_some_and(|value| value.to_lowercase().contains(&query))
                    })
                })
        })
}

#[cfg(test)]
#[path = "adrs_tests.rs"]
mod tests;
