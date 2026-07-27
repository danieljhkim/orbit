//! ADR scan and lifecycle handlers.

use crate::state::Ws;
use axum::extract::rejection::JsonRejection;
use axum::extract::{Path, Query};
use axum::response::{IntoResponse, Json, Response};
use orbit_core::{OrbitError, OrbitRuntime};
use serde::Deserialize;
use serde_json::{Map, Value, json};

use super::{bad_request, bounded_limit, map_runtime_error, non_empty_string};

const ADRS_DEFAULT_LIMIT: usize = 100;
const HUMAN_ACTOR_LABEL: &str = "human";

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

/// Create body for `POST /adrs`. Mirrors the `orbit.adr.add` tool schema:
/// `title` and `body` are required (validated in the handler for a structured
/// 400 rather than serde's default rejection); the remaining fields default to
/// empty. Status is not a field — the tool always creates a Proposed ADR.
/// `model` carries caller-supplied attribution (the canonical agent family,
/// e.g. `codex`/`claude`, or `human`) and is forwarded to the tool host
/// unchanged; when absent, the HTTP route supplies the human actor label so
/// the server process's ambient identity cannot become the actor. `orbit.adr.add`
/// rejects a separate `agent` field (attribution was consolidated to
/// `model`-only), so this body does not accept one.
#[derive(Deserialize, Default)]
pub(super) struct CreateAdrBody {
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    body: Option<String>,
    #[serde(default)]
    owner: Option<String>,
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    related_features: Vec<String>,
    #[serde(default)]
    related_tasks: Vec<String>,
    #[serde(default)]
    tags: Vec<String>,
    #[serde(default)]
    paths: Vec<String>,
}

#[derive(Deserialize, Default)]
pub(super) struct SupersedeAdrBody {
    #[serde(default)]
    by: Option<String>,
    #[serde(default)]
    reason: Option<String>,
    #[serde(default)]
    model: Option<String>,
}

/// Partial-update payload for `PATCH /adrs/:id`, mirroring the mutable fields
/// of the `orbit.adr.update` tool. Status transitions follow the ADR
/// lifecycle rules (e.g. `accepted -> proposed` and direct writes to
/// `superseded` are rejected — supersede has its own route). List fields
/// replace wholesale; an empty list clears, absence leaves unchanged.
#[derive(Deserialize, Default)]
pub(super) struct AdrPatchBody {
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    owner: Option<String>,
    #[serde(default)]
    body: Option<String>,
    #[serde(default)]
    status: Option<String>,
    #[serde(default)]
    related_features: Option<Vec<String>>,
    #[serde(default)]
    related_tasks: Option<Vec<String>>,
    #[serde(default)]
    tags: Option<Vec<String>>,
    #[serde(default)]
    paths: Option<Vec<String>>,
    #[serde(default)]
    supersedes: Option<Vec<String>>,
    #[serde(default)]
    legacy_ids: Option<Vec<String>>,
    #[serde(default)]
    model: Option<String>,
}

pub(super) async fn list_adrs(Ws(runtime): Ws, Query(query): Query<AdrsQuery>) -> Response {
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
        let Some(id) = adr.get("id").and_then(Value::as_str) else {
            return map_runtime_error(OrbitError::Execution(
                "orbit.adr.list returned an ADR without an id".to_string(),
            ));
        };
        *adr = match adr_show(&runtime, id) {
            Ok(resolved) => resolved,
            Err(e) => return map_runtime_error(e),
        };
    }

    Json(json!({
        "stats": stats,
        "items": items,
    }))
    .into_response()
}

pub(super) async fn get_adr(Ws(runtime): Ws, Path(id): Path<String>) -> Response {
    match adr_show(&runtime, &id) {
        Ok(adr) => Json(adr).into_response(),
        Err(e) => map_runtime_error(e),
    }
}

/// `POST /adrs` — record a new Proposed ADR, mirroring `orbit adr` creation
/// semantics (`orbit.adr.add`). The freshly created ADR is returned with its
/// body attached so callers see the same shape `GET /adrs/:id` produces, and it
/// is immediately visible via the existing read surfaces. Malformed payloads
/// (unparseable JSON, or a missing/empty `title` or `body`) are rejected with a
/// structured 400; deeper validation errors from the tool surface through
/// [`map_runtime_error`].
pub(super) async fn create_adr_action(
    Ws(runtime): Ws,
    body: Result<Json<CreateAdrBody>, JsonRejection>,
) -> Response {
    let Json(body) = match body {
        Ok(body) => body,
        Err(rejection) => {
            return bad_request(format!("malformed ADR create payload: {rejection}"));
        }
    };
    let Some(title) = body.title.as_deref().and_then(non_empty_string) else {
        return bad_request("request body must include non-empty `title`".to_string());
    };
    let Some(adr_body) = body.body.as_deref().and_then(non_empty_string) else {
        return bad_request("request body must include non-empty `body`".to_string());
    };

    let mut input = json!({
        "title": title,
        "body": adr_body,
        "model": model_attribution(body.model.as_deref()),
        "related_features": body.related_features,
        "related_tasks": body.related_tasks,
        "tags": body.tags,
        "paths": body.paths,
    });
    if let Some(owner) = body.owner.as_deref().and_then(non_empty_string)
        && let Some(object) = input.as_object_mut()
    {
        object.insert("owner".to_string(), Value::String(owner));
    }
    match runtime.run_tool("orbit.adr.add", input) {
        Ok(adr) => match adr.get("id").and_then(Value::as_str) {
            Some(id) => match adr_show(&runtime, id) {
                Ok(resolved) => Json(resolved).into_response(),
                Err(e) => map_runtime_error(e),
            },
            None => map_runtime_error(OrbitError::Execution(
                "orbit.adr.add returned an ADR without an id".to_string(),
            )),
        },
        Err(e) => map_runtime_error(e),
    }
}

pub(super) async fn accept_adr_action(Ws(runtime): Ws, Path(id): Path<String>) -> Response {
    let result = runtime.run_tool(
        "orbit.adr.update",
        json!({
            "id": id.clone(),
            "status": "accepted",
            "model": HUMAN_ACTOR_LABEL,
        }),
    );
    match result {
        Ok(_) => match adr_show(&runtime, &id) {
            Ok(adr) => Json(adr).into_response(),
            Err(e) => map_runtime_error(e),
        },
        Err(e) => map_runtime_error(e),
    }
}

pub(super) async fn update_adr_action(
    Ws(runtime): Ws,
    Path(id): Path<String>,
    body: Option<Json<AdrPatchBody>>,
) -> Response {
    let Some(Json(body)) = body else {
        return bad_request("request body must include at least one updatable field".to_string());
    };
    let AdrPatchBody {
        title,
        owner,
        body,
        status,
        related_features,
        related_tasks,
        tags,
        paths,
        supersedes,
        legacy_ids,
        model,
    } = body;
    let model = model.as_deref().and_then(non_empty_string);

    let mut input = Map::new();
    insert_optional_string(&mut input, "title", title);
    insert_optional_string(&mut input, "owner", owner);
    insert_optional_string(&mut input, "body", body);
    insert_optional_string(&mut input, "status", status);
    insert_optional_list(&mut input, "related_features", related_features);
    insert_optional_list(&mut input, "related_tasks", related_tasks);
    insert_optional_list(&mut input, "tags", tags);
    insert_optional_list(&mut input, "paths", paths);
    insert_optional_list(&mut input, "supersedes", supersedes);
    insert_optional_list(&mut input, "legacy_ids", legacy_ids);
    if input.is_empty() && model.is_none() {
        return bad_request("request body must include at least one updatable field".to_string());
    }
    input.insert("id".to_string(), Value::String(id.clone()));
    input.insert(
        "model".to_string(),
        Value::String(model_attribution(model.as_deref())),
    );

    match runtime.run_tool("orbit.adr.update", Value::Object(input)) {
        Ok(_) => match adr_show(&runtime, &id) {
            Ok(adr) => Json(adr).into_response(),
            Err(e) => map_runtime_error(e),
        },
        Err(e) => map_runtime_error(e),
    }
}

pub(super) async fn supersede_adr_action(
    Ws(runtime): Ws,
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

    let input = json!({
        "old_id": id.clone(),
        "new_id": by.clone(),
        "model": model_attribution(body.model.as_deref()),
    });

    let result = runtime.run_tool("orbit.adr.supersede", input);

    match result {
        Ok(_) => {
            let old = match adr_show(&runtime, &id) {
                Ok(adr) => adr,
                Err(e) => return map_runtime_error(e),
            };
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

fn insert_optional_string(input: &mut Map<String, Value>, key: &str, value: Option<String>) {
    if let Some(value) = value.as_deref().and_then(non_empty_string) {
        input.insert(key.to_string(), Value::String(value));
    }
}

fn insert_optional_list(input: &mut Map<String, Value>, key: &str, value: Option<Vec<String>>) {
    if let Some(value) = value {
        input.insert(key.to_string(), json!(value));
    }
}

fn model_attribution(model: Option<&str>) -> String {
    model
        .and_then(non_empty_string)
        .unwrap_or_else(|| HUMAN_ACTOR_LABEL.to_string())
}

fn adr_list(runtime: &OrbitRuntime, input: Map<String, Value>) -> Result<Vec<Value>, OrbitError> {
    let value = runtime.run_tool("orbit.adr.list", Value::Object(input))?;
    match value {
        Value::Array(adrs) => Ok(adrs),
        other => Err(OrbitError::Execution(format!(
            "orbit.adr.list returned non-array JSON: {other}"
        ))),
    }
}

fn adr_show(runtime: &OrbitRuntime, id: &str) -> Result<Value, OrbitError> {
    runtime.run_tool("orbit.adr.show", json!({ "id": id }))
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
