use orbit_common::types::TokenUsage;
use serde_json::Value;

use super::JsonMap;

const USAGE_CHILD_KEYS: &[&str] = &[
    "usage",
    "token_usage",
    "tokenUsage",
    "tokens",
    "usageMetadata",
    "usage_metadata",
];

#[derive(Clone, Copy)]
enum UsageKeyMode {
    Standard,
    TokenBlock,
}

pub(super) fn sum_usage(documents: &[Value]) -> TokenUsage {
    let mut usage = TokenUsage::default();
    for document in documents {
        collect_usage(document, &mut usage, true, UsageKeyMode::Standard);
    }
    usage
}

fn collect_usage(
    value: &Value,
    usage: &mut TokenUsage,
    allow_direct_usage: bool,
    key_mode: UsageKeyMode,
) {
    match value {
        Value::Object(map) => {
            if allow_direct_usage && let Some(found) = usage_from_map(map, key_mode) {
                add_usage(usage, found);
                return;
            }

            if matches!(map.get("type").and_then(Value::as_str), Some("tool_result")) {
                return;
            }

            let has_model_token_usage = map
                .get("tokens")
                .and_then(Value::as_object)
                .and_then(|tokens| usage_from_map(tokens, UsageKeyMode::TokenBlock))
                .is_some();

            for &key in USAGE_CHILD_KEYS {
                if let Some(mode) = usage_key_mode(key)
                    && let Some(child) = map.get(key)
                {
                    collect_usage(child, usage, true, mode);
                }
            }

            for (key, child) in map {
                if key != "tool_calls"
                    && usage_key_mode(key).is_none()
                    && !(has_model_token_usage && key == "roles")
                {
                    let allow_child = allow_direct_usage
                        || matches!(
                            key.as_str(),
                            "text"
                                | "result"
                                | "response"
                                | "message"
                                | "messages"
                                | "content"
                                | "final"
                                | "final_message"
                                | "output"
                        );
                    collect_usage(child, usage, allow_child, UsageKeyMode::Standard);
                }
            }
        }
        Value::Array(items) => {
            for item in items {
                collect_usage(item, usage, allow_direct_usage, key_mode);
            }
        }
        Value::String(raw) => {
            if allow_direct_usage && let Ok(nested) = serde_json::from_str::<Value>(raw) {
                collect_usage(&nested, usage, true, key_mode);
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

fn usage_key_mode(key: &str) -> Option<UsageKeyMode> {
    match key {
        "tokens" => Some(UsageKeyMode::TokenBlock),
        "usage" | "token_usage" | "tokenUsage" | "usageMetadata" | "usage_metadata" => {
            Some(UsageKeyMode::Standard)
        }
        _ => None,
    }
}

fn usage_from_map(map: &JsonMap, key_mode: UsageKeyMode) -> Option<TokenUsage> {
    let input = match key_mode {
        UsageKeyMode::Standard => first_u64(map, STANDARD_INPUT_KEYS),
        UsageKeyMode::TokenBlock => first_u64(map, TOKEN_BLOCK_INPUT_KEYS),
    };
    let cache_read = match key_mode {
        UsageKeyMode::Standard => first_u64(map, STANDARD_CACHE_READ_KEYS),
        UsageKeyMode::TokenBlock => first_u64(map, TOKEN_BLOCK_CACHE_READ_KEYS),
    };
    let cache_create = match key_mode {
        UsageKeyMode::Standard => first_u64(map, STANDARD_CACHE_CREATE_KEYS),
        UsageKeyMode::TokenBlock => first_u64(map, STANDARD_CACHE_CREATE_KEYS),
    };
    let output = match key_mode {
        UsageKeyMode::Standard => first_u64(map, STANDARD_OUTPUT_KEYS),
        UsageKeyMode::TokenBlock => first_u64(map, TOKEN_BLOCK_OUTPUT_KEYS),
    };

    input.or(cache_read).or(cache_create).or(output)?;

    Some(TokenUsage {
        input: input.unwrap_or(0),
        cache_read: cache_read.unwrap_or(0),
        cache_create: cache_create.unwrap_or(0),
        output: output.unwrap_or(0),
    })
}

const STANDARD_INPUT_KEYS: &[&str] = &[
    "input_tokens",
    "inputTokens",
    "prompt_tokens",
    "promptTokens",
    "promptTokenCount",
    "prompt_token_count",
];

const TOKEN_BLOCK_INPUT_KEYS: &[&str] = &[
    "input_tokens",
    "inputTokens",
    "prompt_tokens",
    "promptTokens",
    "promptTokenCount",
    "prompt_token_count",
    "input",
    "prompt",
];

const STANDARD_CACHE_READ_KEYS: &[&str] = &[
    "cache_read_input_tokens",
    "cacheReadInputTokens",
    "cache_read_tokens",
    "cacheReadTokens",
    "cached_input_tokens",
    "cachedInputTokens",
    "cachedContentTokenCount",
    "cached_content_token_count",
];

const TOKEN_BLOCK_CACHE_READ_KEYS: &[&str] = &[
    "cache_read_input_tokens",
    "cacheReadInputTokens",
    "cache_read_tokens",
    "cacheReadTokens",
    "cached_input_tokens",
    "cachedInputTokens",
    "cachedContentTokenCount",
    "cached_content_token_count",
    "cached",
];

const STANDARD_CACHE_CREATE_KEYS: &[&str] = &[
    "cache_creation_input_tokens",
    "cacheCreationInputTokens",
    "cache_create_tokens",
    "cacheCreateTokens",
];

const STANDARD_OUTPUT_KEYS: &[&str] = &[
    "output_tokens",
    "outputTokens",
    "completion_tokens",
    "completionTokens",
    "candidatesTokenCount",
    "candidates_token_count",
];

const TOKEN_BLOCK_OUTPUT_KEYS: &[&str] = &[
    "output_tokens",
    "outputTokens",
    "completion_tokens",
    "completionTokens",
    "candidatesTokenCount",
    "candidates_token_count",
    "candidates",
    "output",
];

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

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn gemini_cli_model_token_blocks_are_summed_once_per_model() {
        let documents = vec![json!({
            "stats": {
                "models": {
                    "gemini-3.1-pro": {
                        "tokens": {
                            "input": 10,
                            "cached": 2,
                            "candidates": 4,
                            "total": 999,
                            "thoughts": 70,
                            "tool": 30
                        },
                        "roles": {
                            "user": {
                                "tokens": {
                                    "input": 10,
                                    "cached": 2
                                }
                            },
                            "model": {
                                "tokens": {
                                    "candidates": 4
                                }
                            }
                        }
                    },
                    "gemini-2.5-flash": {
                        "tokens": {
                            "prompt": 20,
                            "cached": "3",
                            "output": "5",
                            "total": 28
                        },
                        "roles": {
                            "user": {
                                "tokens": {
                                    "prompt": 20
                                }
                            },
                            "model": {
                                "tokens": {
                                    "output": 5
                                }
                            }
                        }
                    }
                }
            }
        })];

        assert_eq!(
            sum_usage(&documents),
            TokenUsage {
                input: 30,
                cache_read: 5,
                cache_create: 0,
                output: 9,
            }
        );
    }

    #[test]
    fn gemini_cli_role_tokens_are_counted_when_model_tokens_are_absent() {
        let documents = vec![json!({
            "stats": {
                "models": {
                    "gemini-3.1-pro": {
                        "roles": {
                            "user": {
                                "tokens": {
                                    "input": 7,
                                    "cached": 1
                                }
                            },
                            "model": {
                                "tokens": {
                                    "candidates": 3
                                }
                            }
                        }
                    }
                }
            }
        })];

        assert_eq!(
            sum_usage(&documents),
            TokenUsage {
                input: 7,
                cache_read: 1,
                cache_create: 0,
                output: 3,
            }
        );
    }

    #[test]
    fn gemini_cli_total_thoughts_and_tool_are_not_folded_into_usage() {
        let documents = vec![json!({
            "stats": {
                "models": {
                    "gemini-3.1-pro": {
                        "tokens": {
                            "total": 999,
                            "thoughts": 70,
                            "tool": 30
                        }
                    }
                }
            }
        })];

        // TokenUsage has no thoughts/tool fields yet, so these Gemini-only
        // counts are intentionally ignored rather than mixed into I/O totals.
        assert_eq!(sum_usage(&documents), TokenUsage::default());
    }
}
