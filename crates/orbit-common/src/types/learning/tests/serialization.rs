use chrono::{TimeZone, Utc};

use super::super::*;

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
        created_by: Some("claude-opus-4-7".to_string()),
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
