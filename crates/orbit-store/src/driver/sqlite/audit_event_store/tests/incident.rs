//! Failure-incident grouping contract [ORB-10871].
//!
//! Every fixture below is synthetic: tool names, roles, run ids, and messages
//! are generic placeholders, so the contract these tests pin is the grouping
//! *shape*, never a value observed in one workspace.

use std::collections::BTreeSet;

use chrono::{DateTime, Duration, TimeZone, Utc};
use orbit_types::telemetry::{AuditEvent, AuditEventStatus};

use crate::Store;
use crate::driver::sqlite::audit_event_store::incident::{
    CASCADE_WINDOW_SECS, FailureClass, FailureIncidentQuery, build_report, group_failure_incidents,
    normalize_message, signature_for,
};

use super::super::AuditEventInsertParams;

fn base_ts() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2031, 3, 4, 5, 0, 0)
        .single()
        .expect("fixed test timestamp")
}

struct FailureFixture {
    id: i64,
    offset_secs: i64,
    tool: &'static str,
    role: &'static str,
    status: AuditEventStatus,
    message: Option<String>,
    run_id: Option<&'static str>,
    activity_id: Option<&'static str>,
}

impl FailureFixture {
    fn new(id: i64, offset_secs: i64, tool: &'static str) -> Self {
        Self {
            id,
            offset_secs,
            tool,
            role: "actor-one",
            status: AuditEventStatus::Failure,
            message: Some("operation failed".to_string()),
            run_id: None,
            activity_id: None,
        }
    }

    fn role(mut self, role: &'static str) -> Self {
        self.role = role;
        self
    }

    fn status(mut self, status: AuditEventStatus) -> Self {
        self.status = status;
        self
    }

    fn message(mut self, message: impl Into<String>) -> Self {
        self.message = Some(message.into());
        self
    }

    fn run(mut self, run_id: &'static str, activity_id: &'static str) -> Self {
        self.run_id = Some(run_id);
        self.activity_id = Some(activity_id);
        self
    }

    fn build(self) -> AuditEvent {
        AuditEvent {
            id: self.id,
            execution_id: format!("exec-{}", self.id),
            timestamp: base_ts() + Duration::seconds(self.offset_secs),
            command: "tool".to_string(),
            subcommand: Some("run".to_string()),
            tool_name: Some(self.tool.to_string()),
            target_type: Some("tool".to_string()),
            target_id: Some(self.tool.to_string()),
            role: self.role.to_string(),
            status: self.status,
            exit_code: 1,
            duration_ms: 5,
            working_directory: "/workdir".to_string(),
            arguments_json: None,
            stdout_truncated: None,
            stderr_truncated: None,
            error_message: self.message,
            host: None,
            pid: 1,
            session_id: None,
            workspace_id: Some("ws-one".to_string()),
            caller_machine_id: None,
            caller_host_id: None,
            process_machine_id: None,
            process_host_id: None,
            transport: None,
            effective_capabilities: BTreeSet::new(),
            origin_session_id: None,
            mcp_call_id: None,
            trace_id: None,
            caller_ip: None,
            lease_id: None,
            task_id: Some("REC-1".to_string()),
            job_run_id: self.run_id.map(str::to_string),
            activity_id: self.activity_id.map(str::to_string),
            step_index: None,
        }
    }
}

#[test]
fn burst_of_identical_failures_collapses_into_one_incident_carrying_its_raw_count() {
    // 300 refusals of the same shape, differing only in the operand path — the
    // exact pattern that made a single burst read as hundreds of failures.
    let failures: Vec<AuditEvent> = (0..300)
        .map(|index| {
            FailureFixture::new(index, index, "surface.alpha")
                .message(format!("could not remove /work/dir-{index}/file.txt"))
                .build()
        })
        .collect();

    let report = build_report(&failures, false);

    assert_eq!(report.incident_count(), 1, "one burst is one incident");
    assert_eq!(report.raw_failed_events, 300, "no raw evidence is dropped");
    let incident = &report.incidents[0];
    assert_eq!(incident.event_count, 300);
    assert_eq!(incident.root_event_count, 300);
    assert_eq!(incident.first_ts, base_ts());
    assert_eq!(incident.last_ts, base_ts() + Duration::seconds(299));
    assert!(
        incident.signature.contains("<path>"),
        "the volatile operand must be normalized out of the signature: {}",
        incident.signature
    );
    assert!(
        incident.sample_events.len() <= 20 && !incident.sample_events.is_empty(),
        "a bounded sample of the underlying rows must be reachable"
    );
}

#[test]
fn unrelated_failures_remain_separate_incidents() {
    let failures = vec![
        FailureFixture::new(1, 0, "surface.alpha")
            .message("could not remove /work/a.txt")
            .build(),
        FailureFixture::new(2, 1, "surface.beta")
            .message("connection reset by peer")
            .build(),
        FailureFixture::new(3, 2, "surface.alpha")
            .role("actor-two")
            .message("could not remove /work/b.txt")
            .build(),
    ];

    let report = build_report(&failures, false);

    assert_eq!(
        report.incident_count(),
        3,
        "different surface, message, or actor must not be merged"
    );
    assert!(report.incidents.iter().all(|i| i.event_count == 1));
}

#[test]
fn cascading_run_failures_expose_one_root_with_the_chain_collapsed_beneath_it() {
    // One failed run whose failure propagates up three enclosing steps. Each
    // layer writes its own audit row; only the first is a root cause.
    let failures = vec![
        FailureFixture::new(1, 0, "step.inner")
            .message("child step returned a nonzero status")
            .run("run-one", "step-inner")
            .build(),
        FailureFixture::new(2, 1, "step.middle")
            .message("bundle aborted after a child failure")
            .run("run-one", "step-middle")
            .build(),
        FailureFixture::new(3, 2, "step.gate")
            .message("gate refused to advance")
            .run("run-one", "step-gate")
            .build(),
    ];

    let report = build_report(&failures, false);

    assert_eq!(report.incident_count(), 1, "one failed run is one incident");
    let incident = &report.incidents[0];
    assert_eq!(
        incident.surface, "step.inner",
        "the earliest failure is the root, not the outermost wrapper"
    );
    assert_eq!(incident.event_count, 3, "raw rows stay counted");
    assert_eq!(incident.root_event_count, 1);
    assert_eq!(incident.propagated_event_count(), 2);
    let chain: Vec<&str> = incident
        .propagation
        .iter()
        .map(|link| link.surface.as_str())
        .collect();
    assert_eq!(
        chain,
        vec!["step.middle", "step.gate"],
        "the propagation chain is ordered by occurrence"
    );
    assert_eq!(incident.run_ids, vec!["run-one".to_string()]);
}

#[test]
fn a_later_unrelated_failure_in_the_same_run_is_not_folded_into_the_cascade() {
    let failures = vec![
        FailureFixture::new(1, 0, "step.inner")
            .message("child step returned a nonzero status")
            .run("run-one", "step-inner")
            .build(),
        FailureFixture::new(2, 1, "step.middle")
            .message("bundle aborted after a child failure")
            .run("run-one", "step-middle")
            .build(),
        FailureFixture::new(3, CASCADE_WINDOW_SECS * 3, "step.later")
            .message("unrelated downstream problem")
            .run("run-one", "step-later")
            .build(),
    ];

    let report = build_report(&failures, false);

    assert_eq!(
        report.incident_count(),
        2,
        "a failure beyond the cascade window is its own root cause"
    );
    let cascade = report
        .incidents
        .iter()
        .find(|incident| incident.surface == "step.inner")
        .expect("cascade incident");
    assert_eq!(cascade.event_count, 2);
    let later = report
        .incidents
        .iter()
        .find(|incident| incident.surface == "step.later")
        .expect("independent incident");
    assert!(later.propagation.is_empty());
}

#[test]
fn denials_expected_negative_paths_and_unexpected_failures_never_merge() {
    let failures = vec![
        FailureFixture::new(1, 0, "surface.alpha")
            .status(AuditEventStatus::Denied)
            .message("policy denied: write outside the allowed scope")
            .run("run-one", "step-one")
            .build(),
        FailureFixture::new(2, 1, "surface.alpha")
            .message("invalid input: field must not be empty")
            .run("run-one", "step-one")
            .build(),
        FailureFixture::new(3, 2, "surface.alpha")
            .message("internal channel closed unexpectedly")
            .run("run-one", "step-one")
            .build(),
    ];

    let report = build_report(&failures, false);

    assert_eq!(report.incident_count(), 3);
    let classes: BTreeSet<FailureClass> = report
        .incidents
        .iter()
        .map(|incident| incident.class)
        .collect();
    assert_eq!(
        classes,
        BTreeSet::from([
            FailureClass::Denied,
            FailureClass::Expected,
            FailureClass::Unexpected
        ]),
        "same run and same surface must not blur the three failure classes"
    );
    assert_eq!(report.raw_events_by_class.get("denied"), Some(&1));
    assert_eq!(report.raw_events_by_class.get("expected"), Some(&1));
    assert_eq!(report.raw_events_by_class.get("unexpected"), Some(&1));
    assert_eq!(report.incidents_by_class.get("unexpected"), Some(&1));
}

#[test]
fn a_denial_recorded_with_failure_status_still_classifies_as_a_denial() {
    let failures = vec![
        FailureFixture::new(1, 0, "surface.alpha")
            .message("capability denied: caller lacks the required capability")
            .build(),
    ];

    let report = build_report(&failures, false);

    assert_eq!(report.incidents[0].class, FailureClass::Denied);
}

#[test]
fn grouping_is_order_independent_and_ids_are_stable() {
    let mut failures = vec![
        FailureFixture::new(1, 0, "surface.alpha")
            .message("could not remove /work/a.txt")
            .build(),
        FailureFixture::new(2, 5, "surface.beta")
            .message("connection reset by peer")
            .run("run-one", "step-one")
            .build(),
        FailureFixture::new(3, 6, "surface.gamma")
            .message("aborting after upstream failure")
            .run("run-one", "step-two")
            .build(),
    ];

    let first = group_failure_incidents(&failures);
    failures.reverse();
    let second = group_failure_incidents(&failures);

    assert_eq!(first, second, "input order must not change the grouping");
    assert!(
        first
            .iter()
            .all(|incident| incident.incident_id.starts_with("inc-")),
        "every incident carries a deterministic handle"
    );
}

#[test]
fn message_normalization_replaces_volatile_tokens_only() {
    assert_eq!(
        normalize_message("could not remove /work/dir-9/file.txt after 4 tries"),
        "could not remove <path> after <num> tries"
    );
    assert_eq!(
        normalize_message("run REC-42 at 2031-03-04T05:00:00Z failed"),
        "run <id> at <ts> failed"
    );
    assert_eq!(
        normalize_message("digest 0a1b2c3d4e5f6789 mismatch"),
        "digest <hex> mismatch"
    );
    assert_eq!(
        normalize_message("session 123e4567-e89b-12d3-a456-426614174000 expired"),
        "session <uuid> expired"
    );
}

#[test]
fn a_failure_without_a_message_groups_on_its_exit_code() {
    let event = FailureFixture::new(1, 0, "surface.alpha");
    let mut event = event.build();
    event.error_message = None;

    let signature = signature_for(&event);

    assert!(
        signature.contains("exit=1"),
        "an empty message must still yield a usable signature: {signature}"
    );
}

#[test]
fn the_store_query_groups_failures_without_hiding_any_audit_row() {
    let store = Store::open_in_memory().expect("open store");
    let insert = |execution_id: &str, status: AuditEventStatus, message: Option<&str>| {
        store
            .insert_audit_event_record(&AuditEventInsertParams {
                execution_id: execution_id.to_string(),
                command: "tool".to_string(),
                subcommand: Some("run".to_string()),
                tool_name: Some("surface.alpha".to_string()),
                target_type: Some("tool".to_string()),
                target_id: Some("surface.alpha".to_string()),
                role: "actor-one".to_string(),
                status,
                exit_code: if matches!(status, AuditEventStatus::Success) {
                    0
                } else {
                    1
                },
                duration_ms: 3,
                working_directory: "/workdir".to_string(),
                arguments_json: None,
                stdout_truncated: None,
                stderr_truncated: None,
                error_message: message.map(str::to_string),
                host: None,
                pid: 1,
                session_id: None,
                workspace_id: Some("ws-one".to_string()),
                caller_machine_id: None,
                caller_host_id: None,
                process_machine_id: None,
                process_host_id: None,
                transport: None,
                effective_capabilities: BTreeSet::new(),
                origin_session_id: None,
                mcp_call_id: None,
                lease_id: None,
                task_id: None,
                job_run_id: None,
                activity_id: None,
                step_index: None,
            })
            .expect("insert audit event");
    };

    insert("exec-ok", AuditEventStatus::Success, None);
    for index in 0..5 {
        insert(
            &format!("exec-fail-{index}"),
            AuditEventStatus::Failure,
            Some(&format!("could not remove /work/dir-{index}/file.txt")),
        );
    }
    insert(
        "exec-denied",
        AuditEventStatus::Denied,
        Some("policy denied: write outside the allowed scope"),
    );

    let report = store
        .get_failure_incidents(&FailureIncidentQuery::default())
        .expect("failure incidents");

    assert_eq!(report.raw_failed_events, 6, "success rows are not counted");
    assert_eq!(
        report.incident_count(),
        2,
        "the burst collapses; the denial stays separate"
    );
    assert!(!report.truncated);

    let all_rows = store
        .list_audit_events(&super::super::AuditEventFilter {
            limit: 100,
            ..Default::default()
        })
        .expect("raw audit rows");
    assert_eq!(
        all_rows.len(),
        7,
        "grouping must not remove anything from the raw audit view"
    );
}

#[test]
fn a_truncated_scan_is_reported_rather_than_silently_capped() {
    let store = Store::open_in_memory().expect("open store");
    for index in 0..3 {
        store
            .insert_audit_event_record(&AuditEventInsertParams {
                execution_id: format!("exec-{index}"),
                command: "tool".to_string(),
                subcommand: Some("run".to_string()),
                tool_name: Some("surface.alpha".to_string()),
                target_type: Some("tool".to_string()),
                target_id: Some("surface.alpha".to_string()),
                role: "actor-one".to_string(),
                status: AuditEventStatus::Failure,
                exit_code: 1,
                duration_ms: 3,
                working_directory: "/workdir".to_string(),
                arguments_json: None,
                stdout_truncated: None,
                stderr_truncated: None,
                error_message: Some("operation failed".to_string()),
                host: None,
                pid: 1,
                session_id: None,
                workspace_id: None,
                caller_machine_id: None,
                caller_host_id: None,
                process_machine_id: None,
                process_host_id: None,
                transport: None,
                effective_capabilities: BTreeSet::new(),
                origin_session_id: None,
                mcp_call_id: None,
                lease_id: None,
                task_id: None,
                job_run_id: None,
                activity_id: None,
                step_index: None,
            })
            .expect("insert audit event");
    }

    let report = store
        .get_failure_incidents(&FailureIncidentQuery {
            max_events: 2,
            ..Default::default()
        })
        .expect("failure incidents");

    assert!(report.truncated, "a capped scan must say so");
    assert_eq!(report.raw_failed_events, 2);
}
