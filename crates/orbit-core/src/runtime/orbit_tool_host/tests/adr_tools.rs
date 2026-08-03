use orbit_common::types::OrbitError;
use serde_json::json;

use super::super::test_support::{managed_tool_env_guard, test_runtime, unmanaged_tool_env_guard};

const BODY: &str = "## Context\nA real choice existed.\n\n## Decision\nKeep approval separate.\n\n## Consequences\n- Creation stays Proposed.\n- Cost: approval requires another actor.\n";

#[test]
fn managed_run_can_refine_proposed_adr_but_cannot_accept_it() {
    let (_temp, runtime, _repo_root) = test_runtime();
    let created = {
        let _env = unmanaged_tool_env_guard();
        super::super::adr_tools::add(
            &runtime,
            json!({
                "title": "Executor proposal",
                "body": BODY,
                "owner": "codex",
                "related_tasks": ["ORB-10596"]
            }),
            None,
            Some("codex".to_string()),
        )
        .expect("create Proposed ADR")
    };
    let id = created["id"].as_str().expect("ADR id");

    {
        let _env = managed_tool_env_guard("jrun-adr-authoring-test");
        let updated = super::super::adr_tools::update(
            &runtime,
            json!({"id": id, "title": "Refined executor proposal"}),
            None,
            Some("codex".to_string()),
        )
        .expect("executor may refine Proposed ADR");
        assert_eq!(updated["title"], "Refined executor proposal");

        let denied = super::super::adr_tools::update(
            &runtime,
            json!({
                "id": id,
                "body": "must not partially land",
                "status": "accepted"
            }),
            None,
            Some("codex".to_string()),
        )
        .expect_err("executor acceptance must be denied");
        assert!(
            matches!(denied, OrbitError::PolicyDenied(_)),
            "unexpected error: {denied:?}"
        );
    }

    let unchanged =
        super::super::adr_tools::show(&runtime, json!({"id": id})).expect("read after denial");
    assert_eq!(unchanged["status"], "proposed");
    assert_eq!(unchanged["body"].as_str().expect("body"), BODY.trim_end());

    let accepted = {
        let _env = unmanaged_tool_env_guard();
        super::super::adr_tools::update(
            &runtime,
            json!({"id": id, "status": "accepted"}),
            None,
            Some("codex".to_string()),
        )
        .expect("separate unmanaged approval remains available")
    };
    assert_eq!(accepted["status"], "accepted");
}

#[test]
fn managed_run_cannot_rewrite_an_accepted_adr() {
    let (_temp, runtime, _repo_root) = test_runtime();
    let accepted_id = {
        let _env = unmanaged_tool_env_guard();
        let created = super::super::adr_tools::add(
            &runtime,
            json!({
                "title": "Accepted history",
                "body": BODY,
                "owner": "human",
                "related_tasks": ["ORB-10596"]
            }),
            None,
            None,
        )
        .expect("create ADR");
        let id = created["id"].as_str().expect("ADR id").to_string();
        super::super::adr_tools::update(
            &runtime,
            json!({"id": id, "status": "accepted"}),
            None,
            None,
        )
        .expect("accept ADR");
        id
    };

    let denied = {
        let _env = managed_tool_env_guard("jrun-adr-history-test");
        super::super::adr_tools::update(
            &runtime,
            json!({"id": accepted_id, "title": "Executor rewrite"}),
            None,
            Some("codex".to_string()),
        )
        .expect_err("accepted ADR rewrite must be denied")
    };
    assert!(matches!(denied, OrbitError::PolicyDenied(_)));
}
