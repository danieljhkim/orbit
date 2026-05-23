mod artifacts {
    use chrono::{TimeZone, Utc};

    use super::super::super::task_artifacts::*;

    #[test]
    fn artifact_path_validation_rejects_absolute_and_parent_paths() {
        assert!(validate_relative_artifact_path("files/result.txt").is_ok());
        assert!(validate_relative_artifact_path("/tmp/result.txt").is_err());
        assert!(validate_relative_artifact_path("../result.txt").is_err());
        assert!(validate_relative_artifact_path("files/../result.txt").is_err());
        assert!(validate_relative_artifact_path(r"files\result.txt").is_err());
        assert!(validate_relative_artifact_path("./result.txt").is_err());
        assert!(validate_relative_artifact_path("   ").is_err());
    }

    #[test]
    fn artifact_manifest_validates_file_metadata() {
        let manifest = ArtifactManifestV2 {
            schema_version: TASK_ARTIFACT_SCHEMA_VERSION,
            files: vec![ArtifactManifestFileV2 {
                path: "outputs/report.md".to_string(),
                blob: "files/report.md".to_string(),
                sha256: "a".repeat(64),
                media_type: "text/markdown".to_string(),
                size_bytes: 12,
                created_by: "codex:gpt-5.5".to_string(),
                created_at: Utc.with_ymd_and_hms(2026, 5, 10, 12, 0, 0).unwrap(),
            }],
        };
        assert!(manifest.validate().is_ok());

        let mut invalid = manifest;
        invalid.files[0].blob = "../blob".to_string();
        assert!(invalid.validate().is_err());
    }
}

mod envelope {
    use chrono::{TimeZone, Utc};

    use super::super::super::task_artifacts::*;
    use super::super::super::{TaskPriority, TaskStatus, TaskType};

    fn valid_envelope_yaml(id: &str) -> String {
        format!(
            r#"schema_version: 1
id: {id}
title: Build the thing
status: backlog
type: feature
priority: medium
created_at: 2026-05-10T12:00:00Z
updated_at: 2026-05-10T12:00:00Z
"#
        )
    }

    fn valid_envelope(id: &str) -> TaskEnvelopeV2 {
        TaskEnvelopeV2 {
            schema_version: TASK_ARTIFACT_SCHEMA_VERSION,
            id: id.to_string(),
            title: "Build the thing".to_string(),
            status: TaskStatus::Backlog,
            task_type: TaskType::Feature,
            priority: TaskPriority::Medium,
            complexity: None,
            job_run_id: None,
            crew: None,
            relations: Vec::new(),
            tags: Vec::new(),
            context_files: Vec::new(),
            external_refs: Vec::new(),
            created_by: None,
            planned_by: None,
            implemented_by: None,
            created_at: Utc.with_ymd_and_hms(2026, 5, 10, 12, 0, 0).unwrap(),
            updated_at: Utc.with_ymd_and_hms(2026, 5, 10, 12, 0, 0).unwrap(),
        }
    }

    #[test]
    fn envelope_rejects_old_inline_document_fields() {
        let yaml = format!(
            "{}\ndescription: old inline body\n",
            valid_envelope_yaml("ORB-00001")
        );
        let error = serde_yaml::from_str::<TaskEnvelopeV2>(&yaml).unwrap_err();
        assert!(error.to_string().contains("unknown field"));
    }

    #[test]
    fn envelope_requires_schema_version() {
        let yaml = r#"
id: ORB-00001
title: Build the thing
status: backlog
type: feature
priority: medium
created_at: 2026-05-10T12:00:00Z
updated_at: 2026-05-10T12:00:00Z
"#;
        let error = serde_yaml::from_str::<TaskEnvelopeV2>(yaml).unwrap_err();
        assert!(error.to_string().contains("schema_version"));
    }

    #[test]
    fn envelope_validate_rejects_wrong_schema_version() {
        let mut envelope = valid_envelope("ORB-00001");
        envelope.schema_version = 2;
        assert!(envelope.validate().is_err());
    }

    #[test]
    fn jsonl_rows_validate_schema_and_required_ids() {
        let event = TaskEventRowV2 {
            schema_version: TASK_ARTIFACT_SCHEMA_VERSION,
            event_id: "EV-0001".to_string(),
            at: Utc.with_ymd_and_hms(2026, 5, 10, 12, 0, 0).unwrap(),
            by: "codex:gpt-5.5".to_string(),
            event_type: "created".to_string(),
            note: None,
            from_status: None,
            to_status: Some(TaskStatus::Backlog),
        };
        assert!(event.validate().is_ok());

        let mut invalid_event = event;
        invalid_event.event_id = " ".to_string();
        assert!(invalid_event.validate().is_err());

        let comment = TaskCommentRowV2 {
            schema_version: TASK_ARTIFACT_SCHEMA_VERSION,
            comment_id: "C-0001".to_string(),
            at: Utc.with_ymd_and_hms(2026, 5, 10, 12, 0, 0).unwrap(),
            by: "daniel".to_string(),
            body: "Looks good.".to_string(),
        };
        assert!(comment.validate().is_ok());

        let mut invalid_comment = comment;
        invalid_comment.comment_id = String::new();
        assert!(invalid_comment.validate().is_err());
    }
}

mod ids {
    use super::super::super::task_artifacts::*;

    #[test]
    fn validates_and_formats_orb_task_ids() {
        assert!(is_valid_orb_task_id("ORB-00000"));
        assert!(is_valid_orb_task_id("ORB-99999"));
        assert!(!is_valid_orb_task_id("ORB-100000"));
        assert!(!is_valid_orb_task_id("orb-00001"));
        assert_eq!(format_orb_task_id(42).unwrap(), "ORB-00042");
        assert!(format_orb_task_id(100_000).is_err());
        assert!(validate_orb_task_id("ORB-12345").is_ok());
        assert!(validate_orb_task_id("ORB-1234").is_err());
    }
}

mod relations {
    use super::super::super::task_artifacts::*;

    #[test]
    fn relation_validation_rejects_duplicate_and_self_edges() {
        let duplicate = vec![
            TaskRelation {
                relation_type: TaskRelationType::BlockedBy,
                target: "ORB-00002".to_string(),
            },
            TaskRelation {
                relation_type: TaskRelationType::BlockedBy,
                target: "ORB-00002".to_string(),
            },
        ];
        assert!(validate_task_relations_for_source("ORB-00001", &duplicate, &[]).is_err());

        let self_edge = vec![TaskRelation {
            relation_type: TaskRelationType::BlockedBy,
            target: "ORB-00001".to_string(),
        }];
        assert!(validate_task_relations_for_source("ORB-00001", &self_edge, &[]).is_err());

        let self_cross_artifact_edge = vec![TaskRelation {
            relation_type: TaskRelationType::Produces,
            target: "ORB-00001".to_string(),
        }];
        assert!(
            validate_task_relations_for_source("ORB-00001", &self_cross_artifact_edge, &[])
                .is_err()
        );
    }

    #[test]
    fn task_relation_produces_and_resolves_yaml_round_trip() {
        let relations = vec![
            TaskRelation {
                relation_type: TaskRelationType::Produces,
                target: "F2026-05-007".to_string(),
            },
            TaskRelation {
                relation_type: TaskRelationType::Resolves,
                target: "L-0001".to_string(),
            },
        ];

        let yaml = serde_yaml::to_string(&relations).unwrap();
        assert!(yaml.contains("type: produces"));
        assert!(yaml.contains("type: resolves"));

        let decoded: Vec<TaskRelation> = serde_yaml::from_str(&yaml).unwrap();
        assert_eq!(decoded, relations);
    }

    #[test]
    fn produces_and_resolves_accept_cross_artifact_targets() {
        for relation_type in [TaskRelationType::Produces, TaskRelationType::Resolves] {
            for target in ["ORB-00002", "F2026-05-007", "L-0001", "ADR-0001"] {
                let relations = vec![TaskRelation {
                    relation_type,
                    target: target.to_string(),
                }];
                assert!(
                    validate_task_relations_for_source("ORB-00001", &relations, &[]).is_ok(),
                    "{relation_type:?} should accept {target}"
                );
            }
        }
    }

    #[test]
    fn legacy_relation_types_reject_non_task_targets() {
        for relation_type in [
            TaskRelationType::BlockedBy,
            TaskRelationType::ChildOf,
            TaskRelationType::SpawnedFrom,
            TaskRelationType::RegressionFrom,
            TaskRelationType::Supersedes,
            TaskRelationType::RelatedTo,
        ] {
            for target in ["F2026-05-007", "L-0001", "ADR-0001", "not-an-id"] {
                let relations = vec![TaskRelation {
                    relation_type,
                    target: target.to_string(),
                }];
                assert!(
                    validate_task_relations_for_source("ORB-00001", &relations, &[]).is_err(),
                    "{relation_type:?} should reject {target}"
                );
            }
        }
    }

    #[test]
    fn duplicate_cross_artifact_relations_are_rejected() {
        let relations = vec![
            TaskRelation {
                relation_type: TaskRelationType::Resolves,
                target: "F2026-05-007".to_string(),
            },
            TaskRelation {
                relation_type: TaskRelationType::Resolves,
                target: "F2026-05-007".to_string(),
            },
        ];
        assert!(validate_task_relations_for_source("ORB-00001", &relations, &[]).is_err());
    }

    #[test]
    fn relation_validation_rejects_blocking_and_hierarchy_cycles() {
        let existing = vec![TaskRelationEdge {
            source: "ORB-00002".to_string(),
            relation_type: TaskRelationType::BlockedBy,
            target: "ORB-00001".to_string(),
        }];
        let relations = vec![TaskRelation {
            relation_type: TaskRelationType::BlockedBy,
            target: "ORB-00002".to_string(),
        }];
        assert!(validate_task_relations_for_source("ORB-00001", &relations, &existing).is_err());

        let existing = vec![TaskRelationEdge {
            source: "ORB-00002".to_string(),
            relation_type: TaskRelationType::ChildOf,
            target: "ORB-00003".to_string(),
        }];
        let relations = vec![TaskRelation {
            relation_type: TaskRelationType::ChildOf,
            target: "ORB-00002".to_string(),
        }];
        assert!(validate_task_relations_for_source("ORB-00003", &relations, &existing).is_err());
    }

    #[test]
    fn relation_validation_allows_non_cyclic_related_edges() {
        let existing = vec![TaskRelationEdge {
            source: "ORB-00002".to_string(),
            relation_type: TaskRelationType::RelatedTo,
            target: "ORB-00001".to_string(),
        }];
        let relations = vec![TaskRelation {
            relation_type: TaskRelationType::RelatedTo,
            target: "ORB-00002".to_string(),
        }];
        assert!(validate_task_relations_for_source("ORB-00001", &relations, &existing).is_ok());
    }
}
