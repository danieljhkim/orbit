use super::super::artifact_ref::{is_task_ref, parse_artifact_ref};
use super::super::types::ArtifactRef;

#[test]
fn task_related_artifacts_accept_legacy_and_partitioned_ids() {
    assert_eq!(
        parse_artifact_ref("ORB-00042").expect("legacy task id"),
        ArtifactRef::Task("ORB-00042".to_string())
    );
    assert_eq!(
        parse_artifact_ref("DE-100000").expect("partitioned wide task id"),
        ArtifactRef::Task("DE-100000".to_string())
    );
    assert!(is_task_ref("ORB-7"));
    assert!(!is_task_ref("ADR-0001"));
}
