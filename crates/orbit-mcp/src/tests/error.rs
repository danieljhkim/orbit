use super::*;
use orbit_common::types::{ArtifactOrigin, ArtifactOriginMode};

fn federated_origin() -> ArtifactOrigin {
    ArtifactOrigin {
        mode: ArtifactOriginMode::Federated,
        worktree_root: "/safe/sibling".to_string(),
        branch: Some("orbit/ORB-10294".to_string()),
    }
}

#[test]
fn adr_artifact_errors_keep_stable_codes_and_safe_origin() {
    for (error, expected_code) in [
        (
            OrbitError::remote_artifact_unavailable(
                NotFoundKind::Adr,
                "ADR-0234",
                federated_origin(),
            ),
            "remote_artifact_unavailable",
        ),
        (
            OrbitError::artifact_not_local(NotFoundKind::Adr, "ADR-0234", federated_origin()),
            "artifact_not_local",
        ),
    ] {
        let payload = error_payload(&error);
        assert_eq!(payload["code"], expected_code);
        assert_eq!(payload["artifact_origin"]["mode"], "federated");
        assert_eq!(payload["artifact_origin"]["worktree_root"], "/safe/sibling");
        assert_eq!(payload["artifact_origin"]["branch"], "orbit/ORB-10294");
        assert!(payload["artifact_origin"].get("body_path").is_none());

        let result = tool_error_result(&error);
        assert_eq!(result.is_error, Some(true));
        assert_eq!(
            result.structured_content.expect("structured error payload")["code"],
            expected_code
        );
    }
}
