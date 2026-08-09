//! Sibling tests for `mod.rs` (the v2_host module root; migrated per ORB-10387 /
//! docs/design-patterns/test_layout.md).

use orbit_common::types::activity_job::Provider;
use orbit_common::types::{InvocationTrace, JobRunState, TokenUsage, ToolCallTrace};
use orbit_store::InvocationQuery;

use super::super::test_support::runtime_with_workspace_layout;
use super::super::*;

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

fn runtime_with_recovery_config(config: &str) -> (tempfile::TempDir, OrbitRuntime) {
    let root = tempfile::tempdir().expect("create tempdir");
    let global = root.path().join("home/.orbit");
    let workspace = root.path().join("repo/.orbit");
    std::fs::create_dir_all(&global).expect("global orbit dir");
    std::fs::create_dir_all(&workspace).expect("workspace orbit dir");
    std::fs::write(workspace.join("config.toml"), config).expect("write recovery config");
    let runtime = OrbitRuntime::from_roots(&global, &workspace).expect("build runtime");
    (root, runtime)
}

#[test]
fn recovery_crew_maps_persisted_codex_and_claude_lanes_to_middleweight_models() {
    let (_root, runtime, _repo_root) = runtime_with_workspace_layout();

    for (source_crew, expected_provider, expected_model) in [
        ("sol", Provider::Codex, "gpt-5.6-terra"),
        ("terra", Provider::Codex, "gpt-5.6-terra"),
        ("luna", Provider::Codex, "gpt-5.6-terra"),
        ("opus", Provider::Claude, "sonnet"),
        ("sonnet", Provider::Claude, "sonnet"),
        ("fable", Provider::Claude, "sonnet"),
    ] {
        let run_id = seed_running_job_run(&runtime, "recovery_lane_job");
        runtime
            .record_run_crew_from_input(&run_id, &serde_json::json!({ "crew": source_crew }))
            .expect("persist source crew");

        let config = V2RuntimeHost::recovery_agent_crew_config(&runtime, &run_id)
            .expect("resolve lane-local middleweight recovery crew");
        assert_eq!(
            config.provider,
            Some(expected_provider),
            "source {source_crew}"
        );
        assert_eq!(
            config.model.as_deref(),
            Some(expected_model),
            "source {source_crew}"
        );
    }
}

#[test]
fn recovery_crew_missing_or_cross_provider_config_is_an_actionable_error() {
    let missing_terra = r#"
[workflow]
default_crew = "sol"

[crews.sol]
model = "gpt-5.6-sol"
provider = "codex"
backend = "cli"
"#;
    let (_root, runtime) = runtime_with_recovery_config(missing_terra);
    let run_id = seed_running_job_run(&runtime, "recovery_missing_crew");
    runtime
        .record_run_crew_from_input(&run_id, &serde_json::json!({ "crew": "sol" }))
        .expect("persist source crew");
    let err = V2RuntimeHost::recovery_agent_crew_config(&runtime, &run_id)
        .expect_err("missing terra must not fall back to another provider or model");
    assert!(err.to_string().contains("provider `codex` crew `terra`"));

    let malformed_terra = r#"
[workflow]
default_crew = "sol"

[crews.sol]
model = "gpt-5.6-sol"
provider = "codex"
backend = "cli"

[crews.terra]
model = "sonnet"
provider = "claude"
backend = "cli"
"#;
    let (_root, runtime) = runtime_with_recovery_config(malformed_terra);
    let run_id = seed_running_job_run(&runtime, "recovery_malformed_crew");
    runtime
        .record_run_crew_from_input(&run_id, &serde_json::json!({ "crew": "sol" }))
        .expect("persist source crew");
    let err = V2RuntimeHost::recovery_agent_crew_config(&runtime, &run_id)
        .expect_err("cross-provider terra must fail closed");
    assert!(err.to_string().contains("provider `codex` crew `terra`"));
}

#[test]
fn recovery_invocation_persists_the_selected_middleweight_provider_and_model() {
    let (_root, runtime, _repo_root) = runtime_with_workspace_layout();
    let run_id = seed_running_job_run(&runtime, "recovery_telemetry_job");
    runtime
        .record_run_crew_from_input(&run_id, &serde_json::json!({ "crew": "sol" }))
        .expect("persist source crew");
    let recovery = V2RuntimeHost::recovery_agent_crew_config(&runtime, &run_id)
        .expect("resolve terra recovery crew");

    V2RuntimeHost::persist_invocation_trace(
        &runtime,
        &run_id,
        "step_failure_recovery",
        recovery.provider.expect("validated provider").as_str(),
        recovery.model.as_deref(),
        &serde_json::json!({ "task_id": "ORB-10615" }),
        &InvocationTrace::default(),
    )
    .expect("persist recovery invocation");

    let records = runtime
        .invocation_records(InvocationQuery {
            job_run_id: Some(run_id),
            limit: 1,
            ..InvocationQuery::default()
        })
        .expect("query recovery invocation");
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].agent, "codex");
    assert_eq!(records[0].model.as_deref(), Some("gpt-5.6-terra"));
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
