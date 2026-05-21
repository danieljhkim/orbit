use super::super::agent_detect::testing::MockAgentEnvProbe;
use super::super::agent_detect::*;

#[test]
fn detect_reflects_probe_results() {
    let probe = MockAgentEnvProbe::new()
        .with_binary("claude")
        .with_binary("grok")
        .with_binary("ollama")
        .with_env("ANTHROPIC_API_KEY", "sk-test");
    let detected = detect(&probe);
    assert_eq!(
        detected,
        DetectedAgents {
            claude_cli: true,
            grok_cli: true,
            ollama_cli: true,
            anthropic_api_key: true,
            ..DetectedAgents::default()
        }
    );
}

#[test]
fn empty_probe_detects_nothing() {
    let probe = MockAgentEnvProbe::new();
    assert_eq!(detect(&probe), DetectedAgents::default());
}

#[test]
fn default_provider_prefers_cli_in_documented_order() {
    // claude wins when present
    let detected = DetectedAgents {
        claude_cli: true,
        codex_cli: true,
        gemini_cli: true,
        grok_cli: true,
        ollama_cli: true,
        ..DetectedAgents::default()
    };
    assert_eq!(default_provider(&detected), "claude");

    // codex wins when claude absent
    let detected = DetectedAgents {
        codex_cli: true,
        gemini_cli: true,
        grok_cli: true,
        ollama_cli: true,
        ..DetectedAgents::default()
    };
    assert_eq!(default_provider(&detected), "codex");

    // gemini wins when claude/codex absent
    let detected = DetectedAgents {
        gemini_cli: true,
        grok_cli: true,
        ollama_cli: true,
        ..DetectedAgents::default()
    };
    assert_eq!(default_provider(&detected), "gemini");

    // grok wins when claude/codex/gemini absent
    let detected = DetectedAgents {
        grok_cli: true,
        ollama_cli: true,
        ..DetectedAgents::default()
    };
    assert_eq!(default_provider(&detected), "grok");

    // ollama wins when nothing else
    let detected = DetectedAgents {
        ollama_cli: true,
        ..DetectedAgents::default()
    };
    assert_eq!(default_provider(&detected), "ollama");
}

#[test]
fn default_provider_falls_back_to_api_keys() {
    // anthropic key → claude (http)
    let detected = DetectedAgents {
        anthropic_api_key: true,
        openai_api_key: true,
        ..DetectedAgents::default()
    };
    assert_eq!(default_provider(&detected), "claude");

    let detected = DetectedAgents {
        openai_api_key: true,
        gemini_api_key: true,
        ..DetectedAgents::default()
    };
    assert_eq!(default_provider(&detected), "codex");

    let detected = DetectedAgents {
        gemini_api_key: true,
        ..DetectedAgents::default()
    };
    assert_eq!(default_provider(&detected), "gemini");
}

#[test]
fn default_provider_last_resort_is_claude() {
    assert_eq!(default_provider(&DetectedAgents::default()), "claude");
}

#[test]
fn default_backend_picks_cli_when_matching_cli_present() {
    let detected = DetectedAgents {
        codex_cli: true,
        ..DetectedAgents::default()
    };
    assert_eq!(default_backend("codex", &detected), "cli");
    assert_eq!(default_backend("claude", &detected), "http");
}

#[test]
fn default_backend_unknown_provider_is_http() {
    assert_eq!(
        default_backend("openai_compat", &DetectedAgents::default()),
        "http"
    );
}

#[test]
fn model_registry_returns_expected_defaults() {
    assert_eq!(default_model_for("claude"), Some("claude-opus-4-7"));
    assert_eq!(default_model_for("codex"), Some("gpt-5.5"));
    assert_eq!(default_model_for("gemini"), Some("gemini-3-pro"));
    assert_eq!(default_model_for("grok"), Some("grok-build"));
    assert_eq!(default_model_for("ollama"), None);
    assert_eq!(default_model_for("unknown"), None);
}
