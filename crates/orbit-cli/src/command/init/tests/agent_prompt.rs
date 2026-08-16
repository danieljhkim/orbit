use orbit_common::model_defaults::CODEX_DEFAULT_MODEL;

use crate::command::init::agent_detect::DetectedAgents;
use crate::command::init::agent_prompt::testing::CannedPrompter;
use crate::command::init::agent_prompt::*;

#[test]
fn empty_answer_accepts_one_recommended_default_crew() {
    let detected = DetectedAgents {
        claude_cli: true,
        codex_cli: true,
        ..DetectedAgents::default()
    };
    let mut prompter = CannedPrompter::new([""]);
    let result = collect_crew_setting(&detected, &mut prompter).unwrap();

    assert_eq!(result.provider.as_deref(), Some("claude"));
    let transcript = prompter.transcript();
    assert!(transcript.contains("one crew assignment"));
    assert!(transcript.contains("run's resolved crew"));
    assert!(transcript.contains("Use this default crew? [Y/n]: "));
    for retired in ["Reviewer", "Implementer", "Planner"] {
        assert!(!transcript.contains(retired));
    }
}

#[test]
fn customization_selects_one_detected_agent() {
    let detected = DetectedAgents {
        claude_cli: true,
        codex_cli: true,
        ..DetectedAgents::default()
    };
    let mut prompter = CannedPrompter::new(["n", "2", ""]);
    let result = collect_crew_setting(&detected, &mut prompter).unwrap();

    assert_eq!(result.provider.as_deref(), Some("codex"));
    assert_eq!(result.model.as_deref(), Some(CODEX_DEFAULT_MODEL));
    assert!(
        prompter
            .transcript()
            .contains("Choose an agent for the default crew:")
    );
}

#[test]
fn custom_provider_prompts_for_provider_and_model() {
    let detected = DetectedAgents::default();
    let mut prompter = CannedPrompter::new(["n", "custom", "gemini", "my-model"]);
    let result = collect_crew_setting(&detected, &mut prompter).unwrap();
    assert_eq!(result.provider.as_deref(), Some("gemini"));
    assert_eq!(result.model.as_deref(), Some("my-model"));
    // [ORB-10801] There is no backend left to choose, so init never asks.
    assert!(!prompter.transcript().contains("Backend"));
}

#[test]
fn custom_provider_reprompts_for_blank_unknown_model() {
    let detected = DetectedAgents::default();
    let mut prompter = CannedPrompter::new(["n", "custom", "openai_compat", "", "my-model"]);
    let result = collect_crew_setting(&detected, &mut prompter).unwrap();
    assert_eq!(result.model.as_deref(), Some("my-model"));
    assert!(
        prompter
            .transcript()
            .contains("Model is required for a crew assignment.")
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
    let qa = collect_qa_crew_setting(&detected, &mut prompter)
        .expect("qa choice")
        .expect("qa is available");

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
fn qa_crew_is_omitted_without_claude_or_codex_cli() {
    let detected = DetectedAgents {
        gemini_cli: true,
        ..DetectedAgents::default()
    };
    let mut prompter = CannedPrompter::new([] as [&str; 0]);
    let qa = collect_qa_crew_setting(&detected, &mut prompter).expect("qa selection");

    assert!(qa.is_none());
    assert!(prompter.transcript().is_empty());
}
