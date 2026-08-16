use crate::identity::{
    REGISTRY_IDENTIFIER_MAX_BYTES, validate_host_id, validate_machine_id,
    validate_registry_identifier,
};

#[test]
fn machine_id_validation_keeps_transport_targets_out_of_the_identity_namespace() {
    for accepted in ["hm_a", "hm_owner", "hm_9f2c81d4", "hm_0123456789abcdef"] {
        validate_machine_id(accepted).expect("compatible generated/test machine id");
    }
    for rejected in [
        "",
        "hm_",
        "dk1",
        "user@dk1",
        "ssh:dk1",
        "hm_ssh:dk1",
        "hm_path/name",
        " hm_owner",
    ] {
        let error = validate_machine_id(rejected)
            .expect_err("transport-shaped machine id must fail")
            .to_string();
        assert!(error.contains("machine_id"), "unexpected: {error}");
    }
}

#[test]
fn registry_identifiers_are_normalized_path_free_and_bounded() {
    validate_host_id("build-host").expect("normalized host id");
    validate_registry_identifier("workspace_id", "ws_alpha").expect("logical workspace id");

    for rejected in [
        " workspace",
        "workspace ",
        "workspace/path",
        "workspace\\path",
        "workspace\nname",
    ] {
        assert!(
            validate_registry_identifier("workspace_id", rejected).is_err(),
            "identifier should fail: {rejected:?}"
        );
    }
    assert!(
        validate_registry_identifier(
            "workspace_id",
            &"a".repeat(REGISTRY_IDENTIFIER_MAX_BYTES + 1)
        )
        .is_err()
    );
}
