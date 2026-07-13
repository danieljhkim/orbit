use std::fs;
use std::io::Write;
use std::path::PathBuf;

use chrono::{DateTime, TimeZone, Utc};
use orbit_common::types::AuditEventStatus;
use orbit_common::utility::audit_pending;
use orbit_common::utility::audit_writer_guard;
use orbit_store::{AuditEventInsertParams, Store, V2AuditEventInsertParams};

use crate::command::gc::{
    GcCandidate, GcClock, GcCollector, GcContext, GcRequest, GcRevalidation, GcScope, execute_gc,
};
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

    fn audit_root(&self) -> PathBuf {
        self.orbit.join("state/audit")
    }

    fn collector(&self) -> AuditGcCollector {
        AuditGcCollector::new(self.store.clone(), "ws-a", &self.orbit)
    }

    fn scope(&self) -> GcScope {
        GcScope::Workspace {
            workspace_id: Some("ws-a".to_string()),
            root: self.orbit.clone(),
        }
    }

    fn frozen_candidate(
        &self,
        collector: &AuditGcCollector,
        scope: &GcScope,
        id: &str,
    ) -> GcCandidate {
        let context = GcContext {
            scope,
            retention_override: Some("30d"),
            clock: &self.clock,
        };
        collector
            .plan(&context)
            .expect("plan")
            .candidates
            .into_iter()
            .find(|item| item.id == id)
            .unwrap_or_else(|| panic!("expected candidate `{id}`"))
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

// The next three tests exercise the ORB-10186 audit writer/GC guard by
// contending on the *same* `audit_writer_guard` advisory lock that every
// V2SqliteSink writer path holds across its publication. `collector.apply`
// acquires that guard, re-marks/re-fingerprints under it, and only then
// deletes — so a writer that publishes a retained reference (or appends a
// live JSONL envelope) while holding the guard is never lost, and a blob GC
// sweeps is never one a retained envelope points at. Lock ordering mirrors the
// worktree collector: host GC lock → audit writer guard → filesystem mutation.

#[test]
fn writer_wins_published_reference_under_guard_survives_blob_sweep() {
    let fixture = Fixture::new();
    let orphan = fixture.blob(ORPHAN);
    let collector = fixture.collector();
    let scope = fixture.scope();
    // Freeze ORPHAN as an unreferenced sweep candidate.
    let frozen = fixture.frozen_candidate(&collector, &scope, &format!("blob:{ORPHAN}"));

    let audit_root = fixture.audit_root();
    let context = GcContext {
        scope: &scope,
        retention_override: Some("30d"),
        clock: &fixture.clock,
    };
    let fixture_ref = &fixture;
    let audit_root_ref = &audit_root;
    let collector_ref = &collector;
    let context_ref = &context;
    let frozen_ref = &frozen;

    std::thread::scope(|threads| {
        // The writer holds the guard first, exactly as V2SqliteSink::write_envelope does.
        let guard = audit_writer_guard::acquire(audit_root_ref).expect("writer acquires guard");

        let (started_tx, started_rx) = std::sync::mpsc::channel::<()>();
        let apply_handle = threads.spawn(move || {
            // Only start the sweep once the writer holds the guard, so `apply`
            // must block on it rather than racing ahead.
            started_rx.recv().expect("gc start signal");
            collector_ref.apply(frozen_ref, context_ref)
        });
        started_tx.send(()).expect("signal gc");

        // Publish a retained v2 envelope referencing ORPHAN while holding the guard.
        fixture_ref.insert_v2(
            "published-under-guard",
            "live",
            "2026-07-10T00:00:00Z",
            &format!(r#"{{"blob":"{ORPHAN}"}}"#),
        );

        drop(guard); // release; the blocked apply now re-marks under the guard

        let applied = apply_handle.join().expect("gc apply thread");
        assert!(
            applied.is_err(),
            "apply must fail closed once ORPHAN became referenced, got {applied:?}"
        );
    });

    assert!(
        orphan.exists(),
        "a blob referenced by an envelope published under the guard must survive the sweep"
    );
}

#[test]
fn gc_wins_holds_guard_through_deletion_and_blocked_writer_is_not_lost() {
    let fixture = Fixture::new();
    let orphan = fixture.blob(ORPHAN);
    let audit_root = fixture.audit_root();
    let held_path = fixture
        .orbit
        .join("state/audit/blobs")
        .join(&HELD[..2])
        .join(HELD);
    assert!(orphan.exists());

    let fixture_ref = &fixture;
    let audit_root_ref = &audit_root;
    let orphan_ref = &orphan;

    std::thread::scope(|threads| {
        // GC holds the guard for the whole deletion, as `apply` does.
        let guard = audit_writer_guard::acquire(audit_root_ref).expect("gc acquires guard");

        let (started_tx, started_rx) = std::sync::mpsc::channel::<()>();
        let writer = threads.spawn(move || {
            started_rx.recv().expect("writer start signal");
            // Contend for the same guard, then republish under it — models
            // V2SqliteSink::write_blob + write_envelope resuming after GC releases.
            let _held = audit_writer_guard::acquire(audit_root_ref)
                .expect("writer eventually acquires guard");
            fixture_ref.blob(HELD);
            fixture_ref.insert_v2(
                "republished",
                "live",
                "2026-07-10T00:00:00Z",
                &format!(r#"{{"blob":"{HELD}"}}"#),
            );
        });
        started_tx.send(()).expect("signal writer");

        // Delete the genuine orphan under the guard, mimicking `apply`'s unlink.
        fs::remove_file(orphan_ref).expect("gc removes orphan under guard");

        drop(guard); // release; the blocked writer now republishes

        writer.join().expect("writer thread");
    });

    assert!(
        !orphan.exists(),
        "GC removed the genuinely-orphaned blob while holding the guard"
    );
    assert!(
        held_path.exists(),
        "the blocked writer republished its blob after GC released and was not lost"
    );
}

#[test]
fn active_jsonl_writer_append_under_guard_blocks_partition_sweep() {
    let fixture = Fixture::new();
    let loop_root = fixture.orbit.join("state/audit/v2_loop");
    fs::create_dir_all(&loop_root).expect("loop root");
    let partition = loop_root.join("run-live.jsonl");
    // Every seeded envelope predates the cutoff, so the partition plans as an
    // eligible sweep candidate.
    fs::write(
        &partition,
        "{\"ts\":\"2026-01-01T00:00:00Z\"}\n{\"ts\":\"2026-01-02T00:00:00Z\"}\n",
    )
    .expect("seed jsonl");

    let collector = fixture.collector();
    let scope = fixture.scope();
    let frozen = fixture.frozen_candidate(
        &collector,
        &scope,
        &format!("jsonl:{}", partition.display()),
    );

    let audit_root = fixture.audit_root();
    let context = GcContext {
        scope: &scope,
        retention_override: Some("30d"),
        clock: &fixture.clock,
    };
    let audit_root_ref = &audit_root;
    let partition_ref = &partition;
    let collector_ref = &collector;
    let context_ref = &context;
    let frozen_ref = &frozen;

    std::thread::scope(|threads| {
        // The JSONL writer holds the guard and appends a live envelope.
        let guard =
            audit_writer_guard::acquire(audit_root_ref).expect("jsonl writer acquires guard");

        let (started_tx, started_rx) = std::sync::mpsc::channel::<()>();
        let apply_handle = threads.spawn(move || {
            started_rx.recv().expect("gc start signal");
            collector_ref.apply(frozen_ref, context_ref)
        });
        started_tx.send(()).expect("signal gc");

        // Append a fresh, in-retention envelope while holding the guard.
        let mut file = fs::OpenOptions::new()
            .append(true)
            .open(partition_ref)
            .expect("open partition for append");
        writeln!(file, "{{\"ts\":\"2026-07-11T00:00:00Z\"}}").expect("append live envelope");
        file.sync_all().ok();
        drop(file);

        drop(guard); // release; the blocked apply now re-fingerprints under the guard

        let applied = apply_handle.join().expect("gc apply thread");
        assert!(
            applied.is_err(),
            "apply must refuse a partition an active writer appended to, got {applied:?}"
        );
    });

    assert!(
        partition.exists(),
        "the active JSONL partition must survive"
    );
    let contents = fs::read_to_string(&partition).expect("read partition");
    assert!(
        contents.contains("2026-07-11"),
        "the live envelope appended under the guard must not be lost"
    );
}

// The next block covers the ORB-10186 second-round fix: the durable
// pending-publication root that spans the *split* blob→reference transaction.
// `V2SqliteSink::write_blob` records a `pending/<hash>` marker (the same
// `audit_pending::mark` call these tests use) atomically with the blob under the
// guard, and `write_envelope`/`emit` clears it (via `audit_pending::clear_published`)
// once the referencing row is durable. So GC cannot sweep a just-written blob
// between the two real sink calls, and no published reference can point at a
// swept blob.

const FRESH: fn() -> DateTime<Utc> = || Utc.with_ymd_and_hms(2026, 7, 12, 0, 0, 0).unwrap();
const STALE: fn() -> DateTime<Utc> = || Utc.with_ymd_and_hms(2026, 5, 1, 0, 0, 0).unwrap();

#[test]
fn fresh_pending_marker_protects_blob_and_apply_fails_closed() {
    // Models the exact interleaving the reviewer flagged: a blob written by
    // `write_blob` (marker present) whose reference has not yet been published,
    // with GC trying to sweep it in between. `apply` must fail closed.
    let fixture = Fixture::new();
    let blob = fixture.blob(ORPHAN);
    let collector = fixture.collector();
    let scope = fixture.scope();
    let audit_root = fixture.audit_root();

    // Seed a *stale* marker first so the blob still plans as an ordinary sweep
    // candidate we can capture (a fresh marker would keep it out of the plan
    // entirely — see the lifecycle test below).
    audit_pending::mark(&audit_root, ORPHAN, STALE()).expect("stale marker");
    let candidate = fixture.frozen_candidate(&collector, &scope, &format!("blob:{ORPHAN}"));

    // A real `write_blob` now runs: the marker is refreshed into the retention
    // window, exactly as the sink stamps it.
    audit_pending::mark(&audit_root, ORPHAN, FRESH()).expect("fresh marker");

    let context = GcContext {
        scope: &scope,
        retention_override: Some("30d"),
        clock: &fixture.clock,
    };
    let applied = collector.apply(&candidate, &context);
    assert!(
        applied.is_err(),
        "apply must fail closed while the blob is pending publication, got {applied:?}"
    );
    assert!(
        blob.exists(),
        "a blob inside its pending-publication window must survive the sweep"
    );
}

#[test]
fn pending_publish_gc_lifecycle_never_strands_reference() {
    // End-to-end over the real sink primitives: write_blob (mark) → GC →
    // write_envelope (persist row + clear marker) → GC. At no step is the blob
    // both sweepable and about-to-be-referenced.
    let fixture = Fixture::new();
    let blob = fixture.blob(ORPHAN);
    let audit_root = fixture.audit_root();

    // 1. write_blob: durable pending marker, blob not yet referenced.
    audit_pending::mark(&audit_root, ORPHAN, FRESH()).expect("mark");

    // GC while the reference is in flight: the blob is skipped, never swept.
    let plan = fixture.report(false);
    let ids: Vec<&str> = plan.targets[0]
        .items
        .iter()
        .map(|item| item.id.as_str())
        .collect();
    assert!(
        !ids.contains(&format!("blob:{ORPHAN}").as_str()),
        "a pending blob must not be a sweep candidate"
    );
    assert!(
        plan.targets[0]
            .skipped
            .iter()
            .any(|skip| skip.code == "pending_publication"),
        "the pending blob must be skipped as pending_publication"
    );

    // 2. write_envelope: persist the referencing row, then clear the marker
    //    (row commits before the marker is retired, both under the guard).
    let payload = format!(r#"{{"blob":"{ORPHAN}"}}"#);
    fixture.insert_v2("published", "live", "2026-07-10T00:00:00Z", &payload);
    audit_pending::clear_published(&audit_root, &payload);

    // GC after publication: the blob is now protected by its retained reference.
    let after = fixture.report(false);
    assert!(
        after.targets[0]
            .skipped
            .iter()
            .any(|skip| skip.id == format!("blob:{ORPHAN}") && skip.code == "referenced"),
        "after publication the blob is reachable from retained evidence"
    );
    assert!(
        blob.exists(),
        "the published reference must not point at a swept blob"
    );
}

#[test]
fn stale_pending_marker_is_swept_with_its_orphaned_blob() {
    // A blob written but never published (e.g. a discarded `write_blob`) keeps a
    // marker forever unless reclaimed. Once the marker predates the cutoff its
    // window has closed, so both it and the orphan blob are swept — the leak is
    // bounded by the retention window, not unbounded.
    let fixture = Fixture::new();
    let blob = fixture.blob(ORPHAN);
    let audit_root = fixture.audit_root();
    audit_pending::mark(&audit_root, ORPHAN, STALE()).expect("stale marker");
    let marker = audit_pending::pending_dir(&audit_root).join(ORPHAN);
    assert!(marker.exists(), "precondition: stale marker present");

    let apply = fixture.report(true);
    assert!(
        apply.targets[0]
            .items
            .iter()
            .any(|item| item.id == format!("pending:{ORPHAN}")),
        "a stale pending marker must be a reclamation candidate"
    );
    assert!(!marker.exists(), "the stale pending marker is reclaimed");
    assert!(!blob.exists(), "the orphaned blob it protected is swept");
}

#[test]
fn apply_re_marks_reference_added_after_planning_even_without_contention() {
    // The non-threaded core of the fix: a reference published between planning
    // and apply is caught by the in-apply re-mark under the guard, so the blob
    // is never swept even when no writer is actively contending.
    let fixture = Fixture::new();
    let orphan = fixture.blob(ORPHAN);
    let collector = fixture.collector();
    let scope = fixture.scope();
    let frozen = fixture.frozen_candidate(&collector, &scope, &format!("blob:{ORPHAN}"));

    // A writer publishes a retained reference after the plan froze the candidate.
    fixture.insert_v2(
        "late-reference",
        "live",
        "2026-07-10T00:00:00Z",
        &format!(r#"{{"blob":"{ORPHAN}"}}"#),
    );

    let context = GcContext {
        scope: &scope,
        retention_override: Some("30d"),
        clock: &fixture.clock,
    };
    let applied = collector.apply(&frozen, &context);
    assert!(applied.is_err(), "apply must fail closed, got {applied:?}");
    assert!(orphan.exists(), "the newly-referenced blob must survive");
    // Guard against a false positive: without the late reference the same blob
    // is swept.
    let clean = Fixture::new();
    let clean_orphan = clean.blob(ORPHAN);
    let clean_collector = clean.collector();
    let clean_scope = clean.scope();
    let clean_frozen =
        clean.frozen_candidate(&clean_collector, &clean_scope, &format!("blob:{ORPHAN}"));
    let clean_context = GcContext {
        scope: &clean_scope,
        retention_override: Some("30d"),
        clock: &clean.clock,
    };
    assert!(matches!(
        clean_collector
            .revalidate(&clean_frozen, &clean_context)
            .expect("revalidate clean"),
        GcRevalidation::Ready
    ));
    let _ = clean_collector.apply(&clean_frozen, &clean_context);
    assert!(!clean_orphan.exists(), "a truly-orphan blob is still swept");
}
