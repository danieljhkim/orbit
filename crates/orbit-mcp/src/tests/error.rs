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

#[test]
fn task_bundle_corruption_has_a_stable_code_and_structured_context() {
    let payload = error_payload(&OrbitError::TaskBundleCorrupt {
        task_id: "ORB-00123".to_string(),
        path: "/safe/tasks/ORB-00123".to_string(),
        reason: "missing description.md".to_string(),
    });

    assert_eq!(payload["code"], "task_bundle_corrupt");
    assert_eq!(payload["task_id"], "ORB-00123");
    assert_eq!(payload["path"], "/safe/tasks/ORB-00123");
    assert_eq!(payload["reason"], "missing description.md");
}

/// ORB-10544: the shared ship submission path refuses a duplicate dispatch with
/// a typed conflict; the MCP projection of it must name the contended task and
/// the run holding it under the same `ship_run_in_flight` code the dashboard's
/// 409 body carries, so a tool caller can wait on or cancel that run without
/// parsing the message.
#[test]
fn ship_run_in_flight_has_a_stable_code_and_names_both_ids() {
    let error = OrbitError::ShipRunInFlight {
        task_id: "TST-00001".to_string(),
        run_id: "jrun-in-flight".to_string(),
    };
    let payload = error_payload(&error);

    assert_eq!(payload["code"], "ship_run_in_flight");
    assert_eq!(payload["task_id"], "TST-00001");
    assert_eq!(payload["run_id"], "jrun-in-flight");
    assert!(
        payload["message"].as_str().is_some_and(
            |message| message.contains("TST-00001") && message.contains("jrun-in-flight")
        )
    );

    let result = tool_error_result(&error);
    assert_eq!(result.is_error, Some(true));
    assert_eq!(
        result.structured_content.expect("structured error payload")["code"],
        "ship_run_in_flight"
    );
}
