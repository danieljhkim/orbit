use super::super::*;
use super::ts;

#[test]
fn adr_status_serde_yaml_round_trip_for_each_variant() {
    for status in [
        AdrStatus::Proposed,
        AdrStatus::Accepted,
        AdrStatus::Superseded,
        AdrStatus::Deleted,
    ] {
        let yaml = serde_yaml::to_string(&status).expect("serialize");
        let round: AdrStatus = serde_yaml::from_str(&yaml).expect("deserialize");
        assert_eq!(round, status);
    }
}

#[test]
fn legacy_validation_default_is_none() {
    assert_eq!(LegacyValidation::default(), LegacyValidation::None);
}

#[test]
fn adr_yaml_round_trip_full_struct() {
    let adr = Adr {
        id: "ADR-0042".to_string(),
        title: "Use BLAKE3 for dedup".to_string(),
        status: AdrStatus::Accepted,
        owner: "claude".to_string(),
        created_at: ts(2026, 5, 11),
        accepted_at: Some(ts(2026, 5, 12)),
        last_updated: ts(2026, 5, 12),
        related_features: vec!["knowledge-graph".to_string()],
        related_tasks: vec!["T20260511-1".to_string()],
        tags: vec!["schema".to_string(), "cross-cutting".to_string()],
        paths: vec!["crates/orbit-store/**".to_string()],
        supersedes: vec!["ADR-0001".to_string()],
        superseded_by: None,
        legacy_ids: vec![
            "activity-job/ADR-017".to_string(),
            "activity-job/ADR-018".to_string(),
        ],
        validation_warnings: vec!["missing owner in source".to_string()],
        legacy_validation: LegacyValidation::Warned,
    };

    let yaml = serde_yaml::to_string(&adr).expect("serialize");
    let round: Adr = serde_yaml::from_str(&yaml).expect("deserialize");
    assert_eq!(round, adr);
}

#[test]
fn adr_yaml_round_trip_with_missing_optional_fields() {
    let yaml = r#"id: ADR-0001
title: Initial decision
status: proposed
owner: claude
created_at: 2026-05-11T00:00:00Z
last_updated: 2026-05-11T00:00:00Z
"#;
    let adr: Adr = serde_yaml::from_str(yaml).expect("deserialize");
    assert_eq!(adr.id, "ADR-0001");
    assert_eq!(adr.status, AdrStatus::Proposed);
    assert!(adr.accepted_at.is_none());
    assert!(adr.superseded_by.is_none());
    assert!(adr.related_features.is_empty());
    assert!(adr.related_tasks.is_empty());
    assert!(adr.tags.is_empty());
    assert!(adr.paths.is_empty());
    assert!(adr.supersedes.is_empty());
    assert!(adr.legacy_ids.is_empty());
    assert!(adr.validation_warnings.is_empty());
    assert_eq!(adr.legacy_validation, LegacyValidation::None);
}
