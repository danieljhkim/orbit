use orbit_types::TokenUsage;
use serde_json::Value;

use super::JsonMap;

pub(super) fn sum_usage(documents: &[Value]) -> TokenUsage {
    let mut usage = TokenUsage::default();
    for document in documents {
        collect_usage(document, &mut usage, true);
    }
    usage
}

fn collect_usage(value: &Value, usage: &mut TokenUsage, allow_direct_usage: bool) {
    match value {
        Value::Object(map) => {
            if allow_direct_usage && let Some(found) = usage_from_map(map) {
                add_usage(usage, found);
                return;
            }

            if matches!(map.get("type").and_then(Value::as_str), Some("tool_result")) {
                return;
            }

            for key in ["usage", "token_usage", "tokenUsage", "tokens"] {
                if let Some(child) = map.get(key) {
                    collect_usage(child, usage, true);
                }
            }

            for (key, child) in map {
                if key != "tool_calls"
                    && key != "usage"
                    && key != "token_usage"
                    && key != "tokenUsage"
                    && key != "tokens"
                {
                    collect_usage(child, usage, false);
                }
            }
        }
        Value::Array(items) => {
            for item in items {
                collect_usage(item, usage, allow_direct_usage);
            }
        }
        Value::String(raw) => {
            if allow_direct_usage && let Ok(nested) = serde_json::from_str::<Value>(raw) {
                collect_usage(&nested, usage, true);
            }
        }
        _ => {}
    }
}

fn add_usage(usage: &mut TokenUsage, found: TokenUsage) {
    usage.input = usage.input.saturating_add(found.input);
    usage.cache_read = usage.cache_read.saturating_add(found.cache_read);
    usage.cache_create = usage.cache_create.saturating_add(found.cache_create);
    usage.output = usage.output.saturating_add(found.output);
}

fn usage_from_map(map: &JsonMap) -> Option<TokenUsage> {
    let input = first_u64(
        map,
        &[
            "input_tokens",
            "inputTokens",
            "prompt_tokens",
            "promptTokens",
        ],
    );
    let cache_read = first_u64(
        map,
        &[
            "cache_read_input_tokens",
            "cacheReadInputTokens",
            "cache_read_tokens",
            "cacheReadTokens",
            "cached_input_tokens",
            "cachedInputTokens",
        ],
    );
    let cache_create = first_u64(
        map,
        &[
            "cache_creation_input_tokens",
            "cacheCreationInputTokens",
            "cache_create_tokens",
            "cacheCreateTokens",
        ],
    );
    let output = first_u64(
        map,
        &[
            "output_tokens",
            "outputTokens",
            "completion_tokens",
            "completionTokens",
        ],
    );

    input.or(cache_read).or(cache_create).or(output)?;

    Some(TokenUsage {
        input: input.unwrap_or(0),
        cache_read: cache_read.unwrap_or(0),
        cache_create: cache_create.unwrap_or(0),
        output: output.unwrap_or(0),
    })
}

fn first_u64(map: &JsonMap, keys: &[&str]) -> Option<u64> {
    keys.iter().find_map(|key| value_as_u64(map.get(*key)?))
}

pub(super) fn value_as_u64(value: &Value) -> Option<u64> {
    match value {
        Value::Number(number) => number.as_u64(),
        Value::String(raw) => raw.parse::<u64>().ok(),
        _ => None,
    }
}
