use std::collections::BTreeMap;

use super::super::*;

fn assignment(model: &str, provider: &str) -> CrewRoleAssignment {
    CrewRoleAssignment {
        model: model.to_string(),
        provider: provider.to_string(),
        backend: "cli".to_string(),
    }
}

fn registry() -> BTreeMap<String, Crew> {
    let mut registry = BTreeMap::new();
    registry.insert(
        "opus-codex".to_string(),
        Crew {
            name: "opus-codex".to_string(),
            planner: assignment("claude-opus-4-7", "claude"),
            implementer: assignment("gpt-5.5", "codex"),
            reviewer: assignment("claude-opus-4-7", "claude"),
        },
    );
    registry.insert(
        "all-claude".to_string(),
        Crew {
            name: "all-claude".to_string(),
            planner: assignment("claude-opus-4-7", "claude"),
            implementer: assignment("claude-sonnet-4-6", "claude"),
            reviewer: assignment("claude-opus-4-7", "claude"),
        },
    );
    registry
}

#[test]
fn resolve_crew_returns_assignments_for_known_name() {
    let crew = resolve_crew("opus-codex", &registry()).expect("crew resolves");

    assert_eq!(crew.name, "opus-codex");
    assert_eq!(crew.planner.model, "claude-opus-4-7");
    assert_eq!(crew.implementer.provider, "codex");
    assert_eq!(crew.reviewer.backend, "cli");
}

#[test]
fn resolve_crew_lists_defined_names_on_unknown() {
    let error = resolve_crew("missing", &registry()).expect_err("unknown crew fails");

    match error {
        OrbitError::InvalidInputDiagnostic { did_you_mean, .. } => {
            assert_eq!(did_you_mean, vec!["all-claude", "opus-codex"]);
        }
        other => panic!("expected InvalidInputDiagnostic, got {other:?}"),
    }
}

#[test]
fn infer_agent_family_from_model_handles_claude_gpt_gemini_grok_prefixes() {
    assert_eq!(
        infer_agent_family_from_model("claude-opus-4-7").as_deref(),
        Some("claude")
    );
    assert_eq!(
        infer_agent_family_from_model("gpt-5.5").as_deref(),
        Some("codex")
    );
    assert_eq!(
        infer_agent_family_from_model("o3-mini").as_deref(),
        Some("codex")
    );
    assert_eq!(
        infer_agent_family_from_model("gemini-3.1-pro").as_deref(),
        Some("gemini")
    );
    assert_eq!(
        infer_agent_family_from_model("grok-4").as_deref(),
        Some("grok")
    );
    assert_eq!(
        infer_agent_family_from_model("grok3").as_deref(),
        Some("grok")
    );
}
