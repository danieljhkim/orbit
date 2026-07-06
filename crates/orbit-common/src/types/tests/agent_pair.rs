mod resolution {
    use std::collections::BTreeMap;

    use crate::test_fixtures::{TEST_CLAUDE_MODEL, TEST_CLAUDE_WEAK_MODEL, TEST_CODEX_MODEL};

    use super::super::super::OrbitError;
    use super::super::super::agent_pair::*;

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
            "codex".to_string(),
            Crew {
                name: "codex".to_string(),
                planner: assignment(TEST_CODEX_MODEL, "codex"),
                implementer: assignment(TEST_CODEX_MODEL, "codex"),
                reviewer: assignment(TEST_CODEX_MODEL, "codex"),
            },
        );
        registry.insert(
            "claude".to_string(),
            Crew {
                name: "claude".to_string(),
                planner: assignment(TEST_CLAUDE_MODEL, "claude"),
                implementer: assignment(TEST_CLAUDE_WEAK_MODEL, "claude"),
                reviewer: assignment(TEST_CLAUDE_MODEL, "claude"),
            },
        );
        registry
    }

    #[test]
    fn resolve_crew_returns_assignments_for_known_name() {
        let crew = resolve_crew("codex", &registry()).expect("crew resolves");

        assert_eq!(crew.name, "codex");
        assert_eq!(crew.planner.model, TEST_CODEX_MODEL);
        assert_eq!(crew.implementer.provider, "codex");
        assert_eq!(crew.reviewer.backend, "cli");
    }

    #[test]
    fn resolve_crew_lists_defined_names_on_unknown() {
        let error = resolve_crew("missing", &registry()).expect_err("unknown crew fails");

        match error {
            OrbitError::InvalidInputDiagnostic { did_you_mean, .. } => {
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
