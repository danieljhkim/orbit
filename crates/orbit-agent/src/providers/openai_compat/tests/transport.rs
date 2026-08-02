use super::super::chat_completions_transport::turn_usage_from_wire;
use super::super::wire::IncomingUsage;

#[test]
fn response_usage_retains_cached_reads_and_standard_cache_writes() {
    let usage: IncomingUsage = serde_json::from_value(serde_json::json!({
        "prompt_tokens": 1_000,
        "completion_tokens": 50,
        "prompt_tokens_details": {
            "cached_tokens": 200,
            "cache_write_tokens": 300
        }
    }))
    .expect("valid OpenAI-compatible usage");

    let mapped = turn_usage_from_wire(usage);
    assert_eq!(mapped.input_tokens, 1_000);
    assert_eq!(mapped.cache_read_input_tokens, 200);
    assert_eq!(mapped.cache_creation_input_tokens, 300);
    assert_eq!(mapped.output_tokens, 50);
}

#[test]
fn response_usage_accepts_cache_creation_alias() {
    let usage: IncomingUsage = serde_json::from_value(serde_json::json!({
        "prompt_tokens_details": {
            "cache_creation_tokens": 17
        }
    }))
    .expect("valid compatibility-layer usage");

    assert_eq!(turn_usage_from_wire(usage).cache_creation_input_tokens, 17);
}

#[test]
fn response_usage_accepts_a_top_level_cache_write_counter() {
    let usage: IncomingUsage = serde_json::from_value(serde_json::json!({
        "cache_creation_input_tokens": 23
    }))
    .expect("valid compatibility-layer usage");

    assert_eq!(turn_usage_from_wire(usage).cache_creation_input_tokens, 23);
}
