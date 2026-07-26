//! Sibling tests for `mod.rs` (the v2_host module root; migrated per ORB-10387 /
//! docs/design-patterns/test_layout.md).

use std::sync::{Mutex, MutexGuard, OnceLock};

use orbit_common::types::activity_job::{AgentLoopSpec, Backend, OnDenial, Provider};
use orbit_common::types::{
    InvocationTrace, JobRunState, TaskPriority, TaskStatus, TaskType, TokenUsage, ToolCallTrace,
};
use orbit_engine::{V2AuditWriter, drive_agent_loop, reset_replay_transport};
use orbit_store::InvocationQuery;
use tempfile::NamedTempFile;

use super::super::test_support::{runtime_with_workspace_layout, seed_list_backlog_task};
use super::super::*;

fn replay_env_guard() -> MutexGuard<'static, ()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

struct ReplayFixtureGuard {
    prior: Option<String>,
}

impl ReplayFixtureGuard {
    fn set(path: &std::path::Path) -> Self {
        let prior = std::env::var("ORBIT_V2_REPLAY_FIXTURE").ok();
        // SAFETY: replay fixture env mutation is serialized by `replay_env_guard`.
        unsafe {
            std::env::set_var("ORBIT_V2_REPLAY_FIXTURE", path);
        }
        reset_replay_transport();
        Self { prior }
    }
}

impl Drop for ReplayFixtureGuard {
    fn drop(&mut self) {
        reset_replay_transport();
        // SAFETY: replay fixture env mutation is serialized by `replay_env_guard`.
        unsafe {
            match &self.prior {
                Some(value) => std::env::set_var("ORBIT_V2_REPLAY_FIXTURE", value),
                None => std::env::remove_var("ORBIT_V2_REPLAY_FIXTURE"),
            }
        }
    }
}

fn write_replay_fixture(value: Value) -> NamedTempFile {
    let file = NamedTempFile::new().expect("fixture temp file");
    std::fs::write(
        file.path(),
        serde_json::to_vec(&value).expect("serialize replay fixture"),
    )
    .expect("write replay fixture");
    file
}

fn seed_running_job_run(runtime: &OrbitRuntime, job_id: &str) -> String {
    let run = runtime
        .stores()
        .jobs()
        .insert_job_run(job_id, 1, chrono::Utc::now(), None, None)
        .expect("insert job run");
    runtime
        .stores()
        .jobs()
        .mark_job_run_running(&run.run_id, chrono::Utc::now(), std::process::id())
        .expect("mark run running");
    run.run_id
}

fn payload_tool_call(seq: u32, tool_name: &str, payload: Value) -> ToolCallTrace {
    ToolCallTrace {
        seq,
        tool_name: tool_name.to_string(),
        result_bytes: serde_json::to_vec(&payload)
            .expect("serialize payload")
            .len() as u64,
        result_payload: Some(payload),
    }
}

fn byte_count_tool_call(seq: u32, tool_name: &str, result_bytes: u64) -> ToolCallTrace {
    ToolCallTrace {
        seq,
        tool_name: tool_name.to_string(),
        result_bytes,
        result_payload: None,
    }
}

fn trace_with_tool_calls(input_tokens: u64, tool_calls: Vec<ToolCallTrace>) -> InvocationTrace {
    InvocationTrace {
        usage: TokenUsage {
            input: input_tokens,
            cache_read: 0,
            cache_create: 0,
            cache_create_1h: 0,
            output: 0,
        },
        tool_calls,
        duration_ms: 10,
        provider_model: None,
        provider_cost_usd: None,
    }
}

#[test]
fn persist_invocation_trace_prefers_provider_model_over_requested_alias() {
    let (_root, runtime, _repo_root) = runtime_with_workspace_layout();
    let run_id = seed_running_job_run(&runtime, "provider_model_job");
    let trace = InvocationTrace {
        provider_model: Some("claude-fable-5".to_string()),
        ..InvocationTrace::default()
    };

    V2RuntimeHost::persist_invocation_trace(
        &runtime,
        &run_id,
        "implement_one",
        "claude",
        Some("fable"),
        &serde_json::json!({ "task_id": "ORB-10370" }),
        &trace,
    )
    .expect("persist provider model");

    let records = runtime
        .invocation_records(InvocationQuery {
            job_run_id: Some(run_id),
            limit: 1,
            ..InvocationQuery::default()
        })
        .expect("query invocation records");
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].agent, "claude");
    assert_eq!(records[0].model.as_deref(), Some("claude-fable-5"));
}

fn persist_test_trace(runtime: &OrbitRuntime, run_id: &str, trace: &InvocationTrace) {
    V2RuntimeHost::persist_invocation_trace(
        runtime,
        run_id,
        "knowledge_step",
        "codex",
        Some("gpt-test"),
        &serde_json::json!({ "task_id": "ORB-KNOWLEDGE-TEST" }),
        trace,
    )
    .expect("persist invocation trace");
}

#[test]
fn persist_invocation_trace_no_longer_measures_removed_pack_tool() {
    // ORB-00391: orbit.graph.pack was removed with orbit-knowledge (v1). A trace
    // whose only payload tool is the former pack tool records no knowledge metrics,
    // because merge_invocation_trace now measures fs.read exclusively.
    let (_root, runtime, _repo_root) = runtime_with_workspace_layout();
    let run_id = seed_running_job_run(&runtime, "knowledge_pack_job");
    let trace = trace_with_tool_calls(
        155,
        vec![payload_tool_call(
            1,
            "orbit.graph.pack",
            serde_json::json!({
                "raw_read_token_baseline": 400,
                "knowledge_pack_tokens": 100,
                "entries": [{ "selector": "file:src/lib.rs", "source": "pub fn demo() {}" }],
                "unresolved_selectors": [],
            }),
        )],
    );

    persist_test_trace(&runtime, &run_id, &trace);

    let run = runtime.show_job_run(&run_id).expect("show job run");
    assert_eq!(run.state, JobRunState::Running);
    assert!(
        run.knowledge_metrics.is_none(),
        "the removed pack tool must not produce knowledge metrics"
    );
    assert_eq!(run.job_id, "knowledge_pack_job");
}

#[test]
fn persist_invocation_trace_records_fs_read_double_read_metrics() {
    // ORB-00391: with the pack baseline gone, every fs.read is "double read"
    // relative to itself, so double_read_rate is 1.0 for an fs.read-only run.
    let (_root, runtime, _repo_root) = runtime_with_workspace_layout();

    let fallback_run_id = seed_running_job_run(&runtime, "knowledge_fallback_job");
    let fallback_trace = trace_with_tool_calls(50, vec![byte_count_tool_call(1, "fs.read", 120)]);

    persist_test_trace(&runtime, &fallback_run_id, &fallback_trace);

    let fallback_run = runtime
        .show_job_run(&fallback_run_id)
        .expect("show fallback job run");
    let metrics = fallback_run
        .knowledge_metrics
        .expect("fallback metrics recorded");
    assert!(!metrics.knowledge_pack_used);
    assert_eq!(metrics.raw_read_token_baseline, 30);
    assert_eq!(metrics.knowledge_pack_tokens, None);
    assert_eq!(metrics.actual_fs_read_tokens_during_run, 30);
    assert_eq!(metrics.double_read_rate, Some(1.0));
    assert_eq!(metrics.total_llm_input_tokens, 50);
}

#[test]
fn http_agent_loop_tool_update_persists_runtime_identity_family() {
    let _lock = replay_env_guard();
    let (_root, runtime, _repo_root) = runtime_with_workspace_layout();
    let task = seed_list_backlog_task(
        &runtime,
        "runtime identity regression",
        TaskStatus::InProgress,
        TaskPriority::Medium,
        TaskType::Chore,
        None,
        Vec::new(),
    );
    let fixture = write_replay_fixture(serde_json::json!({
        "turns": [
            {
                "content": [{
                    "kind": "tool_use",
                    "id": "toolu_identity_update",
                    "name": "orbit.task.update",
                    "input": {
                        "id": task.id.clone(),
                        "status": "review",
                        "execution_summary": "Identity regression covered.",
                        "model": "grok-build"
                    }
                }],
                "stop_reason": "tool_use"
            },
            {
                "content": [{ "kind": "text", "text": "done" }],
                "stop_reason": "end_turn"
            }
        ]
    }));
    let _guard = ReplayFixtureGuard::set(fixture.path());
    let audit_dir = tempfile::tempdir().expect("audit tempdir");
    let audit = V2AuditWriter::with_disk_sinks(
        audit_dir.path(),
        Store::open_in_memory().expect("audit store"),
        "ws_test",
        "http-identity-regression",
        format!("claude:{}", orbit_common::test_fixtures::TEST_CLAUDE_MODEL),
        None,
    )
    .expect("audit writer");
    let spec = AgentLoopSpec {
        instruction: "exercise tool identity".to_string(),
        tools: vec!["orbit.task.update".to_string()],
        on_denial: OnDenial::Terminate,
        model: Some(orbit_common::test_fixtures::TEST_CLAUDE_MODEL.to_string()),
        max_iterations: 2,
        backend: Backend::Http,
        provider: Provider::Claude,
        wall_clock_timeout_seconds: 30,
        require_response_envelope: false,
        role: None,
        proc_allowed_programs: None,
    };

    drive_agent_loop(
        &spec,
        None,
        "http-identity-regression",
        audit,
        &serde_json::json!({ "prompt": "update the task" }),
        &runtime,
        None,
    )
    .expect("replay agent loop succeeds");

    let updated = runtime.get_task(&task.id).expect("updated task");
    assert_eq!(updated.implemented_by.as_deref(), Some("claude"));
}

#[test]
fn tool_context_for_activity_passes_proc_allowlist() {
    let (_root, runtime, _repo_root) = runtime_with_workspace_layout();

    // No allowlist -> not activity-scoped (legacy unrestricted path).
    let unscoped = <OrbitRuntime as V2RuntimeHost>::tool_context_for_activity(
        &runtime,
        Some("run-allowlist-test"),
        None,
        None,
        None,
    );
    assert!(unscoped.proc_allowed_programs.is_empty());
    assert!(!unscoped.proc_spawn_activity_scoped);

    // Activity-scoped allowlist propagates verbatim and flips the bool.
    let programs = vec!["git".to_string(), "rg".to_string()];
    let scoped = <OrbitRuntime as V2RuntimeHost>::tool_context_for_activity(
        &runtime,
        Some("run-allowlist-test"),
        None,
        None,
        Some(programs.as_slice()),
    );
    assert_eq!(scoped.proc_allowed_programs, programs);
    assert!(scoped.proc_spawn_activity_scoped);

    // Empty Some([]) is meaningful: fail-closed when activity-scoped.
    let empty_scoped = <OrbitRuntime as V2RuntimeHost>::tool_context_for_activity(
        &runtime,
        Some("run-allowlist-test"),
        None,
        None,
        Some(&[]),
    );
    assert!(empty_scoped.proc_allowed_programs.is_empty());
    assert!(empty_scoped.proc_spawn_activity_scoped);
}
