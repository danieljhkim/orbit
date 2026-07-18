mod serialization {
    use super::super::super::error::{
        ArtifactOrigin, ArtifactOriginMode, NotFoundKind, OrbitError,
    };

    #[test]
    fn orbit_not_found_error_serializes_with_typed_kind() {
        let error = OrbitError::NotFound {
            kind: NotFoundKind::Task,
            id: "ORB-00001".to_string(),
        };

        let value = serde_json::to_value(error).expect("serialize orbit error");

        assert_eq!(
            value,
            serde_json::json!({
                "NotFound": {
                    "kind": "task",
                    "id": "ORB-00001"
                }
            })
        );
    }

    #[test]
    fn remote_artifact_error_serializes_typed_origin() {
        let error = OrbitError::remote_artifact_unavailable(
            NotFoundKind::Adr,
            "ADR-0234",
            ArtifactOrigin {
                mode: ArtifactOriginMode::Federated,
                worktree_root: "/safe/worktree".to_string(),
                branch: Some("orbit/ORB-10294".to_string()),
            },
        );

        let value = serde_json::to_value(error).expect("serialize orbit error");

        assert_eq!(
            value["RemoteArtifactUnavailable"]["artifact_origin"],
            serde_json::json!({
                "mode": "federated",
                "worktree_root": "/safe/worktree",
                "branch": "orbit/ORB-10294",
            })
        );
    }
}
