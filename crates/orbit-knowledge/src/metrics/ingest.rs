use orbit_common::types::{InvocationTrace, KnowledgeRunMetrics, ToolCallTrace};
use serde_json::Value;

use crate::KnowledgePackResult;

use super::summary::ratio;

#[derive(Debug, Default)]
struct KnowledgeMetricsDelta {
    saw_knowledge_tool: bool,
    saw_pack: bool,
    resolved_pack_entries: u64,
    explicit_raw_read_baseline: u64,
    explicit_pack_tokens: u64,
    estimated_pack_tokens: u64,
    fs_read_tokens: u64,
    unresolved_count: u32,
    total_llm_input_tokens: u64,
}

pub fn merge_invocation_trace(
    existing: Option<&KnowledgeRunMetrics>,
    trace: &InvocationTrace,
) -> Option<KnowledgeRunMetrics> {
    let delta = KnowledgeMetricsDelta::from_trace(trace);
    if existing.is_none() && !delta.saw_knowledge_tool {
        return None;
    }

    let mut metrics = existing.cloned().unwrap_or_default();
    metrics.raw_read_token_baseline = metrics
        .raw_read_token_baseline
        .saturating_add(delta.raw_read_token_baseline());
    metrics.actual_fs_read_tokens_during_run = metrics
        .actual_fs_read_tokens_during_run
        .saturating_add(delta.fs_read_tokens);
    metrics.knowledge_pack_used = metrics.knowledge_pack_used
        || (delta.saw_pack && (delta.resolved_pack_entries > 0 || delta.unresolved_count == 0));
    metrics.knowledge_pack_unresolved_count = metrics
        .knowledge_pack_unresolved_count
        .saturating_add(delta.unresolved_count);
    metrics.total_llm_input_tokens = metrics
        .total_llm_input_tokens
        .saturating_add(delta.total_llm_input_tokens);

    if delta.saw_pack || metrics.knowledge_pack_tokens.is_some() {
        let current = metrics.knowledge_pack_tokens.unwrap_or_default();
        metrics.knowledge_pack_tokens = Some(current.saturating_add(delta.knowledge_pack_tokens()));
    }

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
            match call.tool_name.as_str() {
                "orbit.graph.pack" => delta.observe_pack_call(call),
                "fs.read" => delta.observe_fs_read_call(call),
                _ => {}
            }
        }

        delta
    }

    fn observe_pack_call(&mut self, call: &ToolCallTrace) {
        self.saw_knowledge_tool = true;
        self.saw_pack = true;
        let estimated_tokens = tool_result_tokens(call);
        self.estimated_pack_tokens = self.estimated_pack_tokens.saturating_add(estimated_tokens);

        if let Some(payload) = call.result_payload.as_ref() {
            match serde_json::from_value::<KnowledgePackResult>(payload.clone()) {
                Ok(pack) => {
                    self.explicit_raw_read_baseline = self
                        .explicit_raw_read_baseline
                        .saturating_add(pack.raw_read_token_baseline);
                    self.explicit_pack_tokens = self
                        .explicit_pack_tokens
                        .saturating_add(pack.knowledge_pack_tokens);
                    self.unresolved_count = self.unresolved_count.saturating_add(
                        pack.unresolved_selectors.len().min(u32::MAX as usize) as u32,
                    );
                    self.resolved_pack_entries = self
                        .resolved_pack_entries
                        .saturating_add(pack.entries.len() as u64);
                }
                Err(error) => {
                    tracing::warn!(
                        error = %error,
                        "failed to decode orbit.graph.pack result payload for knowledge metrics"
                    );
                }
            }
        }
    }

    fn observe_fs_read_call(&mut self, call: &ToolCallTrace) {
        self.saw_knowledge_tool = true;
        self.fs_read_tokens = self.fs_read_tokens.saturating_add(tool_result_tokens(call));
    }

    fn knowledge_pack_tokens(&self) -> u64 {
        if self.explicit_pack_tokens > 0 {
            self.explicit_pack_tokens
        } else {
            self.estimated_pack_tokens
        }
    }

    fn raw_read_token_baseline(&self) -> u64 {
        if !self.saw_pack {
            return self.fs_read_tokens;
        }

        let pack_baseline = if self.explicit_raw_read_baseline > 0 {
            self.explicit_raw_read_baseline
        } else {
            self.knowledge_pack_tokens()
        };

        if self.unresolved_count > 0 {
            pack_baseline.saturating_add(self.fs_read_tokens)
        } else {
            pack_baseline.max(self.fs_read_tokens)
        }
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
