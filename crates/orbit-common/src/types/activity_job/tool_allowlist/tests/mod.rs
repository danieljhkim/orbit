use super::super::*;

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
