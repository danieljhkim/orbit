//! [ORB-10588] Structural discovery of a job's step vs recovery activities.

use super::super::job_v2::JobV2;

fn job(yaml: &str) -> JobV2 {
    serde_yaml::from_str::<JobV2>(yaml).expect("job should parse")
}

#[test]
fn collects_job_level_recovery_and_flat_step_targets() {
    let roles = job(r#"
state: enabled
recovery_activity: patch_up
steps:
  - id: build
    target: activity:compile
  - id: verify
    target: activity:run_tests
"#)
    .activity_roles();

    // Both identifiers a step invocation can be recorded under: the executor
    // persists a dispatched step under its step id, the planning-duel runner
    // under the activity name.
    assert_eq!(
        roles.step.iter().cloned().collect::<Vec<_>>(),
        vec![
            "build".to_string(),
            "compile".to_string(),
            "run_tests".to_string(),
            "verify".to_string(),
        ],
        "the `activity:` namespace prefix must be stripped to match invocation rows"
    );
    assert_eq!(
        roles.recovery.iter().cloned().collect::<Vec<_>>(),
        vec!["patch_up".to_string()]
    );
}

#[test]
fn collects_step_level_recovery_at_every_nesting_depth() {
    let roles = job(r#"
state: enabled
steps:
  - id: outer
    parallel:
      join: { mode: all }
      branches:
        - id: branch_a
          recovery_activity: branch_rescue
          target: activity:branch_work
  - id: spread
    fan_out:
      items: "{{ input.items }}"
      worker:
        id: worker
        recovery_activity: worker_rescue
        target: activity:worker_work
    fan_in:
      join: { mode: all }
  - id: repeat
    loop:
      max_iterations: 3
      steps:
        - id: iteration
          recovery_activity: loop_rescue
          target: activity:loop_work
"#)
    .activity_roles();

    for expected in [
        // step ids, including the container steps
        "outer",
        "branch_a",
        "spread",
        "worker",
        "repeat",
        "iteration",
        // target activity names
        "branch_work",
        "worker_work",
        "loop_work",
    ] {
        assert!(
            roles.step.contains(expected),
            "nested parallel / fan_out / loop bodies must all be walked: missing {expected}"
        );
    }
    assert_eq!(
        roles.recovery.iter().cloned().collect::<Vec<_>>(),
        vec![
            "branch_rescue".to_string(),
            "loop_rescue".to_string(),
            "worker_rescue".to_string(),
        ]
    );
}

#[test]
fn an_activity_used_in_both_roles_is_reported_in_both_sets() {
    // The store records only the activity id, so an invocation of a dual-role
    // activity cannot be attributed structurally. Reporting it in both sets is
    // what lets callers exclude it rather than guess.
    let roles = job(r#"
state: enabled
steps:
  - id: work
    recovery_activity: shared
    target: activity:shared
"#)
    .activity_roles();

    assert!(roles.step.contains("shared"));
    assert!(roles.recovery.contains("shared"));
    assert!(
        !roles.recovery_only().contains("shared"),
        "a dual-role activity must not count as recovery-only"
    );
    assert!(
        !roles.step_only().contains("shared"),
        "a dual-role activity must not count as step-only"
    );
    // The step's own id is unambiguous and stays attributable.
    assert!(roles.step_only().contains("work"));
}

#[test]
fn merge_unions_roles_across_a_catalog() {
    let mut roles = job(r#"
state: enabled
recovery_activity: rescue
steps:
  - id: one
    target: activity:alpha
"#)
    .activity_roles();
    roles.merge(
        &job(r#"
state: enabled
steps:
  - id: two
    target: activity:beta
"#)
        .activity_roles(),
    );

    assert_eq!(
        roles.step_only().into_iter().collect::<Vec<_>>(),
        vec![
            "alpha".to_string(),
            "beta".to_string(),
            "one".to_string(),
            "two".to_string(),
        ]
    );
    assert_eq!(
        roles.recovery_only().into_iter().collect::<Vec<_>>(),
        vec!["rescue".to_string()]
    );
}

#[test]
fn a_job_with_no_recovery_hook_yields_an_empty_recovery_set() {
    // Distinguishes "recovery never fires" from "recovery is not configured";
    // the caller renders the latter as not-applicable rather than as 0%.
    let roles = job(r#"
state: enabled
steps:
  - id: only
    target: activity:alpha
"#)
    .activity_roles();

    assert!(roles.recovery.is_empty());
    assert_eq!(
        roles.step_only().into_iter().collect::<Vec<_>>(),
        vec!["alpha".to_string(), "only".to_string()]
    );
}
