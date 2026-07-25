//! Test-only allowlist: the original tests under orbit-cli passed the same lints via
//! the crate-level test harness configuration; duplicated here for the extracted crate.
#![allow(clippy::expect_used, clippy::unwrap_used)]
use std::sync::Arc;

use axum::body::Body;
use axum::http::{Method, Request, StatusCode, header};
use orbit_common::test_fixtures::TEST_CODEX_MODEL;
use orbit_core::OrbitRuntime;
use serde_json::{Value, json};
use tower::ServiceExt;

use super::super::router;
use super::test_support::body_json;

const ADR_BODY: &str = "## Context\nFixture context.\n\n## Decision\nFixture decision.\n\n## Consequences\n- Dashboard behavior is observable.\n- Cost: Test fixtures carry enough ADR shape to pass validation.\n";

/// A runtime built with the agent-identity env cleared, so its actor resolves
/// to the human default.
///
/// The runtime captures `ActorIdentity` from the process env at construction
/// time, which makes "identity-less write" assertions depend on how the suite
/// was launched — an agent running it inside a managed Orbit run exports
/// `ORBIT_AGENT_MODEL` and the actor comes back as that model (ORB-10350).
/// The guard is confined to this synchronous call so it never spans an
/// `.await` (`clippy::await_holding_lock`).
fn runtime_without_agent_identity() -> OrbitRuntime {
    let _env =
        orbit_common::test_env::unset(orbit_common::test_env::AGENT_IDENTITY_ENV.iter().copied());
    OrbitRuntime::in_memory().expect("build runtime")
}

fn seed_adr(runtime: &OrbitRuntime, title: &str, related_tasks: Vec<&str>) -> Value {
    runtime
        .execute_tool_command(
            "orbit.adr.add",
            json!({
                "title": title,
                "body": ADR_BODY,
                "owner": TEST_CODEX_MODEL,
                "related_features": ["dashboard"],
                "related_tasks": related_tasks,
            }),
            None,
            Some(TEST_CODEX_MODEL.to_string()),
        )
        .expect("seed ADR")
}

fn accept_adr(runtime: &OrbitRuntime, id: &str) -> Value {
    runtime
        .execute_tool_command(
            "orbit.adr.update",
            json!({
                "id": id,
                "status": "accepted",
            }),
            None,
            Some(TEST_CODEX_MODEL.to_string()),
        )
        .expect("accept ADR")
}

fn adr_id(adr: &Value) -> &str {
    adr["id"].as_str().expect("ADR id")
}

async fn request_create(
    runtime: OrbitRuntime,
    origin: Option<&str>,
    body: Option<Value>,
) -> axum::response::Response {
    let mut builder = Request::builder().method(Method::POST).uri("/adrs");
    if let Some(origin) = origin {
        builder = builder.header(header::ORIGIN, origin);
    }
    let request = if let Some(body) = body {
        builder
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(body.to_string()))
            .expect("request")
    } else {
        builder.body(Body::empty()).expect("request")
    };

    router()
        .with_state(crate::state::DashboardState::single(Arc::new(runtime)))
        .oneshot(request)
        .await
        .expect("response")
}

async fn request_accept(
    runtime: OrbitRuntime,
    id: &str,
    origin: Option<&str>,
) -> axum::response::Response {
    let mut builder = Request::builder()
        .method(Method::POST)
        .uri(format!("/adrs/{id}/accept"));
    if let Some(origin) = origin {
        builder = builder.header(header::ORIGIN, origin);
    }

    router()
        .with_state(crate::state::DashboardState::single(Arc::new(runtime)))
        .oneshot(builder.body(Body::empty()).expect("request"))
        .await
        .expect("response")
}

async fn request_get(runtime: OrbitRuntime, id: &str) -> axum::response::Response {
    router()
        .with_state(crate::state::DashboardState::single(Arc::new(runtime)))
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri(format!("/adrs/{id}"))
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("response")
}

async fn request_supersede(
    runtime: OrbitRuntime,
    id: &str,
    origin: Option<&str>,
    body: Option<Value>,
) -> axum::response::Response {
    let mut builder = Request::builder()
        .method(Method::POST)
        .uri(format!("/adrs/{id}/supersede"));
    if let Some(origin) = origin {
        builder = builder.header(header::ORIGIN, origin);
    }
    let request = if let Some(body) = body {
        builder
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(body.to_string()))
            .expect("request")
    } else {
        builder.body(Body::empty()).expect("request")
    };

    router()
        .with_state(crate::state::DashboardState::single(Arc::new(runtime)))
        .oneshot(request)
        .await
        .expect("response")
}

#[tokio::test]
async fn create_persists_proposed_adr_and_reads_back() {
    let runtime = OrbitRuntime::in_memory().expect("build runtime");

    let response = request_create(
        runtime.clone(),
        Some("http://localhost:7878"),
        Some(json!({
            "title": "Created over HTTP",
            "body": ADR_BODY,
            "owner": TEST_CODEX_MODEL,
            "related_features": ["dashboard"],
            "related_tasks": ["ORB-00063"],
        })),
    )
    .await;

    assert_eq!(response.status(), StatusCode::OK);
    let created = body_json(response).await;
    let id = created["id"].as_str().expect("created ADR id").to_string();
    assert_eq!(created["status"], "proposed");
    assert_eq!(created["title"], "Created over HTTP");
    assert!(
        created["body"]
            .as_str()
            .is_some_and(|b| b.contains("Fixture decision")),
        "response carries the ADR body"
    );

    // On-disk shape matches the CLI/tool: proposed/<ID>/{adr.yaml,body.md}.
    let adr_dir = runtime.data_root().join("adrs").join("proposed").join(&id);
    assert!(adr_dir.join("adr.yaml").is_file(), "{}", adr_dir.display());
    assert!(adr_dir.join("body.md").is_file(), "{}", adr_dir.display());

    // Immediately visible via the existing read surface.
    let stored = runtime
        .execute_tool_command(
            "orbit.adr.show",
            json!({ "id": id }),
            None,
            Some(TEST_CODEX_MODEL.to_string()),
        )
        .expect("show created");
    assert_eq!(stored["id"], id);
    assert_eq!(stored["status"], "proposed");
    assert_eq!(stored["title"], "Created over HTTP");
}

#[tokio::test]
async fn create_without_attribution_defaults_owner_to_human() {
    let runtime = runtime_without_agent_identity();

    let response = request_create(
        runtime,
        Some("http://localhost:7878"),
        Some(json!({
            "title": "No attribution supplied",
            "body": ADR_BODY,
            "related_features": ["dashboard"],
            "related_tasks": ["ORB-00063"],
        })),
    )
    .await;

    assert_eq!(response.status(), StatusCode::OK);
    let created = body_json(response).await;
    assert_eq!(
        created["owner"], "human",
        "identity-less writes attribute to the human actor label, not a model constant"
    );
}

#[tokio::test]
async fn create_forwards_explicit_model_attribution() {
    let runtime = OrbitRuntime::in_memory().expect("build runtime");

    let response = request_create(
        runtime,
        Some("http://localhost:7878"),
        Some(json!({
            "title": "Explicit attribution supplied",
            "body": ADR_BODY,
            "model": TEST_CODEX_MODEL,
            "related_features": ["dashboard"],
            "related_tasks": ["ORB-00063"],
        })),
    )
    .await;

    assert_eq!(response.status(), StatusCode::OK);
    let created = body_json(response).await;
    assert_eq!(
        created["owner"], TEST_CODEX_MODEL,
        "caller-supplied `model` is forwarded to the tool host unchanged"
    );
}

#[tokio::test]
async fn federated_http_show_and_mutators_preserve_typed_origin_boundary() {
    let root = tempfile::tempdir().expect("tempdir");
    let global_root = root.path().join("global");
    let shared_root = root.path().join("hub/.orbit");
    let local_root = root.path().join("local/.orbit");
    let sibling_root = root.path().join("sibling/.orbit");
    for path in [&global_root, &shared_root, &local_root, &sibling_root] {
        std::fs::create_dir_all(path).expect("runtime root");
    }
    let sibling = OrbitRuntime::from_resolved_roots(&global_root, &shared_root, &sibling_root)
        .expect("sibling runtime");
    let local = OrbitRuntime::from_resolved_roots(&global_root, &shared_root, &local_root)
        .expect("local runtime");
    let old = seed_adr(&sibling, "Federated old", vec!["ORB-10297"]);
    let new = seed_adr(&sibling, "Federated new", vec!["ORB-10297"]);
    accept_adr(&sibling, adr_id(&new));
    let old_id = adr_id(&old);
    let new_id = adr_id(&new);

    let response = request_get(local.clone(), old_id).await;
    assert_eq!(response.status(), StatusCode::OK);
    let shown = body_json(response).await;
    assert_eq!(shown["title"], "Federated old");
    let stored_body = std::fs::read_to_string(
        sibling_root
            .join("adrs/proposed")
            .join(old_id)
            .join("body.md"),
    )
    .expect("stored sibling body");
    assert_eq!(shown["body"], stored_body);
    assert_eq!(shown["artifact_origin"]["mode"], "federated");
    assert_eq!(shown["artifact_origin"]["branch"], Value::Null);
    assert!(shown["artifact_origin"].get("body_path").is_none());

    for response in [
        request_update(
            local.clone(),
            old_id,
            Some("http://localhost:7878"),
            Some(json!({"title": "must not write"})),
        )
        .await,
        request_accept(local.clone(), old_id, Some("http://localhost:7878")).await,
        request_supersede(
            local.clone(),
            old_id,
            Some("http://localhost:7878"),
            Some(json!({"by": new_id})),
        )
        .await,
    ] {
        assert_eq!(response.status(), StatusCode::CONFLICT);
        let payload = body_json(response).await;
        assert_eq!(payload["code"], "artifact_not_local");
        assert!(payload["error"].is_string());
        assert_eq!(payload["artifact_origin"]["mode"], "federated");
        assert!(payload["artifact_origin"].get("body_path").is_none());
    }

    let unavailable = seed_adr(&sibling, "Unavailable", vec!["ORB-10297"]);
    std::fs::remove_file(
        sibling_root
            .join("adrs/proposed")
            .join(adr_id(&unavailable))
            .join("body.md"),
    )
    .expect("remove body");
    let response = request_get(local.clone(), adr_id(&unavailable)).await;
    assert_eq!(response.status(), StatusCode::CONFLICT);
    let payload = body_json(response).await;
    assert_eq!(payload["code"], "remote_artifact_unavailable");
    assert_eq!(payload["artifact_origin"]["mode"], "federated");

    let response = request_get(local.clone(), "ADR-9999").await;
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    let payload = body_json(response).await;
    assert!(payload["error"].is_string());
    assert!(payload.get("code").is_none());

    let source = sibling_root.join("adrs/proposed").join(old_id);
    let target = local_root.join("adrs/proposed").join(old_id);
    std::fs::create_dir_all(&target).expect("local ADR dir");
    std::fs::copy(source.join("adr.yaml"), target.join("adr.yaml")).expect("copy ADR yaml");
    std::fs::write(target.join("body.md"), "landed HTTP body").expect("write local body");
    let response = request_update(
        local.clone(),
        old_id,
        Some("http://localhost:7878"),
        Some(json!({"title": "Landed HTTP update"})),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let payload = body_json(response).await;
    assert_eq!(payload["title"], "Landed HTTP update");
    assert_eq!(payload["body"], "landed HTTP body");
    assert_eq!(payload["artifact_origin"]["mode"], "local");
}

#[tokio::test]
async fn create_requires_localhost_origin() {
    let runtime = OrbitRuntime::in_memory().expect("build runtime");

    let response = request_create(
        runtime.clone(),
        None,
        Some(json!({ "title": "No origin", "body": ADR_BODY })),
    )
    .await;

    assert_eq!(response.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn create_rejects_missing_title() {
    let runtime = OrbitRuntime::in_memory().expect("build runtime");

    let response = request_create(
        runtime,
        Some("http://localhost:7878"),
        Some(json!({ "body": ADR_BODY })),
    )
    .await;

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let payload = body_json(response).await;
    let error = payload["error"].as_str().expect("error");
    assert!(error.contains("title"), "{error}");
}

#[tokio::test]
async fn create_rejects_malformed_json() {
    let runtime = OrbitRuntime::in_memory().expect("build runtime");

    let response = router()
        .with_state(crate::state::DashboardState::single(Arc::new(runtime)))
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/adrs")
                .header(header::ORIGIN, "http://localhost:7878")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from("{not valid json"))
                .expect("request"),
        )
        .await
        .expect("response");

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let payload = body_json(response).await;
    assert!(
        payload["error"]
            .as_str()
            .is_some_and(|e| e.contains("malformed")),
        "structured error names the malformed payload"
    );
}

#[tokio::test]
async fn post_adr_routes_require_localhost_origin() {
    let runtime = OrbitRuntime::in_memory().expect("build runtime");
    let proposed = seed_adr(&runtime, "Proposed dashboard ADR", vec!["ORB-00063"]);

    let response = request_accept(runtime.clone(), adr_id(&proposed), None).await;

    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    let stored = runtime
        .execute_tool_command(
            "orbit.adr.show",
            json!({ "id": adr_id(&proposed) }),
            None,
            Some(TEST_CODEX_MODEL.to_string()),
        )
        .expect("show proposed");
    assert_eq!(stored["status"], "proposed");

    let old = seed_adr(&runtime, "Old dashboard ADR", vec!["ORB-00063"]);
    let new = seed_adr(&runtime, "New dashboard ADR", vec!["ORB-00063"]);
    accept_adr(&runtime, adr_id(&old));
    accept_adr(&runtime, adr_id(&new));

    let response = request_supersede(
        runtime.clone(),
        adr_id(&old),
        None,
        Some(json!({ "by": adr_id(&new) })),
    )
    .await;

    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    let stored = runtime
        .execute_tool_command(
            "orbit.adr.show",
            json!({ "id": adr_id(&old) }),
            None,
            Some(TEST_CODEX_MODEL.to_string()),
        )
        .expect("show old");
    assert_eq!(stored["status"], "accepted");
}

#[tokio::test]
async fn accept_returns_bad_request_when_tool_rejects_missing_related_tasks() {
    let runtime = OrbitRuntime::in_memory().expect("build runtime");
    let adr = seed_adr(&runtime, "Needs task linkage", vec![]);

    let response = request_accept(runtime, adr_id(&adr), Some("http://localhost:7878")).await;

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let payload = body_json(response).await;
    let error = payload["error"].as_str().expect("error");
    assert!(
        error.contains(&format!(
            "Invalid ADR status transition: {}: proposed -> accepted requires non-empty related_tasks",
            adr_id(&adr)
        )),
        "{error}"
    );
}

#[tokio::test]
async fn supersede_returns_not_found_for_unknown_source_id() {
    let runtime = OrbitRuntime::in_memory().expect("build runtime");
    let replacement = seed_adr(&runtime, "Replacement dashboard ADR", vec!["ORB-00063"]);
    accept_adr(&runtime, adr_id(&replacement));

    let response = request_supersede(
        runtime,
        "ADR-9999",
        Some("http://localhost:7878"),
        Some(json!({ "by": adr_id(&replacement) })),
    )
    .await;

    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn supersede_rejects_malformed_by() {
    let runtime = OrbitRuntime::in_memory().expect("build runtime");
    let old = seed_adr(&runtime, "Old dashboard ADR", vec!["ORB-00063"]);
    accept_adr(&runtime, adr_id(&old));

    let response = request_supersede(
        runtime.clone(),
        adr_id(&old),
        Some("http://127.0.0.1:7878"),
        Some(json!({ "by": "bad" })),
    )
    .await;

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let stored = runtime
        .execute_tool_command(
            "orbit.adr.show",
            json!({ "id": adr_id(&old) }),
            None,
            Some(TEST_CODEX_MODEL.to_string()),
        )
        .expect("show old");
    assert_eq!(stored["status"], "accepted");
}

#[tokio::test]
async fn supersede_moves_source_to_superseded_and_populates_edge() {
    let runtime = OrbitRuntime::in_memory().expect("build runtime");
    let old = seed_adr(&runtime, "Old dashboard ADR", vec!["ORB-00063"]);
    let new = seed_adr(&runtime, "New dashboard ADR", vec!["ORB-00063"]);
    accept_adr(&runtime, adr_id(&old));
    accept_adr(&runtime, adr_id(&new));

    let response = request_supersede(
        runtime.clone(),
        adr_id(&old),
        Some("http://localhost:7878"),
        Some(json!({ "by": adr_id(&new), "reason": "replacement" })),
    )
    .await;

    assert_eq!(response.status(), StatusCode::OK);
    let payload = body_json(response).await;
    assert_eq!(payload["old"]["id"], adr_id(&old));
    assert_eq!(payload["old"]["status"], "superseded");
    assert_eq!(payload["old"]["superseded_by"], adr_id(&new));
    assert_eq!(payload["new"]["id"], adr_id(&new));
    assert_eq!(payload["new"]["supersedes"][0], adr_id(&old));

    let superseded_dir = runtime
        .data_root()
        .join("adrs")
        .join("superseded")
        .join(adr_id(&old));
    assert!(superseded_dir.is_dir(), "{}", superseded_dir.display());
    let accepted_dir = runtime
        .data_root()
        .join("adrs")
        .join("accepted")
        .join(adr_id(&old));
    assert!(!accepted_dir.exists(), "{}", accepted_dir.display());
}

async fn request_update(
    runtime: OrbitRuntime,
    id: &str,
    origin: Option<&str>,
    body: Option<Value>,
) -> axum::response::Response {
    let mut builder = Request::builder()
        .method(Method::PATCH)
        .uri(format!("/adrs/{id}"));
    if let Some(origin) = origin {
        builder = builder.header(header::ORIGIN, origin);
    }
    let request = if let Some(body) = body {
        builder
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(body.to_string()))
            .expect("request")
    } else {
        builder.body(Body::empty()).expect("request")
    };

    router()
        .with_state(crate::state::DashboardState::single(Arc::new(runtime)))
        .oneshot(request)
        .await
        .expect("response")
}

#[tokio::test]
async fn update_requires_localhost_origin() {
    let runtime = OrbitRuntime::in_memory().expect("build runtime");
    let adr = seed_adr(&runtime, "Cross-origin ADR", vec!["ORB-00063"]);

    let response = request_update(
        runtime.clone(),
        adr_id(&adr),
        None,
        Some(json!({ "tags": ["blocked"] })),
    )
    .await;

    assert_eq!(response.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn update_rejects_empty_body() {
    let runtime = OrbitRuntime::in_memory().expect("build runtime");
    let adr = seed_adr(&runtime, "Empty patch ADR", vec!["ORB-00063"]);

    let response = request_update(
        runtime,
        adr_id(&adr),
        Some("http://localhost:7878"),
        Some(json!({})),
    )
    .await;

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn update_sets_status_and_tags() {
    let runtime = OrbitRuntime::in_memory().expect("build runtime");
    let adr = seed_adr(&runtime, "Curated ADR", vec!["ORB-00063"]);

    let response = request_update(
        runtime.clone(),
        adr_id(&adr),
        Some("http://localhost:7878"),
        Some(json!({ "status": "accepted", "tags": ["curated", "api"] })),
    )
    .await;

    assert_eq!(response.status(), StatusCode::OK);
    let payload = body_json(response).await;
    assert_eq!(payload["status"], "accepted");
    assert_eq!(payload["tags"][0], "curated");
    assert_eq!(payload["tags"][1], "api");
    // Body is re-attached from disk on the update response.
    assert!(payload["body"].as_str().is_some());

    let stored = runtime
        .execute_tool_command(
            "orbit.adr.show",
            json!({ "id": adr_id(&adr) }),
            None,
            Some(TEST_CODEX_MODEL.to_string()),
        )
        .expect("show adr");
    assert_eq!(stored["status"], "accepted");
}

fn transition_audit_actor(runtime: &OrbitRuntime, adr_id: &str) -> String {
    let events = runtime
        .list_audit_events(None, Some("orbit.adr.update".to_string()), None, None, 20)
        .expect("audit events");
    let event = events
        .iter()
        .find(|event| {
            event
                .arguments_json
                .as_deref()
                .is_some_and(|args| args.contains(adr_id))
        })
        .expect("transition audit event");
    let payload: Value = serde_json::from_str(event.arguments_json.as_deref().expect("args"))
        .expect("audit payload");
    payload["actor"].as_str().expect("actor field").to_string()
}

#[tokio::test]
async fn update_without_attribution_records_human_actor_in_audit() {
    let runtime = runtime_without_agent_identity();
    let adr = seed_adr(&runtime, "No attribution transition", vec!["ORB-00063"]);

    let response = request_update(
        runtime.clone(),
        adr_id(&adr),
        Some("http://localhost:7878"),
        Some(json!({ "status": "accepted" })),
    )
    .await;

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        transition_audit_actor(&runtime, adr_id(&adr)),
        "human",
        "identity-less transitions attribute to the human actor label"
    );
}

#[tokio::test]
async fn update_forwards_explicit_model_attribution_to_audit() {
    let runtime = OrbitRuntime::in_memory().expect("build runtime");
    let adr = seed_adr(
        &runtime,
        "Explicit attribution transition",
        vec!["ORB-00063"],
    );

    let response = request_update(
        runtime.clone(),
        adr_id(&adr),
        Some("http://localhost:7878"),
        Some(json!({ "status": "accepted", "model": TEST_CODEX_MODEL })),
    )
    .await;

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        transition_audit_actor(&runtime, adr_id(&adr)),
        TEST_CODEX_MODEL,
        "caller-supplied `model` is forwarded to the tool host unchanged"
    );
}

#[tokio::test]
async fn update_rejects_invalid_status_transition() {
    let runtime = OrbitRuntime::in_memory().expect("build runtime");
    let adr = seed_adr(&runtime, "Accepted ADR", vec!["ORB-00063"]);
    accept_adr(&runtime, adr_id(&adr));

    // accepted -> proposed is a rejected lifecycle transition.
    let response = request_update(
        runtime.clone(),
        adr_id(&adr),
        Some("http://localhost:7878"),
        Some(json!({ "status": "proposed" })),
    )
    .await;

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let payload = body_json(response).await;
    assert!(
        payload["error"]
            .as_str()
            .expect("error")
            .contains("Invalid ADR status transition"),
        "{}",
        payload["error"]
    );

    let stored = runtime
        .execute_tool_command(
            "orbit.adr.show",
            json!({ "id": adr_id(&adr) }),
            None,
            Some(TEST_CODEX_MODEL.to_string()),
        )
        .expect("show adr");
    assert_eq!(stored["status"], "accepted");
}

#[tokio::test]
async fn update_rejects_direct_write_to_superseded() {
    let runtime = OrbitRuntime::in_memory().expect("build runtime");
    let adr = seed_adr(&runtime, "Guarded ADR", vec!["ORB-00063"]);
    accept_adr(&runtime, adr_id(&adr));

    // Direct writes to `superseded` must go through the supersede route.
    let response = request_update(
        runtime,
        adr_id(&adr),
        Some("http://localhost:7878"),
        Some(json!({ "status": "superseded" })),
    )
    .await;

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}
