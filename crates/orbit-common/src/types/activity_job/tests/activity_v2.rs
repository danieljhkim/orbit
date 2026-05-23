use super::super::activity_v2::*;

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

#[test]
fn agent_loop_spec_proc_allowed_programs_defaults_to_none() {
    let yaml = "instruction: hi\n";
    let parsed: AgentLoopSpec = serde_yaml::from_str(yaml).expect("parse spec");
    assert_eq!(parsed.proc_allowed_programs, None);
}

#[test]
fn agent_loop_spec_proc_allowed_programs_round_trips() {
    let yaml = "instruction: hi\nproc_allowed_programs:\n  - git\n  - rg\n";
    let parsed: AgentLoopSpec = serde_yaml::from_str(yaml).expect("parse spec");
    assert_eq!(
        parsed.proc_allowed_programs,
        Some(vec!["git".to_string(), "rg".to_string()])
    );
    let reserialized = serde_yaml::to_string(&parsed).expect("serialize spec");
    let reparsed: AgentLoopSpec = serde_yaml::from_str(&reserialized).expect("re-parse spec");
    assert_eq!(reparsed.proc_allowed_programs, parsed.proc_allowed_programs);
}

#[test]
fn agent_loop_spec_proc_allowed_programs_accepts_empty_seq() {
    // Empty Some(vec![]) is meaningful: fail-closed when activity-scoped.
    let yaml = "instruction: hi\nproc_allowed_programs: []\n";
    let parsed: AgentLoopSpec = serde_yaml::from_str(yaml).expect("parse spec");
    assert_eq!(parsed.proc_allowed_programs, Some(Vec::<String>::new()));
}

#[test]
fn groundhog_spec_mirrors_proc_allowed_programs_into_agent_loop() {
    let yaml = "instruction: hi\nproc_allowed_programs:\n  - git\n";
    let parsed: GroundhogSpec = serde_yaml::from_str(yaml).expect("parse groundhog spec");
    assert_eq!(parsed.proc_allowed_programs, Some(vec!["git".to_string()]));
    let derived = parsed.as_agent_loop_spec();
    assert_eq!(derived.proc_allowed_programs, parsed.proc_allowed_programs);
}
