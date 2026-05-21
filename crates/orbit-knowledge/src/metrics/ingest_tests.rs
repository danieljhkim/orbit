use orbit_common::types::{InvocationTrace, KnowledgeRunMetrics, TokenUsage, ToolCallTrace};
use serde_json::json;

use super::merge_invocation_trace;

#[test]
fn merge_invocation_trace_records_graph_pack_and_fs_read_metrics() {
    let trace = InvocationTrace {
        usage: TokenUsage {
            input: 100,
            cache_read: 20,
            cache_create: 5,
            output: 50,
        },
        tool_calls: vec![
            ToolCallTrace {
                seq: 0,
                tool_name: "orbit.graph.pack".to_string(),
                result_bytes: 400,
                result_payload: Some(json!({
                    "entries": [
                        {
                            "selector": "file:src/lib.rs",
                            "kind": "file"
                        }
                    ],
                    "raw_read_token_baseline": 1_000,
                    "knowledge_pack_tokens": 250,
                    "unresolved_selectors": []
                })),
            },
            ToolCallTrace {
                seq: 1,
                tool_name: "fs.read".to_string(),
                result_bytes: 160,
                result_payload: None,
            },
        ],
        duration_ms: 1_234,
    };

    let metrics = merge_invocation_trace(None, &trace).expect("knowledge metrics");

    assert_eq!(
        metrics,
        KnowledgeRunMetrics {
            raw_read_token_baseline: 1_000,
            knowledge_pack_tokens: Some(250),
            compression_ratio: Some(4.0),
            actual_fs_read_tokens_during_run: 40,
            double_read_rate: Some(0.04),
            knowledge_pack_used: true,
            knowledge_pack_unresolved_count: 0,
            total_llm_input_tokens: 125,
        }
    );
}
