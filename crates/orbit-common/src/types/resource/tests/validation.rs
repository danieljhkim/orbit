use super::super::validate_resource_name;

#[test]
fn resource_name_accepts_existing_seeded_name_shapes() {
    for name in [
        "default",
        "local-shell",
        "task_auto_pipeline",
        "agent_loop_cli_reference",
    ] {
        validate_resource_name(name).expect(name);
    }
}

#[test]
fn resource_name_rejects_path_like_names() {
    for name in [
        "", " ", ".hidden", ".", "..", "../x", "x/../y", "x/y", "x\\y", "C:foo", "foo:bar",
        "foo.yaml", "foo\nbar",
    ] {
        assert!(
            validate_resource_name(name).is_err(),
            "expected invalid resource name: {name:?}"
        );
    }
}
