use super::super::*;

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
        validate_task_relations_for_source("ORB-00001", &self_cross_artifact_edge, &[]).is_err()
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
