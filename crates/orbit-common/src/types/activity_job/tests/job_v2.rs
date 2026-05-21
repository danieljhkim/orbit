use super::super::{AgentRole, job_v2::*};

fn assert_step_body_shape_error(yaml: &str) {
    let err = serde_yaml::from_str::<JobV2Step>(yaml).expect_err("step should fail to parse");
    assert!(
        err.to_string().contains("exactly one body shape"),
        "unexpected parse error: {err}",
    );
}

#[test]
fn rejects_step_with_parallel_and_target() {
    assert_step_body_shape_error(
        r#"
id: invalid
parallel:
  join: { mode: all }
  branches:
    - id: branch
      target: activity:something
target: activity:other
"#,
    );
}

#[test]
fn rejects_step_with_fan_out_and_loop() {
    assert_step_body_shape_error(
        r#"
id: invalid
fan_out:
  items: "{{ input.items }}"
  worker:
    id: worker
    target: activity:something
fan_in:
  join: { mode: all }
loop:
  max_iterations: 1
  steps:
    - id: loop_child
      target: activity:something
"#,
    );
}

#[test]
fn rejects_step_without_body_shape() {
    assert_step_body_shape_error(
        r#"
id: invalid
when: "{{ input.ready }}"
"#,
    );
}

#[test]
fn target_step_yaml_carries_step_level_role() {
    let yaml = r#"
id: my_step
role: implementer
spec:
  type: agent_loop
  instruction: hi
"#;
    let parsed: JobV2Step = serde_yaml::from_str(yaml).expect("parse step");
    let JobV2StepBody::Target(target) = parsed.body else {
        panic!("expected inline target body, got {:?}", parsed.body);
    };
    assert_eq!(target.role, Some(AgentRole::Implementer));
}

#[test]
fn target_ref_yaml_carries_step_level_role() {
    let yaml = r#"
id: my_step
role: planner
target: activity:something
"#;
    let parsed: JobV2Step = serde_yaml::from_str(yaml).expect("parse step");
    let JobV2StepBody::TargetRef(target_ref) = parsed.body else {
        panic!("expected target ref body, got {:?}", parsed.body);
    };
    assert_eq!(target_ref.role, Some(AgentRole::Planner));
}

#[test]
fn target_step_yaml_without_role_defaults_to_none() {
    let yaml = r#"
id: my_step
spec:
  type: agent_loop
  instruction: hi
"#;
    let parsed: JobV2Step = serde_yaml::from_str(yaml).expect("parse step");
    let JobV2StepBody::Target(target) = parsed.body else {
        panic!("expected inline target body, got {:?}", parsed.body);
    };
    assert_eq!(target.role, None);
}
