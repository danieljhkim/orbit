//! Test-only allowlist: endpoint tests use unwrap/expect for fixture setup.
#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::sync::Arc;

use axum::Router;
use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode};
use axum::response::Response;
use chrono::{Duration, SecondsFormat};
use orbit_common::types::{InvocationTrace, KnowledgeRunMetrics, TokenUsage, ToolCallTrace};
use orbit_core::command::job::JobRunListParams;
use orbit_core::metrics::{KnowledgeStatsSummary, aggregate as aggregate_knowledge_stats};
use orbit_core::{
    ActivityInvocationMetrics, InvocationInsertParams, InvocationQuery, InvocationRecord,
    JobRunState, OrbitRuntime, TaskInvocationMetrics, ToolInvocationMetrics,
};
use tower::ServiceExt;

use super::super::router;
use super::test_support::{body_json, seed_run, write_seeded_run};

const RUN_ID: &str = "jrun-metrics-api";
const OTHER_RUN_ID: &str = "jrun-metrics-other";
const JOB_ID: &str = "metrics_api";
const TASK_ID: &str = "ORB-METRICS-1";

async fn request_metrics(runtime: OrbitRuntime, uri: &str) -> Response {
    Router::new()
        .nest("/api", router())
        .with_state(Arc::new(runtime))
        .oneshot(
            Request::builder()
                .uri(format!("/api{uri}"))
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("response")
}

async fn body_bytes(response: Response) -> Vec<u8> {
    to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read response body")
        .to_vec()
}

fn seed_metrics_runtime() -> OrbitRuntime {
    let runtime = OrbitRuntime::in_memory().expect("build runtime");
    let mut run = seed_run(&runtime, RUN_ID, JOB_ID, JobRunState::Success);
    run.knowledge_metrics = Some(KnowledgeRunMetrics {
        raw_read_token_baseline: 1_000,
        knowledge_pack_tokens: Some(250),
        compression_ratio: Some(4.0),
        actual_fs_read_tokens_during_run: 100,
        double_read_rate: Some(0.25),
        knowledge_pack_used: true,
        knowledge_pack_unresolved_count: 0,
        total_llm_input_tokens: 600,
    });
    write_seeded_run(&runtime, &run);

    let mut fallback = seed_run(&runtime, OTHER_RUN_ID, JOB_ID, JobRunState::Success);
    fallback.knowledge_metrics = Some(KnowledgeRunMetrics {
        raw_read_token_baseline: 1_000,
        knowledge_pack_tokens: None,
        compression_ratio: None,
        actual_fs_read_tokens_during_run: 700,
        double_read_rate: Some(0.75),
        knowledge_pack_used: false,
        knowledge_pack_unresolved_count: 0,
        total_llm_input_tokens: 1_200,
    });
    write_seeded_run(&runtime, &fallback);

    seed_invocation(
        &runtime,
        SeedInvocation {
            job_run_id: RUN_ID,
            activity_id: "implement_one",
            agent: "codex",
            model: Some("gpt-5.5"),
            duration_ms: 1_234,
            input_tokens: 100,
            cache_read_tokens: 25,
            cache_create_tokens: 5,
            output_tokens: 20,
            task_ids: &[TASK_ID],
            tool_calls: &[("fs.read", 321), ("orbit.task.show", 123)],
        },
    );
    seed_invocation(
        &runtime,
        SeedInvocation {
            job_run_id: OTHER_RUN_ID,
            activity_id: "plan",
            agent: "claude",
            model: Some("claude-opus"),
            duration_ms: 2_000,
            input_tokens: 80,
            cache_read_tokens: 0,
            cache_create_tokens: 0,
            output_tokens: 40,
            task_ids: &["ORB-METRICS-OTHER"],
            tool_calls: &[("fs.write", 64)],
        },
    );

    runtime
}

struct SeedInvocation<'a> {
    job_run_id: &'a str,
    activity_id: &'a str,
    agent: &'a str,
    model: Option<&'a str>,
    duration_ms: u64,
    input_tokens: u64,
    cache_read_tokens: u64,
    cache_create_tokens: u64,
    output_tokens: u64,
    task_ids: &'a [&'a str],
    tool_calls: &'a [(&'a str, u64)],
}

fn seed_invocation(runtime: &OrbitRuntime, seed: SeedInvocation<'_>) {
    runtime
        .insert_invocation_trace_record(&InvocationInsertParams {
            job_run_id: seed.job_run_id.to_string(),
            activity_id: seed.activity_id.to_string(),
            agent: seed.agent.to_string(),
            model: seed.model.map(ToOwned::to_owned),
            slot: None,
            task_ids: seed
                .task_ids
                .iter()
                .map(|task_id| (*task_id).to_string())
                .collect(),
            trace: InvocationTrace {
                usage: TokenUsage {
                    input: seed.input_tokens,
                    cache_read: seed.cache_read_tokens,
                    cache_create: seed.cache_create_tokens,
                    output: seed.output_tokens,
                },
                tool_calls: seed
                    .tool_calls
                    .iter()
                    .enumerate()
                    .map(|(seq, (tool_name, result_bytes))| ToolCallTrace {
                        seq: seq as u32,
                        tool_name: (*tool_name).to_string(),
                        result_bytes: *result_bytes,
                        result_payload: None,
                    })
                    .collect(),
                duration_ms: seed.duration_ms,
            },
        })
        .expect("insert invocation");
}

#[tokio::test]
async fn metrics_endpoints_return_runtime_shapes() {
    let runtime = seed_metrics_runtime();

    let knowledge = request_metrics(runtime.clone(), "/metrics/knowledge?limit=20").await;
    assert_eq!(knowledge.status(), StatusCode::OK);
    let knowledge_bytes = body_bytes(knowledge).await;
    let expected_knowledge = aggregate_knowledge_stats(
        &runtime
            .list_job_runs(JobRunListParams {
                limit: Some(20),
                ..Default::default()
            })
            .expect("list job runs"),
    );
    let decoded_knowledge: KnowledgeStatsSummary =
        serde_json::from_slice(&knowledge_bytes).expect("knowledge json");
    assert_eq!(decoded_knowledge, expected_knowledge);
    assert!(decoded_knowledge.total_runs > 0);
    assert_eq!(
        knowledge_bytes,
        serde_json::to_vec(&expected_knowledge).expect("expected knowledge json")
    );

    let activity = request_metrics(runtime.clone(), "/metrics/activity").await;
    assert_eq!(activity.status(), StatusCode::OK);
    let activity_bytes = body_bytes(activity).await;
    let expected_activity: Vec<ActivityInvocationMetrics> = runtime
        .activity_invocation_metrics()
        .expect("activity metrics");
    let decoded_activity: Vec<ActivityInvocationMetrics> =
        serde_json::from_slice(&activity_bytes).expect("activity json");
    assert_eq!(decoded_activity, expected_activity);
    assert_eq!(
        activity_bytes,
        serde_json::to_vec(&expected_activity).expect("expected activity json")
    );

    let tools = request_metrics(runtime.clone(), "/metrics/tools").await;
    assert_eq!(tools.status(), StatusCode::OK);
    let tools_bytes = body_bytes(tools).await;
    let expected_tools: Vec<ToolInvocationMetrics> =
        runtime.tool_invocation_metrics().expect("tool metrics");
    let decoded_tools: Vec<ToolInvocationMetrics> =
        serde_json::from_slice(&tools_bytes).expect("tools json");
    assert_eq!(decoded_tools, expected_tools);
    assert_eq!(
        tools_bytes,
        serde_json::to_vec(&expected_tools).expect("expected tools json")
    );

    let task = request_metrics(runtime.clone(), &format!("/metrics/task/{TASK_ID}")).await;
    assert_eq!(task.status(), StatusCode::OK);
    let task_bytes = body_bytes(task).await;
    let expected_task: TaskInvocationMetrics = runtime
        .task_invocation_metrics(TASK_ID)
        .expect("task metrics");
    let decoded_task: TaskInvocationMetrics =
        serde_json::from_slice(&task_bytes).expect("task json");
    assert_eq!(decoded_task, expected_task);
    assert_eq!(
        task_bytes,
        serde_json::to_vec(&expected_task).expect("expected task json")
    );

    let invocations = request_metrics(runtime.clone(), "/metrics/invocations?limit=10").await;
    assert_eq!(invocations.status(), StatusCode::OK);
    let invocations_bytes = body_bytes(invocations).await;
    let expected_invocations: Vec<InvocationRecord> = runtime
        .invocation_records(InvocationQuery {
            limit: 10,
            ..Default::default()
        })
        .expect("invocation records");
    let decoded_invocations: Vec<InvocationRecord> =
        serde_json::from_slice(&invocations_bytes).expect("invocations json");
    assert_eq!(decoded_invocations, expected_invocations);
    assert_eq!(
        invocations_bytes,
        serde_json::to_vec(&expected_invocations).expect("expected invocations json")
    );
}

#[tokio::test]
async fn metrics_invocations_accepts_full_filter_set() {
    let runtime = seed_metrics_runtime();
    let record = runtime
        .invocation_records(InvocationQuery {
            job_run_id: Some(RUN_ID.to_string()),
            activity_id: Some("implement_one".to_string()),
            limit: 1,
            ..Default::default()
        })
        .expect("invocation record")
        .into_iter()
        .next()
        .expect("seeded invocation");
    let since = (record.ts - Duration::minutes(1)).to_rfc3339_opts(SecondsFormat::Millis, true);
    let until = (record.ts + Duration::minutes(1)).to_rfc3339_opts(SecondsFormat::Millis, true);
    let uri = format!(
        "/metrics/invocations?since={since}&until={until}&job_run_id={RUN_ID}&activity_id=implement_one&task_id={TASK_ID}&agent=codex&model=gpt-5.5&tool_name=fs.read&limit=1"
    );

    let response = request_metrics(runtime, &uri).await;

    assert_eq!(response.status(), StatusCode::OK);
    let payload = body_json(response).await;
    let rows = payload.as_array().expect("rows");
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["job_run_id"], RUN_ID);
    assert_eq!(rows[0]["activity_id"], "implement_one");
    assert_eq!(rows[0]["agent"], "codex");
    assert_eq!(rows[0]["model"], "gpt-5.5");
    assert_eq!(rows[0]["task_ids"][0], TASK_ID);
    assert_eq!(rows[0]["tool_calls"][0]["tool_name"], "fs.read");
}

#[tokio::test]
async fn metrics_invocations_reports_invalid_rfc3339_field() {
    let runtime = seed_metrics_runtime();

    let response = request_metrics(runtime, "/metrics/invocations?since=not-a-date").await;

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let payload = body_json(response).await;
    let message = payload["error"].as_str().expect("error message");
    assert!(message.contains("invalid since:"));
}
