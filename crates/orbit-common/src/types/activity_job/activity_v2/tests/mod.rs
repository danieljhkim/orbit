use super::super::*;

#[test]
fn agent_role_serde_roundtrips_lowercase() {
    for (value, expected) in [
        (AgentRole::Reviewer, "\"reviewer\""),
        (AgentRole::Implementer, "\"implementer\""),
        (AgentRole::Planner, "\"planner\""),
    ] {
        let serialized = serde_json::to_string(&value).expect("serialize role");
        assert_eq!(serialized, expected);
        let parsed: AgentRole = serde_json::from_str(expected).expect("deserialize role");
        assert_eq!(parsed, value);
    }
}

#[test]
fn agent_loop_spec_yaml_includes_role_when_present() {
    let yaml = "instruction: hi\nrole: implementer\n";
    let parsed: AgentLoopSpec = serde_yaml::from_str(yaml).expect("parse spec");
    assert_eq!(parsed.role, Some(AgentRole::Implementer));
}

#[test]
fn agent_loop_spec_yaml_role_is_optional() {
    let yaml = "instruction: hi\n";
    let parsed: AgentLoopSpec = serde_yaml::from_str(yaml).expect("parse spec");
    assert_eq!(parsed.role, None);
}

#[test]
fn agent_loop_spec_defaults_to_cli_backend() {
    let yaml = "instruction: hi\n";
    let parsed: AgentLoopSpec = serde_yaml::from_str(yaml).expect("parse spec");
    assert_eq!(parsed.backend, Backend::Cli);
}
