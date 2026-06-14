use orbit_common::types::{InvocationTrace, KnowledgeRunMetrics, ToolCallTrace};
use serde_json::Value;

use super::summary::ratio;

/// Per-trace deltas folded into the persisted [`KnowledgeRunMetrics`].
///
/// The v1 `orbit.graph.pack` compression path was removed with the tool in
/// ORB-00391 (see ORB-00388). Only `fs.read` token accounting remains, which
/// reproduces exactly what the prior implementation computed for pack-less
/// runs (`raw_read_token_baseline == fs_read_tokens`).
#[derive(Debug, Default)]
struct KnowledgeMetricsDelta {
    saw_measured_tool: bool,
    fs_read_tokens: u64,
    total_llm_input_tokens: u64,
}

pub(crate) fn merge_invocation_trace(
    existing: Option<&KnowledgeRunMetrics>,
    trace: &InvocationTrace,
) -> Option<KnowledgeRunMetrics> {
    let delta = KnowledgeMetricsDelta::from_trace(trace);
    if existing.is_none() && !delta.saw_measured_tool {
        return None;
    }

    let mut metrics = existing.cloned().unwrap_or_default();
    metrics.raw_read_token_baseline = metrics
        .raw_read_token_baseline
        .saturating_add(delta.fs_read_tokens);
    metrics.actual_fs_read_tokens_during_run = metrics
        .actual_fs_read_tokens_during_run
        .saturating_add(delta.fs_read_tokens);
    metrics.total_llm_input_tokens = metrics
        .total_llm_input_tokens
        .saturating_add(delta.total_llm_input_tokens);

    metrics.compression_ratio = metrics
        .knowledge_pack_used
        .then(|| {
            ratio(
                metrics.raw_read_token_baseline,
                metrics.knowledge_pack_tokens.unwrap_or(0),
            )
        })
        .flatten();
    metrics.double_read_rate = ratio(
        metrics.actual_fs_read_tokens_during_run,
        metrics.raw_read_token_baseline,
    );

    Some(metrics)
}

impl KnowledgeMetricsDelta {
    fn from_trace(trace: &InvocationTrace) -> Self {
        let mut delta = Self {
            total_llm_input_tokens: trace
                .usage
                .input
                .saturating_add(trace.usage.cache_read)
                .saturating_add(trace.usage.cache_create),
            ..Self::default()
        };

        for call in &trace.tool_calls {
            if call.tool_name.as_str() == "fs.read" {
                delta.observe_fs_read_call(call);
            }
        }

        delta
    }

    fn observe_fs_read_call(&mut self, call: &ToolCallTrace) {
        self.saw_measured_tool = true;
        self.fs_read_tokens = self.fs_read_tokens.saturating_add(tool_result_tokens(call));
    }
}

fn tool_result_tokens(call: &ToolCallTrace) -> u64 {
    call.result_payload
        .as_ref()
        .map(value_token_count)
        .unwrap_or_else(|| bytes_to_token_estimate(call.result_bytes))
}

fn value_token_count(value: &Value) -> u64 {
    let text = match value {
        Value::String(text) => text.clone(),
        other => match serde_json::to_string(other) {
            Ok(text) => text,
            Err(_) => return 0,
        },
    };

    tiktoken_rs::cl100k_base_singleton()
        .encode_with_special_tokens(&text)
        .len() as u64
}

fn bytes_to_token_estimate(bytes: u64) -> u64 {
    bytes.saturating_add(3) / 4
}
