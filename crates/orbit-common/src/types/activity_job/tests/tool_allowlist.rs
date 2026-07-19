use super::super::tool_allowlist::*;

#[test]
fn registry_validation_accepts_documented_empty_audit_root() {
    validate_tool_allowlist_against_registered_tools(
        &["orbit.audit.*".to_string()],
        ["orbit.task.show"],
    )
    .expect("reserved audit root is intentionally empty");
}

#[test]
fn registry_validation_rejects_unmatched_non_empty_root() {
    let err = validate_tool_allowlist_against_registered_tools(
        &["fs.*".to_string()],
        ["orbit.task.show"],
    )
    .expect_err("fs wildcard must match registered tools");

    assert_eq!(
        err,
        ToolAllowlistError::WildcardRootMatchesNoTools {
            entry: "fs.*".to_string()
        }
    );
}

#[test]
fn registry_validation_rejects_removed_graph_mcp_names() {
    let wildcard = validate_tool_allowlist(&["orbit.graph.*".to_string()])
        .expect_err("removed graph wildcard must fail");
    assert_eq!(
        wildcard,
        ToolAllowlistError::WildcardRootNotPermitted {
            entry: "orbit.graph.*".to_string()
        }
    );

    let concrete = validate_tool_allowlist_against_registered_tools(
        &["orbit.graph.search".to_string()],
        ["orbit.search", "orbit.task.show"],
    )
    .expect_err("removed graph tool name must fail");
    assert_eq!(
        concrete,
        ToolAllowlistError::UnknownToolName {
            entry: "orbit.graph.search".to_string()
        }
    );
}
