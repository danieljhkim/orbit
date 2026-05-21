use super::super::{agent_from_model, provider_from_model};

#[test]
fn agent_from_model_maps_known_prefixes() {
    assert_eq!(agent_from_model("claude-opus-4-7"), Some("claude"));
    assert_eq!(agent_from_model("gpt-5.5"), Some("codex"));
    assert_eq!(agent_from_model("gemini-3.1-pro-preview"), Some("gemini"));
    assert_eq!(agent_from_model("ollama:llama3.2"), Some("ollama"));
    assert_eq!(agent_from_model("grok-4"), Some("grok"));
    assert_eq!(agent_from_model("grok3-latest"), Some("grok"));
}

#[test]
fn agent_from_model_returns_none_for_unknown_prefix() {
    assert_eq!(agent_from_model("unknown-model"), None);
    assert_eq!(agent_from_model(""), None);
}

#[test]
fn provider_from_model_maps_known_prefixes() {
    assert_eq!(provider_from_model("claude-sonnet-4-7"), Some("anthropic"));
    assert_eq!(provider_from_model("gpt-5.5"), Some("openai"));
    assert_eq!(provider_from_model("gemini-3-pro"), Some("google"));
    assert_eq!(provider_from_model("ollama:mistral"), Some("ollama"));
    assert_eq!(provider_from_model("grok-4"), Some("xai"));
    assert_eq!(provider_from_model("grok3-mini"), Some("xai"));
    assert_eq!(provider_from_model("unknown-model"), None);
}
