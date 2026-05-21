use std::cmp::Ordering;

use orbit_common::types::{InvocationTrace, JobRun, KnowledgeRunMetrics, ToolCallTrace};
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RatioSummary {
    pub mean: f64,
    pub p50: f64,
    pub p90: f64,
    pub min: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DoubleReadSummary {
    pub mean_rate: f64,
    pub runs_over_fifty_percent: u64,
    pub measured_runs: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TokenInputSummary {
    pub with_pack_avg: f64,
    pub without_pack_avg: f64,
    pub estimated_savings: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct KnowledgeStatsSummary {
    pub total_runs: u64,
    pub pack_runs: u64,
    pub fallback_runs: u64,
    pub fallback_rate: f64,
    pub compression: Option<RatioSummary>,
    pub double_read: DoubleReadSummary,
    pub total_llm_input_tokens: TokenInputSummary,
}

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

pub fn merge_invocation_knowledge_metrics(
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

pub fn aggregate(runs: &[JobRun]) -> KnowledgeStatsSummary {
    let metrics = runs
        .iter()
        .filter_map(|run| run.knowledge_metrics.as_ref())
        .collect::<Vec<_>>();

    let total_runs = metrics.len() as u64;
    let pack_runs = metrics.iter().filter(|m| m.knowledge_pack_used).count() as u64;
    let fallback_runs = total_runs.saturating_sub(pack_runs);
    let fallback_rate = ratio(fallback_runs, total_runs).unwrap_or(0.0);

    let compression_values = metrics
        .iter()
        .filter_map(|m| m.compression_ratio)
        .collect::<Vec<_>>();
    let double_read_values = metrics
        .iter()
        .filter_map(|m| m.double_read_rate)
        .collect::<Vec<_>>();
    let with_pack_tokens = metrics
        .iter()
        .filter(|m| m.knowledge_pack_used)
        .map(|m| m.total_llm_input_tokens as f64)
        .collect::<Vec<_>>();
    let without_pack_tokens = metrics
        .iter()
        .filter(|m| !m.knowledge_pack_used)
        .map(|m| m.total_llm_input_tokens as f64)
        .collect::<Vec<_>>();

    KnowledgeStatsSummary {
        total_runs,
        pack_runs,
        fallback_runs,
        fallback_rate,
        compression: summarize_ratios(&compression_values),
        double_read: DoubleReadSummary {
            mean_rate: mean(&double_read_values),
            runs_over_fifty_percent: double_read_values
                .iter()
                .filter(|value| **value > 0.5)
                .count() as u64,
            measured_runs: double_read_values.len() as u64,
        },
        total_llm_input_tokens: TokenInputSummary {
            with_pack_avg: mean(&with_pack_tokens),
            without_pack_avg: mean(&without_pack_tokens),
            estimated_savings: estimate_savings(
                mean(&with_pack_tokens),
                mean(&without_pack_tokens),
            ),
        },
    }
}

fn summarize_ratios(values: &[f64]) -> Option<RatioSummary> {
    if values.is_empty() {
        return None;
    }

    let mut sorted = values.to_vec();
    sorted.sort_by(|left, right| left.partial_cmp(right).unwrap_or(Ordering::Equal));
    Some(RatioSummary {
        mean: mean(&sorted),
        p50: percentile(&sorted, 50),
        p90: percentile(&sorted, 90),
        min: sorted[0],
    })
}

fn estimate_savings(with_pack_avg: f64, without_pack_avg: f64) -> Option<f64> {
    (without_pack_avg > 0.0).then_some(1.0 - (with_pack_avg / without_pack_avg))
}

fn mean(values: &[f64]) -> f64 {
    if values.is_empty() {
        return 0.0;
    }
    values.iter().sum::<f64>() / values.len() as f64
}

fn percentile(sorted: &[f64], pct: usize) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    let rank = ((pct as f64 / 100.0) * sorted.len() as f64).ceil() as usize;
    let index = rank.saturating_sub(1).min(sorted.len() - 1);
    sorted[index]
}

fn ratio(numerator: u64, denominator: u64) -> Option<f64> {
    (denominator != 0).then_some(numerator as f64 / denominator as f64)
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
            self.explicit_raw_read_baseline = self
                .explicit_raw_read_baseline
                .saturating_add(sum_metric_fields(payload, RAW_BASELINE_KEYS));
            self.explicit_pack_tokens = self
                .explicit_pack_tokens
                .saturating_add(sum_metric_fields(payload, PACK_TOKEN_KEYS));
            self.unresolved_count = self
                .unresolved_count
                .saturating_add(count_unresolved_selectors(payload));
            self.resolved_pack_entries = self
                .resolved_pack_entries
                .saturating_add(count_pack_entries(payload));
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

const RAW_BASELINE_KEYS: &[&str] = &[
    "raw_read_token_baseline",
    "rawReadTokenBaseline",
    "raw_read_tokens",
    "rawReadTokens",
    "baseline_tokens",
    "baselineTokens",
    "source_tokens",
    "sourceTokens",
];

const PACK_TOKEN_KEYS: &[&str] = &[
    "knowledge_pack_tokens",
    "knowledgePackTokens",
    "pack_tokens",
    "packTokens",
    "compressed_tokens",
    "compressedTokens",
];

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

fn sum_metric_fields(value: &Value, keys: &[&str]) -> u64 {
    match value {
        Value::Object(map) => {
            let direct = keys
                .iter()
                .filter_map(|key| map.get(*key))
                .filter_map(value_as_u64)
                .fold(0u64, u64::saturating_add);
            if direct > 0 {
                return direct;
            }
            map.values()
                .map(|child| sum_metric_fields(child, keys))
                .fold(0u64, u64::saturating_add)
        }
        Value::Array(items) => items
            .iter()
            .map(|child| sum_metric_fields(child, keys))
            .fold(0u64, u64::saturating_add),
        _ => 0,
    }
}

fn count_unresolved_selectors(value: &Value) -> u32 {
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
                        value_as_u64(child).unwrap_or(0)
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

fn count_pack_entries(value: &Value) -> u64 {
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
                .map(count_pack_entries)
                .fold(0u64, u64::saturating_add)
        }
        Value::Array(items) => items
            .iter()
            .map(count_pack_entries)
            .fold(0u64, u64::saturating_add),
        _ => 0,
    }
}

fn value_as_u64(value: &Value) -> Option<u64> {
    match value {
        Value::Number(number) => number.as_u64(),
        Value::String(raw) => raw.parse::<u64>().ok(),
        _ => None,
    }
}
