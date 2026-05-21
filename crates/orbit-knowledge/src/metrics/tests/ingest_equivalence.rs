use orbit_common::types::{InvocationTrace, KnowledgeRunMetrics, TokenUsage, ToolCallTrace};
use serde_json::{Value, json};

use super::merge_invocation_trace;
use crate::KnowledgePackResult;

#[test]
fn representative_traces_match_legacy_value_walking_path() {
    for trace in representative_traces() {
        let legacy = legacy_merge_invocation_trace(None, &trace);
        let typed = merge_invocation_trace(None, &trace);
        assert_eq!(
            serde_json::to_vec(&legacy).expect("serialize legacy metrics"),
            serde_json::to_vec(&typed).expect("serialize typed metrics")
        );
    }
}

fn representative_traces() -> Vec<InvocationTrace> {
    vec![
        trace(
            100,
            vec![payload_call(
                1,
                "orbit.graph.pack",
                pack_payload(1_000, 250, vec![entry("file:src/lib.rs", "file")], vec![]),
            )],
        ),
        trace(80, vec![byte_call(1, "fs.read", 320)]),
        trace(
            120,
            vec![
                payload_call(
                    1,
                    "orbit.graph.pack",
                    pack_payload(900, 300, vec![entry("file:src/main.rs", "file")], vec![]),
                ),
                byte_call(2, "fs.read", 160),
            ],
        ),
        trace(
            150,
            vec![
                payload_call(
                    1,
                    "orbit.graph.pack",
                    pack_payload(
                        700,
                        175,
                        vec![
                            entry("file:src/lib.rs", "file"),
                            entry("file:src/missing.rs", "unresolved"),
                        ],
                        vec!["file:src/missing.rs"],
                    ),
                ),
                byte_call(2, "fs.read", 200),
            ],
        ),
        trace(
            210,
            vec![
                payload_call(
                    1,
                    "orbit.graph.pack",
                    pack_payload(600, 150, vec![entry("file:src/a.rs", "file")], vec![]),
                ),
                payload_call(
                    2,
                    "orbit.graph.pack",
                    pack_payload(
                        400,
                        100,
                        vec![entry("file:src/b.rs", "file")],
                        vec!["file:src/missing.rs"],
                    ),
                ),
                byte_call(3, "fs.read", 120),
            ],
        ),
    ]
}

fn trace(input_tokens: u64, tool_calls: Vec<ToolCallTrace>) -> InvocationTrace {
    InvocationTrace {
        usage: TokenUsage {
            input: input_tokens,
            cache_read: 10,
            cache_create: 5,
            output: 25,
        },
        tool_calls,
        duration_ms: 42,
    }
}

fn payload_call(seq: u32, tool_name: &str, payload: Value) -> ToolCallTrace {
    ToolCallTrace {
        seq,
        tool_name: tool_name.to_string(),
        result_bytes: serde_json::to_vec(&payload)
            .expect("serialize payload")
            .len() as u64,
        result_payload: Some(payload),
    }
}

fn byte_call(seq: u32, tool_name: &str, result_bytes: u64) -> ToolCallTrace {
    ToolCallTrace {
        seq,
        tool_name: tool_name.to_string(),
        result_bytes,
        result_payload: None,
    }
}

fn pack_payload(
    raw_read_token_baseline: u64,
    knowledge_pack_tokens: u64,
    entries: Vec<Value>,
    unresolved_selectors: Vec<&str>,
) -> Value {
    json!({
        "raw_read_token_baseline": raw_read_token_baseline,
        "knowledge_pack_tokens": knowledge_pack_tokens,
        "knowledge_dir": "/tmp/orbit/knowledge",
        "manifest_generated_at": "2026-05-21T00:00:00Z",
        "unresolved_selectors": unresolved_selectors,
        "total_nodes": entries.len(),
        "entries": entries,
    })
}

fn entry(selector: &str, kind: &str) -> Value {
    json!({
        "selector": selector,
        "kind": kind,
    })
}

#[derive(Debug, Default)]
struct LegacyKnowledgeMetricsDelta {
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

fn legacy_merge_invocation_trace(
    existing: Option<&KnowledgeRunMetrics>,
    trace: &InvocationTrace,
) -> Option<KnowledgeRunMetrics> {
    let delta = LegacyKnowledgeMetricsDelta::from_trace(trace);
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
            legacy_ratio(
                metrics.raw_read_token_baseline,
                metrics.knowledge_pack_tokens.unwrap_or(0),
            )
        })
        .flatten();
    metrics.double_read_rate = legacy_ratio(
        metrics.actual_fs_read_tokens_during_run,
        metrics.raw_read_token_baseline,
    );

    Some(metrics)
}

impl LegacyKnowledgeMetricsDelta {
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
        let estimated_tokens = legacy_tool_result_tokens(call);
        self.estimated_pack_tokens = self.estimated_pack_tokens.saturating_add(estimated_tokens);

        if let Some(payload) = call.result_payload.as_ref() {
            self.explicit_raw_read_baseline = self
                .explicit_raw_read_baseline
                .saturating_add(legacy_sum_metric_fields(payload, LEGACY_RAW_BASELINE_KEYS));
            self.explicit_pack_tokens = self
                .explicit_pack_tokens
                .saturating_add(legacy_sum_metric_fields(payload, LEGACY_PACK_TOKEN_KEYS));
            self.unresolved_count = self
                .unresolved_count
                .saturating_add(legacy_count_unresolved_selectors(payload));
            self.resolved_pack_entries = self
                .resolved_pack_entries
                .saturating_add(legacy_count_pack_entries(payload));
        }
    }

    fn observe_fs_read_call(&mut self, call: &ToolCallTrace) {
        self.saw_knowledge_tool = true;
        self.fs_read_tokens = self
            .fs_read_tokens
            .saturating_add(legacy_tool_result_tokens(call));
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

const LEGACY_RAW_BASELINE_KEYS: &[&str] = &[
    "raw_read_token_baseline",
    "rawReadTokenBaseline",
    "raw_read_tokens",
    "rawReadTokens",
    "baseline_tokens",
    "baselineTokens",
    "source_tokens",
    "sourceTokens",
];

const LEGACY_PACK_TOKEN_KEYS: &[&str] = &[
    "knowledge_pack_tokens",
    "knowledgePackTokens",
    "pack_tokens",
    "packTokens",
    "compressed_tokens",
    "compressedTokens",
];

fn legacy_tool_result_tokens(call: &ToolCallTrace) -> u64 {
    call.result_payload
        .as_ref()
        .map(legacy_value_token_count)
        .unwrap_or_else(|| legacy_bytes_to_token_estimate(call.result_bytes))
}

fn legacy_value_token_count(value: &Value) -> u64 {
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

fn legacy_bytes_to_token_estimate(bytes: u64) -> u64 {
    bytes.saturating_add(3) / 4
}

fn legacy_sum_metric_fields(value: &Value, keys: &[&str]) -> u64 {
    match value {
        Value::Object(map) => {
            let direct = keys
                .iter()
                .filter_map(|key| map.get(*key))
                .filter_map(legacy_value_as_u64)
                .fold(0u64, u64::saturating_add);
            if direct > 0 {
                return direct;
            }
            map.values()
                .map(|child| legacy_sum_metric_fields(child, keys))
                .fold(0u64, u64::saturating_add)
        }
        Value::Array(items) => items
            .iter()
            .map(|child| legacy_sum_metric_fields(child, keys))
            .fold(0u64, u64::saturating_add),
        _ => 0,
    }
}

fn legacy_count_unresolved_selectors(value: &Value) -> u32 {
    fn count(value: &Value) -> u64 {
        match value {
            Value::Object(map) => map
                .iter()
                .map(|(key, child)| {
                    if matches!(key.as_str(), "unresolved_selectors" | "unresolvedSelectors") {
                        child
                            .as_array()
                            .map(|items| items.len() as u64)
                            .unwrap_or(0)
                    } else if matches!(
                        key.as_str(),
                        "knowledge_pack_unresolved_count"
                            | "knowledgePackUnresolvedCount"
                            | "unresolved_count"
                            | "unresolvedCount"
                    ) {
                        legacy_value_as_u64(child).unwrap_or(0)
                    } else {
                        count(child)
                    }
                })
                .fold(0u64, u64::saturating_add),
            Value::Array(items) => items.iter().map(count).fold(0u64, u64::saturating_add),
            _ => 0,
        }
    }

    count(value).min(u32::MAX as u64) as u32
}

fn legacy_count_pack_entries(value: &Value) -> u64 {
    match value {
        Value::Object(map) => {
            let direct = map
                .get("entries")
                .and_then(Value::as_array)
                .map(|items| items.len() as u64)
                .unwrap_or(0);
            if direct > 0 {
                return direct;
            }
            map.values()
                .map(legacy_count_pack_entries)
                .fold(0u64, u64::saturating_add)
        }
        Value::Array(items) => items
            .iter()
            .map(legacy_count_pack_entries)
            .fold(0u64, u64::saturating_add),
        _ => 0,
    }
}

fn legacy_value_as_u64(value: &Value) -> Option<u64> {
    match value {
        Value::Number(number) => number.as_u64(),
        Value::String(raw) => raw.parse::<u64>().ok(),
        _ => None,
    }
}

fn legacy_ratio(numerator: u64, denominator: u64) -> Option<f64> {
    if denominator == 0 {
        None
    } else {
        Some(numerator as f64 / denominator as f64)
    }
}

#[test]
fn typed_pack_result_rejects_camel_case_metric_aliases() {
    let payload = json!({
        "rawReadTokenBaseline": 100,
        "knowledgePackTokens": 50,
        "entries": [],
        "unresolvedSelectors": [],
    });

    let parsed = serde_json::from_value::<KnowledgePackResult>(payload)
        .expect("typed pack result decodes with unknown aliases ignored");

    assert_eq!(parsed.raw_read_token_baseline, 0);
    assert_eq!(parsed.knowledge_pack_tokens, 0);
    assert!(parsed.unresolved_selectors.is_empty());
}
