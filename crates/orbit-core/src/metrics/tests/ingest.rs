use orbit_common::types::{InvocationTrace, KnowledgeRunMetrics, TokenUsage, ToolCallTrace};

use super::super::merge_invocation_trace;

#[test]
fn merge_invocation_trace_records_fs_read_metrics() {
    let trace = InvocationTrace {
        usage: TokenUsage {
            input: 100,
            cache_read: 20,
            cache_create: 5,
            output: 50,
        },
        tool_calls: vec![ToolCallTrace {
            seq: 0,
            tool_name: "fs.read".to_string(),
            result_bytes: 160,
            result_payload: None,
        }],
        duration_ms: 1_234,
    };

    let metrics = merge_invocation_trace(None, &trace).expect("fs.read metrics");

    // `raw_read_token_baseline == actual_fs_read_tokens` for pack-less runs, so
    // the double-read rate is 1.0; pack-specific fields stay at their defaults
    // now that `orbit.graph.pack` is decommissioned.
    assert_eq!(
        metrics,
        KnowledgeRunMetrics {
            raw_read_token_baseline: 40,
            knowledge_pack_tokens: None,
            compression_ratio: None,
            actual_fs_read_tokens_during_run: 40,
            double_read_rate: Some(1.0),
            knowledge_pack_used: false,
            knowledge_pack_unresolved_count: 0,
            total_llm_input_tokens: 125,
        }
    );
}

#[test]
fn merge_invocation_trace_without_measured_tool_or_existing_returns_none() {
    let trace = InvocationTrace {
        usage: TokenUsage {
            input: 100,
            cache_read: 0,
            cache_create: 0,
            output: 10,
        },
        tool_calls: vec![ToolCallTrace {
            seq: 0,
            tool_name: "orbit.task.show".to_string(),
            result_bytes: 80,
            result_payload: None,
        }],
        duration_ms: 5,
    };

    assert!(merge_invocation_trace(None, &trace).is_none());
}
