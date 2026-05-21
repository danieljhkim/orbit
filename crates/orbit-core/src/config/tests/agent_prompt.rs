use super::super::agent_prompt::testing::CannedPrompter;
use super::super::agent_prompt::*;
use super::super::agent_detect::DetectedAgents;

#[test]
fn empty_answer_accepts_role_aware_recommended_setup() {
    let detected = DetectedAgents {
        claude_cli: true,
        codex_cli: true,
        ..DetectedAgents::default()
    };
    let mut prompter = CannedPrompter::new([""]);
    let result = collect_role_settings(&detected, &mut prompter).unwrap();

    let reviewer = result.get("reviewer").expect("reviewer entry");
    assert_eq!(reviewer.provider.as_deref(), Some("codex"));
    assert_eq!(reviewer.backend.as_deref(), Some("cli"));
    assert_eq!(reviewer.model.as_deref(), Some("gpt-5.5"));

    let implementer = result.get("implementer").expect("implementer entry");
    assert_eq!(implementer.provider.as_deref(), Some("codex"));
    assert_eq!(implementer.backend.as_deref(), Some("cli"));
    assert_eq!(implementer.model.as_deref(), Some("gpt-5.5"));

    let planner = result.get("planner").expect("planner entry");
    assert_eq!(planner.provider.as_deref(), Some("claude"));
    assert_eq!(planner.backend.as_deref(), Some("cli"));
    assert_eq!(planner.model.as_deref(), Some("claude-opus-4-7"));

    let transcript = prompter.transcript();
    assert!(transcript.contains("Orbit uses agents for three workflow roles"));
    assert!(transcript.contains("Recommended setup:"));
    assert!(transcript.contains("Use this setup? [Y/n]: "));
}

#[test]
fn claude_only_detection_still_recommends_claude_for_all_roles() {
    let detected = DetectedAgents {
        claude_cli: true,
        ..DetectedAgents::default()
    };
    let mut prompter = CannedPrompter::new([""]);
    let result = collect_role_settings(&detected, &mut prompter).unwrap();

    let reviewer = result.get("reviewer").expect("reviewer entry");
    assert_eq!(reviewer.provider.as_deref(), Some("claude"));
    assert_eq!(reviewer.backend.as_deref(), Some("cli"));
    assert_eq!(reviewer.model.as_deref(), Some("claude-opus-4-7"));

    let implementer = result.get("implementer").expect("implementer entry");
    assert_eq!(implementer.provider.as_deref(), Some("claude"));
    assert_eq!(implementer.backend.as_deref(), Some("cli"));
    assert_eq!(implementer.model.as_deref(), Some("claude-opus-4-7"));

    let planner = result.get("planner").expect("planner entry");
    assert_eq!(planner.provider.as_deref(), Some("claude"));
    assert_eq!(planner.backend.as_deref(), Some("cli"));
    assert_eq!(planner.model.as_deref(), Some("claude-opus-4-7"));
}

#[test]
fn customization_enter_selects_role_recommendation() {
    let detected = DetectedAgents {
        claude_cli: true,
        codex_cli: true,
        ..DetectedAgents::default()
    };
    let mut prompter = CannedPrompter::new(["n", "reviewer", "", "", ""]);
    let result = collect_role_settings(&detected, &mut prompter).unwrap();

    let reviewer = result.get("reviewer").expect("reviewer entry");
    assert_eq!(reviewer.provider.as_deref(), Some("codex"));
    assert_eq!(reviewer.backend.as_deref(), Some("cli"));
    assert_eq!(reviewer.model.as_deref(), Some("gpt-5.5"));

    let implementer = result.get("implementer").expect("implementer entry");
    assert_eq!(implementer.provider.as_deref(), Some("codex"));
    assert_eq!(implementer.backend.as_deref(), Some("cli"));
    assert_eq!(implementer.model.as_deref(), Some("gpt-5.5"));

    let planner = result.get("planner").expect("planner entry");
    assert_eq!(planner.provider.as_deref(), Some("claude"));
    assert_eq!(planner.backend.as_deref(), Some("cli"));
    assert_eq!(planner.model.as_deref(), Some("claude-opus-4-7"));

    let transcript = prompter.transcript();
    assert!(transcript.contains("Choose an agent for Reviewer:"));
    assert!(transcript.contains("  1. Codex CLI"));
    assert!(transcript.contains("Updated setup:"));
}

#[test]
fn custom_provider_prompts_for_backend_and_model() {
    let detected = DetectedAgents::default();
    let mut prompter = CannedPrompter::new([
        "n",
        "reviewer",
        "custom",
        "openai_compat",
        "http",
        "my-model",
        "",
    ]);
    let result = collect_role_settings(&detected, &mut prompter).unwrap();
    let reviewer = result.get("reviewer").expect("reviewer entry");
    assert_eq!(reviewer.provider.as_deref(), Some("openai_compat"));
    assert_eq!(reviewer.backend.as_deref(), Some("http"));
    assert_eq!(reviewer.model.as_deref(), Some("my-model"));

    let implementer = result.get("implementer").expect("implementer entry");
    assert_eq!(implementer.provider.as_deref(), Some("claude"));
    assert_eq!(implementer.backend.as_deref(), Some("http"));
    assert_eq!(implementer.model.as_deref(), Some("claude-opus-4-7"));
}

#[test]
fn custom_provider_reprompts_for_blank_unknown_model() {
    let detected = DetectedAgents::default();
    let mut prompter = CannedPrompter::new([
        "n",
        "reviewer",
        "custom",
        "openai_compat",
        "http",
        "",
        "my-model",
        "",
    ]);
    let result = collect_role_settings(&detected, &mut prompter).unwrap();
    let reviewer = result.get("reviewer").expect("reviewer entry");
    assert_eq!(reviewer.provider.as_deref(), Some("openai_compat"));
    assert_eq!(reviewer.backend.as_deref(), Some("http"));
    assert_eq!(reviewer.model.as_deref(), Some("my-model"));
    assert!(
        prompter
            .transcript()
            .contains("Model is required for crew role assignments.")
    );
}
