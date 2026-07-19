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

#[test]
fn hub_transport_errors_keep_stable_codes_and_call_identity() {
    let unavailable = error_payload(&OrbitError::HubUnavailable("offline".to_string()));
    assert_eq!(unavailable["code"], "hub_unavailable");

    let negotiation = error_payload(&OrbitError::HubNegotiation("digest drift".to_string()));
    assert_eq!(negotiation["code"], "hub_negotiation");

    let unknown = error_payload(&OrbitError::OutcomeUnknown {
        mcp_call_id: "mcall-exact".to_string(),
        message: "EOF after handoff".to_string(),
    });
    assert_eq!(unknown["code"], "outcome_unknown");
    assert!(
        unknown["message"]
            .as_str()
            .is_some_and(|message| message.contains("mcall-exact"))
    );

    let remote = error_payload(&OrbitError::RemoteTool {
        code: "invalid_input".to_string(),
        message: "definitive".to_string(),
        payload: json!({"code": "invalid_input", "message": "definitive", "detail": 7}),
    });
    assert_eq!(remote["code"], "invalid_input");
    assert_eq!(remote["detail"], 7);
}
