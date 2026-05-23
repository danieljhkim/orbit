//! Test-only allowlist (sibling test layout); mirrors learnings.rs.
#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::sync::Arc;

use axum::body::Body;
use axum::http::{Method, Request, StatusCode, header};
use orbit_core::ActorIdentity;
use orbit_core::OrbitRuntime;
use orbit_core::TaskStatus;
use orbit_core::command::task::{TaskAddParams, TaskUpdateParams};
use serde_json::Value;
use tower::ServiceExt;

use super::super::router;
use super::test_support::body_json;

fn human_runtime() -> OrbitRuntime {
    // Force a human actor so threads created without an explicit model land
    // with `by="human"`. Without this, ambient `ORBIT_AGENT_MODEL` env vars
    // (e.g., from CI) leak in and break author-kind assertions.
    OrbitRuntime::in_memory()
        .expect("build runtime")
        .with_actor(ActorIdentity::human("human"))
}

fn seed_task(runtime: &OrbitRuntime) -> String {
    runtime
        .add_task(TaskAddParams {
            title: "Threads panel".to_string(),
            description: "Surface review threads in the dashboard.".to_string(),
            workspace_path: Some(".".to_string()),
            status: Some(TaskStatus::InProgress),
            ..Default::default()
        })
        .expect("add task")
        .id
}

async fn get(runtime: Arc<OrbitRuntime>, uri: &str) -> axum::response::Response {
    router()
        .with_state(runtime)
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri(uri)
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("response")
}

async fn post(
    runtime: Arc<OrbitRuntime>,
    uri: &str,
    body: Option<Value>,
) -> axum::response::Response {
    let mut builder = Request::builder()
        .method(Method::POST)
        .uri(uri)
        .header(header::ORIGIN, "http://localhost:7878");
    let body = if let Some(value) = body {
        builder = builder.header(header::CONTENT_TYPE, "application/json");
        Body::from(value.to_string())
    } else {
        Body::empty()
    };
    router()
        .with_state(runtime)
        .oneshot(builder.body(body).expect("request"))
        .await
        .expect("response")
}

#[tokio::test]
async fn list_review_threads_returns_threads_across_tasks() {
    let runtime = human_runtime();
    let task_id = seed_task(&runtime);
    runtime
        .add_review_thread(
            &task_id,
            "Initial agent ask.".to_string(),
            Some("src/main.rs".to_string()),
            Some(42),
            Some("codex".to_string()),
            Some("gpt-5.5".to_string()),
        )
        .expect("add agent thread");
    runtime
        .add_review_thread(
            &task_id,
            "Human follow-up thread.".to_string(),
            None,
            None,
            None,
            None,
        )
        .expect("add human thread");

    let response = get(Arc::new(runtime), "/review-threads").await;
    assert_eq!(response.status(), StatusCode::OK);
    let body = body_json(response).await;
    let items = body["items"].as_array().expect("items array");
    assert_eq!(items.len(), 2);

    let agent_thread = items
        .iter()
        .find(|item| item["path"].as_str() == Some("src/main.rs"))
        .expect("agent thread");
    assert_eq!(agent_thread["last_author_kind"].as_str(), Some("agent"));
    assert_eq!(agent_thread["status"].as_str(), Some("open"));
    assert_eq!(agent_thread["task_id"].as_str().unwrap(), task_id);
    assert_eq!(agent_thread["anchor"]["kind"].as_str(), Some("inline"));

    let human_thread = items
        .iter()
        .find(|item| item["path"].is_null())
        .expect("human thread");
    assert_eq!(human_thread["last_author_kind"].as_str(), Some("human"));
    assert_eq!(human_thread["anchor"]["kind"].as_str(), Some("task_level"));

    let stats = &body["stats"];
    assert_eq!(stats["open"].as_u64(), Some(2));
    assert_eq!(stats["resolved"].as_u64(), Some(0));
    assert_eq!(stats["agent_authored"].as_u64(), Some(1));
    assert_eq!(stats["human_authored"].as_u64(), Some(1));
}

#[tokio::test]
async fn list_review_threads_filters_by_status_and_author_kind() {
    let runtime = human_runtime();
    let task_id = seed_task(&runtime);
    let agent_thread = runtime
        .add_review_thread(
            &task_id,
            "Agent ask.".to_string(),
            None,
            None,
            Some("codex".to_string()),
            Some("gpt-5.5".to_string()),
        )
        .expect("add agent thread");
    runtime
        .add_review_thread(&task_id, "Human note.".to_string(), None, None, None, None)
        .expect("add human thread");
    runtime
        .resolve_review_thread(&task_id, &agent_thread.thread_id, None, None)
        .expect("resolve agent thread");

    let runtime = Arc::new(runtime);

    let open_only = get(runtime.clone(), "/review-threads?status=open").await;
    let open_body = body_json(open_only).await;
    let open_items = open_body["items"].as_array().expect("items");
    assert_eq!(open_items.len(), 1);
    assert_eq!(open_items[0]["last_author_kind"].as_str(), Some("human"));

    let resolved_only = get(runtime.clone(), "/review-threads?status=resolved").await;
    let resolved_body = body_json(resolved_only).await;
    let resolved_items = resolved_body["items"].as_array().expect("items");
    assert_eq!(resolved_items.len(), 1);
    assert_eq!(resolved_items[0]["status"].as_str(), Some("resolved"));

    let agent_only = get(
        runtime.clone(),
        "/review-threads?status=all&author_kind=agent",
    )
    .await;
    let agent_body = body_json(agent_only).await;
    let agent_items = agent_body["items"].as_array().expect("items");
    assert_eq!(agent_items.len(), 1);
    assert_eq!(agent_items[0]["last_author_family"].as_str(), Some("codex"));
}

#[tokio::test]
async fn reply_resolve_reopen_review_thread_through_router() {
    let runtime = human_runtime();
    let task_id = seed_task(&runtime);
    let thread = runtime
        .add_review_thread(
            &task_id,
            "Agent ask.".to_string(),
            Some("src/lib.rs".to_string()),
            Some(7),
            Some("codex".to_string()),
            Some("gpt-5.5".to_string()),
        )
        .expect("add thread");
    let runtime = Arc::new(runtime);

    let reply_uri = format!("/tasks/{task_id}/review-threads/{}/reply", thread.thread_id);
    let reply_resp = post(
        runtime.clone(),
        &reply_uri,
        Some(serde_json::json!({"body": "Human reply"})),
    )
    .await;
    assert_eq!(reply_resp.status(), StatusCode::OK);
    let reply_body = body_json(reply_resp).await;
    assert_eq!(reply_body["message_count"].as_u64(), Some(2));

    let resolve_uri = format!(
        "/tasks/{task_id}/review-threads/{}/resolve",
        thread.thread_id
    );
    let resolve_resp = post(runtime.clone(), &resolve_uri, None).await;
    assert_eq!(resolve_resp.status(), StatusCode::OK);
    let resolve_body = body_json(resolve_resp).await;
    assert_eq!(resolve_body["status"].as_str(), Some("resolved"));

    let reopen_uri = format!(
        "/tasks/{task_id}/review-threads/{}/reopen",
        thread.thread_id
    );
    let reopen_resp = post(runtime.clone(), &reopen_uri, None).await;
    assert_eq!(reopen_resp.status(), StatusCode::OK);
    let reopen_body = body_json(reopen_resp).await;
    assert_eq!(reopen_body["status"].as_str(), Some("open"));
}

#[tokio::test]
async fn reply_rejects_empty_body() {
    let runtime = human_runtime();
    let task_id = seed_task(&runtime);
    let thread = runtime
        .add_review_thread(&task_id, "Agent ask.".to_string(), None, None, None, None)
        .expect("add thread");

    let response = post(
        Arc::new(runtime),
        &format!("/tasks/{task_id}/review-threads/{}/reply", thread.thread_id),
        Some(serde_json::json!({"body": "   "})),
    )
    .await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn cross_origin_post_is_forbidden() {
    let runtime = human_runtime();
    let task_id = seed_task(&runtime);
    let thread = runtime
        .add_review_thread(&task_id, "Agent ask.".to_string(), None, None, None, None)
        .expect("add thread");
    let runtime = Arc::new(runtime);

    let response = router()
        .with_state(runtime)
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri(format!(
                    "/tasks/{task_id}/review-threads/{}/resolve",
                    thread.thread_id
                ))
                .header(header::ORIGIN, "http://evil.example.com")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("response");
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn list_review_threads_excludes_threads_on_closed_tasks() {
    let runtime = human_runtime();

    // Workable (included) statuses -- threads added and tasks remain workable
    let t_backlog = runtime
        .add_task(TaskAddParams {
            title: "backlog task".to_string(),
            status: Some(TaskStatus::Backlog),
            ..Default::default()
        })
        .expect("add backlog")
        .id;
    let t_inprog = runtime
        .add_task(TaskAddParams {
            title: "in-progress task".to_string(),
            status: Some(TaskStatus::InProgress),
            ..Default::default()
        })
        .expect("add inprog")
        .id;
    let t_review = runtime
        .add_task(TaskAddParams {
            title: "review task".to_string(),
            status: Some(TaskStatus::Review),
            ..Default::default()
        })
        .expect("add review")
        .id;

    // For excluded: create as workable, add thread while modifiable, then
    // transition to closed status so the persisted thread is on a now-closed task.
    // This exercises the hard filter in list_review_threads.
    // Friction/Archived omitted (special creation/archive paths); Proposed
    // transition may be restricted so omitted here ("if applicable").
    for (target_status, title) in [
        (TaskStatus::Done, "done task"),
        (TaskStatus::Blocked, "blocked/paused task"),
        (TaskStatus::Rejected, "rejected task"),
    ] {
        let tid = runtime
            .add_task(TaskAddParams {
                title: title.to_string(),
                status: Some(TaskStatus::InProgress),
                ..Default::default()
            })
            .expect("add pre-closed")
            .id;
        let th = runtime
            .add_review_thread(
                &tid,
                format!("thread on {title} (pre)"),
                None,
                None,
                None,
                None,
            )
            .expect("add thread while open");
        let _ = th.thread_id; // thread exists on now-closed task; filter tested by absence in list
        runtime
            .update_task_with_identity(
                &tid,
                TaskUpdateParams {
                    status: Some(target_status),
                    ..Default::default()
                },
                None,
                None,
            )
            .expect("transition to excluded status");
    }

    // Add one thread per workable task (use simple human threads)
    let mut good_ids: Vec<String> = Vec::new();
    for (tid, label) in [
        (&t_backlog, "backlog"),
        (&t_inprog, "inprog"),
        (&t_review, "review"),
    ] {
        let th = runtime
            .add_review_thread(tid, format!("thread on {label}"), None, None, None, None)
            .expect("add thread");
        good_ids.push(th.thread_id);
    }

    let runtime = Arc::new(runtime);
    let response = get(runtime, "/review-threads").await;
    assert_eq!(response.status(), StatusCode::OK);
    let body = body_json(response).await;
    let items = body["items"].as_array().expect("items array");

    // Only the 3 workable threads
    assert_eq!(items.len(), 3, "should exclude closed-task threads");

    let mut returned_ids: Vec<String> = items
        .iter()
        .map(|it| it["thread_id"].as_str().unwrap().to_string())
        .collect();
    let mut expected_ids = good_ids.clone();
    returned_ids.sort();
    expected_ids.sort();
    assert_eq!(
        returned_ids, expected_ids,
        "exact thread-ID set for workable tasks"
    );

    // Each row has task_status in the allowed set
    for item in items {
        let ts = item["task_status"].as_str().expect("task_status present");
        assert!(
            matches!(ts, "backlog" | "in-progress" | "review"),
            "unexpected task_status: {}",
            ts
        );
    }

    // Stats reflect only workable (3 open)
    let stats = &body["stats"];
    assert_eq!(stats["open"].as_u64(), Some(3));
    assert_eq!(stats["total"].as_u64(), Some(3));
}
