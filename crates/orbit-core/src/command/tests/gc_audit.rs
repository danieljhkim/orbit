use std::fs;
use std::path::PathBuf;

use chrono::{DateTime, TimeZone, Utc};
use orbit_common::types::AuditEventStatus;
use orbit_store::{AuditEventInsertParams, Store, V2AuditEventInsertParams};

use crate::command::gc::{GcClock, GcCollector, GcRequest, GcScope, execute_gc};
use crate::command::gc_audit::AuditGcCollector;

const SHARED: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const ORPHAN: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
const HELD: &str = "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc";
const MISSING: &str = "dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd";

struct FixedClock(DateTime<Utc>);

impl GcClock for FixedClock {
    fn now(&self) -> DateTime<Utc> {
        self.0
    }
}

struct Fixture {
    _temp: tempfile::TempDir,
    orbit: PathBuf,
    global: PathBuf,
    store: Store,
    clock: FixedClock,
}

impl Fixture {
    fn new() -> Self {
        let temp = tempfile::tempdir().expect("temp");
        let orbit = temp.path().join("repo/.orbit");
        let global = temp.path().join("global/state");
        fs::create_dir_all(orbit.join("state/audit/blobs")).expect("audit root");
        fs::create_dir_all(&global).expect("global root");
        Self {
            _temp: temp,
            orbit,
            global,
            store: Store::open_in_memory().expect("store"),
            clock: FixedClock(Utc.with_ymd_and_hms(2026, 7, 12, 12, 0, 0).unwrap()),
        }
    }

    fn insert_v2(&self, event_id: &str, run_id: &str, ts: &str, payload: &str) {
        self.store
            .insert_v2_audit_event(&V2AuditEventInsertParams {
                workspace_id: "ws-a".to_string(),
                event_id: event_id.to_string(),
                source: "loop_event".to_string(),
                schema_version: 1,
                event_type: "test".to_string(),
                ts: DateTime::parse_from_rfc3339(ts)
                    .expect("timestamp")
                    .with_timezone(&Utc),
                run_id: run_id.to_string(),
                agent_identity: "codex".to_string(),
                parent_event_id: None,
                workspace_path: Some(self.orbit.display().to_string()),
                payload_json: payload.to_string(),
            })
            .expect("insert v2");
    }

    fn insert_legacy(&self, execution_id: &str, ts: &str, payload: &str) {
        self.store
            .insert_audit_event_record(&AuditEventInsertParams {
                execution_id: execution_id.to_string(),
                command: "test".to_string(),
                subcommand: None,
                tool_name: None,
                target_type: None,
                target_id: None,
                role: "codex".to_string(),
                status: AuditEventStatus::Success,
                exit_code: 0,
                duration_ms: 1,
                working_directory: self.orbit.parent().expect("repo").display().to_string(),
                arguments_json: Some(payload.to_string()),
                stdout_truncated: None,
                stderr_truncated: None,
                error_message: None,
                host: None,
                pid: 1,
                session_id: None,
                task_id: None,
                job_run_id: None,
                activity_id: None,
                step_index: None,
            })
            .expect("insert legacy");
        self.store
            .connection()
            .lock()
            .expect("connection")
            .execute(
                "UPDATE audit_events SET timestamp = ?1 WHERE execution_id = ?2",
                rusqlite::params![ts, execution_id],
            )
            .expect("set legacy timestamp");
    }

    fn blob(&self, hash: &str) -> PathBuf {
        let path = self
            .orbit
            .join("state/audit/blobs")
            .join(&hash[..2])
            .join(hash);
        fs::create_dir_all(path.parent().expect("parent")).expect("blob parent");
        fs::write(&path, hash).expect("blob");
        path
    }

    fn report(&self, apply: bool) -> crate::command::gc::GcReport {
        execute_gc(
            &AuditGcCollector::new(self.store.clone(), "ws-a", &self.orbit),
            GcRequest {
                apply,
                scope: GcScope::Workspace {
                    workspace_id: Some("ws-a".to_string()),
                    root: self.orbit.clone(),
                },
                retention_override: Some("30d"),
                global_state_dir: &self.global,
                clock: &self.clock,
            },
        )
        .expect("gc report")
    }
}

#[test]
fn mark_and_sweep_keeps_shared_blob_and_deletes_only_orphan_after_events() {
    let fixture = Fixture::new();
    let shared = fixture.blob(SHARED);
    let orphan = fixture.blob(ORPHAN);
    fixture.insert_v2(
        "old",
        "gone",
        "2026-01-01T00:00:00Z",
        &format!(r#"{{"blob":"{SHARED}"}}"#),
    );
    fixture.insert_v2(
        "new",
        "live",
        "2026-07-10T00:00:00Z",
        &format!(r#"{{"blob":"{SHARED}"}}"#),
    );
    fixture.insert_legacy(
        "legacy-old",
        "2026-01-01T00:00:00Z",
        &format!(r#"{{"blob":"{SHARED}"}}"#),
    );

    let plan = fixture.report(false);
    let ids: Vec<&str> = plan.targets[0]
        .items
        .iter()
        .map(|item| item.id.as_str())
        .collect();
    assert!(ids.iter().any(|id| id.starts_with("v2:")));
    assert!(ids.iter().any(|id| id.starts_with("legacy:")));
    assert!(ids.contains(&format!("blob:{ORPHAN}").as_str()));
    assert!(!ids.contains(&format!("blob:{SHARED}").as_str()));

    let apply = fixture.report(true);
    assert!(!orphan.exists());
    assert!(shared.exists());
    assert!(apply.manifest_path.is_some());
    assert_eq!(fixture.report(true).targets[0].counts.reclaimed, 0);
}

#[test]
fn holds_and_retained_runs_protect_blobs_and_envelopes() {
    let fixture = Fixture::new();
    let held = fixture.blob(HELD);
    fs::create_dir_all(fixture.orbit.join("state/audit/holds")).expect("holds");
    fs::write(
        fixture.orbit.join("state/audit/holds/export.json"),
        format!(r#"{{"blob":"{HELD}"}}"#),
    )
    .expect("hold");
    fs::create_dir_all(fixture.orbit.join("state/job-runs/job/jrun-kept")).expect("run");
    fs::write(
        fixture.orbit.join("state/job-runs/job/jrun-kept/run.json"),
        format!(r#"{{"run_id":"jrun-kept","blob":"{SHARED}"}}"#),
    )
    .expect("run evidence");
    let shared = fixture.blob(SHARED);
    fixture.insert_v2("old-kept", "jrun-kept", "2026-01-01T00:00:00Z", "{}");

    let apply = fixture.report(true);
    assert!(held.exists());
    assert!(shared.exists());
    assert!(
        apply.targets[0]
            .skipped
            .iter()
            .any(|skip| skip.code == "retained_run")
    );
}

#[test]
fn malformed_jsonl_is_retained_and_missing_references_are_reported() {
    let fixture = Fixture::new();
    let loop_root = fixture.orbit.join("state/audit/v2_loop");
    fs::create_dir_all(&loop_root).expect("loop root");
    let malformed = loop_root.join("run-bad.jsonl");
    fs::write(&malformed, format!("not-json {MISSING}\n")).expect("jsonl");

    let report = fixture.report(false);
    assert!(malformed.exists());
    assert!(
        report.targets[0]
            .skipped
            .iter()
            .any(|skip| skip.code == "malformed_jsonl")
    );
    assert!(
        report.targets[0]
            .errors
            .iter()
            .any(|error| error.code == "missing_referenced_blob")
    );
}

#[test]
fn workspace_filter_never_collects_another_workspaces_v2_rows() {
    let fixture = Fixture::new();
    fixture
        .store
        .insert_v2_audit_event(&V2AuditEventInsertParams {
            workspace_id: "ws-b".to_string(),
            event_id: "other".to_string(),
            source: "loop_event".to_string(),
            schema_version: 1,
            event_type: "test".to_string(),
            ts: Utc.with_ymd_and_hms(2020, 1, 1, 0, 0, 0).unwrap(),
            run_id: "gone".to_string(),
            agent_identity: "codex".to_string(),
            parent_event_id: None,
            workspace_path: None,
            payload_json: "{}".to_string(),
        })
        .expect("insert other workspace");
    fixture.report(true);
    assert_eq!(
        fixture
            .store
            .list_v2_audit_events_for_gc("ws-b")
            .expect("list")
            .len(),
        1
    );
}

#[test]
fn changed_file_is_skipped_like_an_interrupted_or_concurrent_writer() {
    let fixture = Fixture::new();
    let path = fixture.blob(ORPHAN);
    let collector = AuditGcCollector::new(fixture.store.clone(), "ws-a", &fixture.orbit);
    let scope = GcScope::Workspace {
        workspace_id: Some("ws-a".to_string()),
        root: fixture.orbit.clone(),
    };
    let context = crate::command::gc::GcContext {
        scope: &scope,
        retention_override: Some("30d"),
        clock: &fixture.clock,
    };
    let candidate = collector
        .plan(&context)
        .expect("plan")
        .candidates
        .into_iter()
        .find(|item| item.id == format!("blob:{ORPHAN}"))
        .expect("blob candidate");
    fs::write(&path, "concurrent append").expect("writer");
    assert!(matches!(
        collector
            .revalidate(&candidate, &context)
            .expect("revalidate"),
        crate::command::gc::GcRevalidation::Skip { .. }
    ));
    assert!(path.exists());
}
