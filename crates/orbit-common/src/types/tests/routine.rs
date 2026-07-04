use super::super::routine::{MissedRunPolicy, OverlapPolicy, RoutineTarget, parse_routine_yaml};

const VALID_ROUTINE: &str = r#"
schemaVersion: 1
name: almanac-auto-commit
description: Commit & push almanac changes nightly
enabled: true
hosts: [dk-mac]
trigger:
  cron: "0 22 * * *"
  missed_run: catch_up_once
target: job:almanac_commit_pipeline
policy:
  timeout_minutes: 10
  retries: { max: 2, backoff_minutes: 2 }
  overlap: forbid
"#;

#[test]
fn parses_the_design_doc_example() {
    let routine = parse_routine_yaml(VALID_ROUTINE).expect("valid routine");
    assert_eq!(routine.name, "almanac-auto-commit");
    assert!(routine.enabled);
    assert_eq!(routine.hosts, vec!["dk-mac".to_string()]);
    assert_eq!(routine.trigger.cron, "0 22 * * *");
    assert_eq!(routine.trigger.missed_run, MissedRunPolicy::CatchUpOnce);
    assert_eq!(
        routine.target,
        RoutineTarget::Job("almanac_commit_pipeline".to_string())
    );
    assert_eq!(routine.policy.timeout_minutes, 10);
    assert_eq!(routine.policy.retries.max, 2);
    assert_eq!(routine.policy.retries.backoff_minutes, 2);
    assert_eq!(routine.policy.overlap, OverlapPolicy::Forbid);
}

#[test]
fn defaults_apply_when_optional_fields_are_absent() {
    let routine = parse_routine_yaml(
        r#"
schemaVersion: 1
name: reindex
hosts: [dk-server-1]
trigger:
  cron: "*/30 * * * *"
target: job:docs_reindex
"#,
    )
    .expect("valid minimal routine");
    assert!(routine.enabled);
    assert_eq!(routine.trigger.missed_run, MissedRunPolicy::Skip);
    assert_eq!(routine.policy.timeout_minutes, 60);
    assert_eq!(routine.policy.retries.max, 0);
    assert_eq!(routine.policy.overlap, OverlapPolicy::Forbid);
}

#[test]
fn rejects_unknown_fields_fail_closed() {
    let error = parse_routine_yaml(
        r#"
schemaVersion: 1
name: reindex
hosts: [dk-server-1]
trigger:
  cron: "*/30 * * * *"
  jitter_seconds: 5
target: job:docs_reindex
"#,
    )
    .expect_err("unknown trigger field must fail");
    assert!(error.to_string().contains("jitter_seconds"), "{error}");
}

#[test]
fn rejects_unsupported_schema_version() {
    let error = parse_routine_yaml(
        r#"
schemaVersion: 2
name: reindex
hosts: [dk-server-1]
trigger:
  cron: "*/30 * * * *"
target: job:docs_reindex
"#,
    )
    .expect_err("schemaVersion 2 must fail");
    assert!(error.to_string().contains("schemaVersion 2"), "{error}");
}

#[test]
fn rejects_activity_targets_with_wrapping_guidance() {
    let error = parse_routine_yaml(
        r#"
schemaVersion: 1
name: reindex
hosts: [dk-server-1]
trigger:
  cron: "*/30 * * * *"
target: activity:semantic_reindex
"#,
    )
    .expect_err("activity target must fail in v1");
    let message = error.to_string();
    assert!(message.contains("wrap the"), "{message}");
    assert!(message.contains("job:<name>"), "{message}");
}

#[test]
fn rejects_inline_command_shaped_targets() {
    let error = parse_routine_yaml(
        r#"
schemaVersion: 1
name: reindex
hosts: [dk-server-1]
trigger:
  cron: "*/30 * * * *"
target: "sh -c 'rm -rf /'"
"#,
    )
    .expect_err("inline command target must fail");
    assert!(error.to_string().contains("ADR-0194"), "{error}");
}

#[test]
fn rejects_empty_hosts_and_bad_names() {
    let no_hosts = parse_routine_yaml(
        r#"
schemaVersion: 1
name: reindex
hosts: []
trigger:
  cron: "*/30 * * * *"
target: job:docs_reindex
"#,
    )
    .expect_err("empty hosts must fail");
    assert!(
        no_hosts.to_string().contains("at least one host"),
        "{no_hosts}"
    );

    let bad_name = parse_routine_yaml(
        r#"
schemaVersion: 1
name: "Almanac Commit"
hosts: [dk-mac]
trigger:
  cron: "0 22 * * *"
target: job:almanac_commit_pipeline
"#,
    )
    .expect_err("uppercase/space name must fail");
    assert!(bad_name.to_string().contains("routine name"), "{bad_name}");
}

#[test]
fn target_round_trips_through_serde() {
    let routine = parse_routine_yaml(VALID_ROUTINE).expect("valid routine");
    let serialized = serde_yaml::to_string(&routine).expect("serialize");
    assert!(serialized.contains("target: job:almanac_commit_pipeline"));
    let reparsed = parse_routine_yaml(&serialized).expect("reparse");
    assert_eq!(reparsed, routine);
}
