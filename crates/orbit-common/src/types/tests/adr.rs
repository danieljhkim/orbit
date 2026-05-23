use chrono::{DateTime, TimeZone, Utc};

pub(super) fn ts(year: i32, month: u32, day: u32) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(year, month, day, 0, 0, 0).unwrap()
}

mod serde {
    use super::super::super::adr::*;
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
}

mod transitions {
    use super::super::super::OrbitError;
    use super::super::super::adr::*;

    #[test]
    fn transition_proposed_to_accepted_is_allowed() {
        AdrStatus::validate_transition(AdrStatus::Proposed, AdrStatus::Accepted)
            .expect("proposed -> accepted should be allowed");
    }

    #[test]
    fn transition_proposed_to_superseded_is_allowed() {
        AdrStatus::validate_transition(AdrStatus::Proposed, AdrStatus::Superseded)
            .expect("proposed -> superseded should be allowed");
    }

    #[test]
    fn transition_proposed_to_deleted_is_allowed() {
        AdrStatus::validate_transition(AdrStatus::Proposed, AdrStatus::Deleted)
            .expect("proposed -> deleted should be allowed");
    }

    #[test]
    fn transition_accepted_to_superseded_is_allowed() {
        AdrStatus::validate_transition(AdrStatus::Accepted, AdrStatus::Superseded)
            .expect("accepted -> superseded should be allowed");
    }

    #[test]
    fn transition_same_state_is_idempotent_for_all_variants() {
        for status in [
            AdrStatus::Proposed,
            AdrStatus::Accepted,
            AdrStatus::Superseded,
            AdrStatus::Deleted,
        ] {
            AdrStatus::validate_transition(status, status)
                .expect("same-state transition should be idempotent");
        }
    }

    #[test]
    fn transition_accepted_to_proposed_is_rejected() {
        let err = AdrStatus::validate_transition(AdrStatus::Accepted, AdrStatus::Proposed)
            .expect_err("accepted -> proposed should be rejected");
        assert!(matches!(err, OrbitError::AdrInvalidTransition(_)));
        assert!(err.to_string().contains("accepted"));
        assert!(err.to_string().contains("proposed"));
    }

    #[test]
    fn transition_accepted_to_deleted_is_rejected() {
        let err = AdrStatus::validate_transition(AdrStatus::Accepted, AdrStatus::Deleted)
            .expect_err("accepted -> deleted should be rejected");
        assert!(matches!(err, OrbitError::AdrInvalidTransition(_)));
        assert!(err.to_string().contains("superseded"));
    }

    #[test]
    fn transition_superseded_to_anything_is_rejected() {
        for target in [AdrStatus::Proposed, AdrStatus::Accepted, AdrStatus::Deleted] {
            let err = AdrStatus::validate_transition(AdrStatus::Superseded, target)
                .expect_err("superseded is terminal");
            assert!(matches!(err, OrbitError::AdrInvalidTransition(_)));
            assert!(err.to_string().contains("terminal"));
        }
    }

    #[test]
    fn transition_deleted_to_anything_is_rejected() {
        for target in [
            AdrStatus::Proposed,
            AdrStatus::Accepted,
            AdrStatus::Superseded,
        ] {
            let err = AdrStatus::validate_transition(AdrStatus::Deleted, target)
                .expect_err("deleted is terminal");
            assert!(matches!(err, OrbitError::AdrInvalidTransition(_)));
            assert!(err.to_string().contains("terminal"));
        }
    }
}

mod validation {
    use super::super::super::adr::*;

    #[test]
    fn validate_adr_id_accepts_canonical_ids() {
        validate_adr_id("ADR-0001").expect("ADR-0001 should be valid");
        validate_adr_id("ADR-9999").expect("ADR-9999 should be valid");
        validate_adr_id("ADR-12345").expect("ADR-12345 (5 digits) should be valid");
    }

    #[test]
    fn validate_adr_id_rejects_invalid_ids() {
        assert!(validate_adr_id("").is_err(), "empty should be rejected");
        assert!(
            validate_adr_id("ADR-1").is_err(),
            "1 digit should be rejected"
        );
        assert!(
            validate_adr_id("ADR-001").is_err(),
            "3 digits should be rejected"
        );
        assert!(
            validate_adr_id("adr-0001").is_err(),
            "lowercase prefix should be rejected"
        );
        assert!(
            validate_adr_id("ADR-XXXX").is_err(),
            "non-digit suffix should be rejected"
        );
    }

    #[test]
    fn legacy_id_for_pads_local_number_to_three_digits() {
        assert_eq!(legacy_id_for("activity-job", 17), "activity-job/ADR-017");
    }
}
