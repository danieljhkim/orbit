mod resolution {
    use std::collections::BTreeMap;

    const TEST_CLAUDE_WEAK_MODEL: &str = "claude-sonnet-4-6";
    const TEST_CODEX_MODEL: &str = "gpt-5.5";

    use super::super::super::IdentityError;
    use super::super::super::agent_pair::*;

    fn assignment(model: &str, provider: &str) -> CrewAssignment {
        CrewAssignment {
            model: model.to_string(),
            provider: provider.to_string(),
        }
    }

    fn registry() -> BTreeMap<String, Crew> {
        let mut registry = BTreeMap::new();
        registry.insert(
            "codex".to_string(),
            Crew {
                name: "codex".to_string(),
                assignment: assignment(TEST_CODEX_MODEL, "codex"),
                description: None,
                tags: Vec::new(),
            },
        );
        registry.insert(
            "claude".to_string(),
            Crew {
                name: "claude".to_string(),
                assignment: assignment(TEST_CLAUDE_WEAK_MODEL, "claude"),
                description: None,
                tags: Vec::new(),
            },
        );
        registry
    }

    #[test]
    fn resolve_crew_returns_assignments_for_known_name() {
        let crew = resolve_crew("codex", &registry()).expect("crew resolves");

        assert_eq!(crew.name, "codex");
        assert_eq!(crew.assignment.model, TEST_CODEX_MODEL);
        assert_eq!(crew.assignment.provider, "codex");
    }

    #[test]
    fn resolve_crew_lists_defined_names_on_unknown() {
        let error = resolve_crew("missing", &registry()).expect_err("unknown crew fails");

        match error {
            IdentityError::InvalidWithSuggestions { did_you_mean, .. } => {
                assert_eq!(did_you_mean, vec!["claude", "codex"]);
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
}
