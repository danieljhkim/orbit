use orbit_common::model_defaults::{CLAUDE_DEFAULT_STRONG, CODEX_DEFAULT_MODEL};

use super::super::agent_detect::DetectedAgents;
use super::super::agent_prompt::testing::CannedPrompter;
use super::super::agent_prompt::*;

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
    assert_eq!(reviewer.model.as_deref(), Some(CODEX_DEFAULT_MODEL));

    let implementer = result.get("implementer").expect("implementer entry");
    assert_eq!(implementer.provider.as_deref(), Some("codex"));
    assert_eq!(implementer.backend.as_deref(), Some("cli"));
    assert_eq!(implementer.model.as_deref(), Some(CODEX_DEFAULT_MODEL));

    let planner = result.get("planner").expect("planner entry");
    assert_eq!(planner.provider.as_deref(), Some("claude"));
    assert_eq!(planner.backend.as_deref(), Some("cli"));
    assert_eq!(planner.model.as_deref(), Some(CLAUDE_DEFAULT_STRONG));

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
    assert_eq!(reviewer.model.as_deref(), Some(CLAUDE_DEFAULT_STRONG));

    let implementer = result.get("implementer").expect("implementer entry");
    assert_eq!(implementer.provider.as_deref(), Some("claude"));
    assert_eq!(implementer.backend.as_deref(), Some("cli"));
    assert_eq!(implementer.model.as_deref(), Some(CLAUDE_DEFAULT_STRONG));

    let planner = result.get("planner").expect("planner entry");
    assert_eq!(planner.provider.as_deref(), Some("claude"));
    assert_eq!(planner.backend.as_deref(), Some("cli"));
    assert_eq!(planner.model.as_deref(), Some(CLAUDE_DEFAULT_STRONG));
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
    assert_eq!(reviewer.model.as_deref(), Some(CODEX_DEFAULT_MODEL));

    let implementer = result.get("implementer").expect("implementer entry");
    assert_eq!(implementer.provider.as_deref(), Some("codex"));
    assert_eq!(implementer.backend.as_deref(), Some("cli"));
    assert_eq!(implementer.model.as_deref(), Some(CODEX_DEFAULT_MODEL));

    let planner = result.get("planner").expect("planner entry");
    assert_eq!(planner.provider.as_deref(), Some("claude"));
    assert_eq!(planner.backend.as_deref(), Some("cli"));
    assert_eq!(planner.model.as_deref(), Some(CLAUDE_DEFAULT_STRONG));

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
    assert_eq!(implementer.model.as_deref(), Some(CLAUDE_DEFAULT_STRONG));
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

#[test]
fn qa_crew_prompt_offers_only_detected_claude_and_codex_defaults() {
    let detected = DetectedAgents {
        claude_cli: true,
        codex_cli: true,
        gemini_cli: true,
        ..DetectedAgents::default()
    };
    let mut prompter = CannedPrompter::new(["2"]);
    let qa = collect_qa_crew_setting(&detected, &mut prompter).expect("qa choice");

    assert_eq!(qa.provider.as_deref(), Some("claude"));
    assert_eq!(
        qa.model.as_deref(),
        Some(orbit_common::model_defaults::CLAUDE_DEFAULT_WEAK)
    );
    let transcript = prompter.transcript();
    assert!(transcript.contains("Codex  terra"));
    assert!(transcript.contains("Claude sonnet"));
    assert!(!transcript.contains("Gemini"));
}

#[test]
fn qa_crew_noninteractive_default_prefers_detected_codex() {
    let detected = DetectedAgents {
        claude_cli: true,
        codex_cli: true,
        ..DetectedAgents::default()
    };
    let mut prompter = CannedPrompter::new([""]);
    let qa = collect_qa_crew_setting(&detected, &mut prompter).expect("qa default");

    assert_eq!(qa.provider.as_deref(), Some("codex"));
    assert_eq!(qa.model.as_deref(), Some(CODEX_DEFAULT_MODEL));
}
