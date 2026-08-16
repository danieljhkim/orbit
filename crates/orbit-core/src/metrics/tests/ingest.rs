use orbit_types::telemetry::{InvocationTrace, TokenUsage, ToolCallTrace};
use orbit_types::workflow::KnowledgeRunMetrics;

use super::super::merge_invocation_trace;

#[test]
fn merge_invocation_trace_without_existing_returns_none() {
    let trace = InvocationTrace {
        usage: TokenUsage {
            input: 100,
            cache_read: 20,
            cache_create: 5,
            cache_create_1h: 0,
            output: 50,
        },
        tool_calls: vec![ToolCallTrace {
            seq: 0,
            tool_name: "orbit.task.show".to_string(),
            result_bytes: 160,
            result_payload: None,
        }],
        duration_ms: 1_234,
        provider_model: None,
        provider_cost_usd: None,
    };

    assert!(merge_invocation_trace(None, &trace).is_none());
}

#[test]
fn merge_invocation_trace_accumulates_llm_tokens_on_existing_metrics() {
    let existing = KnowledgeRunMetrics {
        raw_read_token_baseline: 40,
        knowledge_pack_tokens: None,
        compression_ratio: None,
        actual_fs_read_tokens_during_run: 40,
        double_read_rate: Some(1.0),
        knowledge_pack_used: false,
        knowledge_pack_unresolved_count: 0,
        total_llm_input_tokens: 125,
    };
    let trace = InvocationTrace {
        usage: TokenUsage {
            input: 10,
            cache_read: 0,
            cache_create: 0,
            cache_create_1h: 0,
            output: 4,
        },
        tool_calls: Vec::new(),
        duration_ms: 5,
        provider_model: None,
        provider_cost_usd: None,
    };

    let metrics = merge_invocation_trace(Some(&existing), &trace).expect("existing metrics");
    assert_eq!(metrics.total_llm_input_tokens, 135);
    assert_eq!(metrics.actual_fs_read_tokens_during_run, 40);
    assert_eq!(metrics.double_read_rate, Some(1.0));
}
