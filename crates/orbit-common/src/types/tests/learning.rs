mod admission {
    use super::super::super::learning::*;

    #[test]
    fn learning_injection_state_dedupes_without_consuming_hard_cap() {
        let caps = LearningInjectionCaps {
            per_call: 5,
            per_session_hard: 2,
        };
        let mut state = LearningInjectionState::new();

        assert!(state.try_admit("L1", caps));
        assert!(!state.try_admit("L1", caps));
        assert!(state.try_admit("L2", caps));
        assert!(!state.try_admit("L3", caps));
        assert_eq!(state.count, 2);
        assert_eq!(state.emitted_ids.len(), 2);
    }

    #[test]
    fn admit_reminders_enforces_per_call_cap() {
        let caps = LearningInjectionCaps {
            per_call: 2,
            per_session_hard: 20,
        };
        let mut state = LearningInjectionState::new();
        let reminders: Vec<_> = (0..4)
            .map(|idx| LearningReminder {
                id: format!("L{idx}"),
                summary: format!("summary {idx}"),
                tags: Vec::new(),
            })
            .collect();

        let admitted = state.admit_reminders(&reminders, caps);

        assert_eq!(
            admitted.iter().map(|r| r.id.as_str()).collect::<Vec<_>>(),
            vec!["L0", "L1"]
        );
        assert_eq!(state.count, 2);
    }
}

mod normalize {
    use super::super::super::learning::*;

    #[test]
    fn normalize_learning_tags_trims_lowercases_and_dedupes() {
        let tags = normalize_learning_tags(vec![
            "  Perf ".to_string(),
            "BENCH".to_string(),
            "perf".to_string(),
            "   ".to_string(),
        ]);

        assert_eq!(tags, vec!["perf", "bench"]);
    }

    #[test]
    fn normalize_learning_paths_trims_and_dedupes_preserving_case() {
        let paths = normalize_learning_paths(vec![
            "  crates/Foo/**  ".to_string(),
            "crates/Foo/**".to_string(),
            "crates/Bar/*.rs".to_string(),
            "   ".to_string(),
        ]);

        assert_eq!(paths, vec!["crates/Foo/**", "crates/Bar/*.rs"]);
    }
}

mod parsing {
    use std::str::FromStr;

    use super::super::super::learning::*;

    #[test]
    fn learning_status_from_str_round_trips() {
        for status in [LearningStatus::Active, LearningStatus::Superseded] {
            let parsed: LearningStatus = status.as_str().parse().expect("parse");
            assert_eq!(parsed, status);
        }
        assert!(LearningStatus::from_str("nope").is_err());
    }

    #[test]
    fn evidence_kind_from_str_covers_all_variants() {
        assert_eq!(
            EvidenceKind::from_str("task").expect("task"),
            EvidenceKind::Task
        );
        assert_eq!(
            EvidenceKind::from_str("commit").expect("commit"),
            EvidenceKind::Commit
        );
        assert_eq!(
            EvidenceKind::from_str("external").expect("external"),
            EvidenceKind::External
        );
        assert!(EvidenceKind::from_str("other").is_err());
    }
}

mod reminders {
    use super::super::super::learning::*;

    #[test]
    fn render_reminder_block_returns_empty_for_no_reminders() {
        assert_eq!(render_reminder_block(&[]), "");
        assert_eq!(prepend_reminder_block("baseline", &[]), "baseline");
    }

    #[test]
    fn render_reminder_block_matches_design_shape() {
        let block = render_reminder_block(&[LearningReminder {
            id: "L-0001".to_string(),
            summary: "Verify output equivalence before freezing a result.".to_string(),
            tags: Vec::new(),
        }]);

        assert_eq!(
            block,
            "<system-reminder>\n\
Project learnings relevant to this task:\n\n\
- [L-0001] Verify output equivalence before freezing a result.\n\n\
Read full body via `orbit.learning.show <id>` if needed.\n\
</system-reminder>"
        );
    }

    #[test]
    fn render_reminder_block_includes_scope_tags_in_teaser() {
        let block = render_reminder_block(&[LearningReminder {
            id: "L-0002".to_string(),
            summary: "Scope tags ride in the teaser.".to_string(),
            tags: vec!["hooks".to_string(), "observability".to_string()],
        }]);

        assert_eq!(
            block,
            "<system-reminder>\n\
Project learnings relevant to this task:\n\n\
- [L-0002] Scope tags ride in the teaser. [tags: hooks, observability]\n\n\
Read full body via `orbit.learning.show <id>` if needed.\n\
</system-reminder>"
        );
    }
}

mod serialization {
    use chrono::{TimeZone, Utc};

    use crate::test_fixtures::TEST_CLAUDE_MODEL;

    use super::super::super::learning::*;

    fn sample_learning() -> Learning {
        let ts = Utc.with_ymd_and_hms(2026, 5, 11, 0, 0, 0).unwrap();
        Learning {
            id: "L-0001".to_string(),
            status: LearningStatus::Active,
            scope: LearningScope {
                paths: vec!["crates/orbit-engine/**/perf*.rs".to_string()],
                tags: vec!["performance".to_string()],
                symbols: vec!["orbit_engine::perf_runner::run".to_string()],
                semantic_seed: Some("benchmark equivalence check".to_string()),
            },
            summary: "Verify output equivalence on perf changes.".to_string(),
            body: "Full body here.".to_string(),
            evidence: vec![LearningEvidence {
                kind: EvidenceKind::Task,
                reference: "T20260510-1".to_string(),
            }],
            supersedes: None,
            superseded_by: None,
            legacy_ids: Vec::new(),
            created_at: ts,
            updated_at: ts,
            created_by: Some(TEST_CLAUDE_MODEL.to_string()),
            priority: None,
        }
    }

    #[test]
    fn learning_yaml_round_trips_reserved_phase_two_fields() {
        let learning = sample_learning();
        let yaml = serde_yaml::to_string(&learning).expect("serialize");
        assert!(yaml.contains("symbols:"));
        assert!(yaml.contains("semantic_seed: benchmark equivalence check"));

        let decoded: Learning = serde_yaml::from_str(&yaml).expect("deserialize");
        assert_eq!(decoded, learning);
    }

    #[test]
    fn learning_loads_minimal_yaml_with_phase_two_defaults() {
        let yaml = r#"id: L-0002
status: active
scope:
  paths: []
  tags: []
summary: Minimal record
body: ''
created_at: 2026-05-11T00:00:00Z
updated_at: 2026-05-11T00:00:00Z
"#;
        let learning: Learning = serde_yaml::from_str(yaml).expect("deserialize");
        assert!(learning.scope.symbols.is_empty());
        assert!(learning.scope.semantic_seed.is_none());
        assert!(learning.evidence.is_empty());
        assert_eq!(learning.id, "L-0002");
        assert_eq!(learning.status, LearningStatus::Active);
    }

    #[test]
    fn forward_compat_fixture_with_symbols_and_semantic_seed_round_trips() {
        let yaml = r#"id: L-0003
status: active
scope:
  paths:
    - "crates/orbit-engine/**"
  tags:
    - performance
  symbols:
    - "a::b"
  semantic_seed: "x"
summary: Fixture with phase-2 fields
body: ''
created_at: 2026-05-11T00:00:00Z
updated_at: 2026-05-11T00:00:00Z
"#;
        let learning: Learning = serde_yaml::from_str(yaml).expect("deserialize");
        assert_eq!(learning.scope.symbols, vec!["a::b"]);
        assert_eq!(learning.scope.semantic_seed.as_deref(), Some("x"));

        let yaml_out = serde_yaml::to_string(&learning).expect("serialize");
        let round_tripped: Learning = serde_yaml::from_str(&yaml_out).expect("deserialize 2");
        assert_eq!(round_tripped, learning);
    }
}
