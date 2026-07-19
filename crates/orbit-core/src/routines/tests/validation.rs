use std::cell::Cell;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Duration, TimeZone, Utc};
use orbit_common::types::{
    HostStatus, JobRunState, REGISTRY_CACHE_SCHEMA_VERSION, REGISTRY_SNAPSHOT_SCHEMA_VERSION,
    RegistryAliasV1, RegistryCacheV1, RegistryHostV1, RegistrySnapshotV1, RoutineDefinition,
    parse_routine_yaml,
};
use orbit_remote::{HOST_IDENTITY_SCHEMA_VERSION, HostIdentity, HostMode, RegistryCacheService};
use orbit_store::{RoutineFireIntentParams, Store};
use tempfile::tempdir;

use crate::OrbitError;
use crate::routines::loader::{LoadedRoutine, RoutineCollection, RoutineOrigin};
use crate::routines::sweep::{RoutineDispatch, SweepOptions, run_sweep_core_with_registry};
use crate::routines::validation::{
    RoutineDiagnosticSeverity, RoutineRegistryCacheView, RoutineRegistryView,
    load_routine_registry_view, validate_routine_pins,
};

fn ts(minute: u32, second: u32) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 7, 18, 12, minute, second)
        .single()
        .expect("valid timestamp")
}

fn identity(machine_id: &str, host_id: &str, mode: HostMode) -> HostIdentity {
    HostIdentity {
        schema_version: HOST_IDENTITY_SCHEMA_VERSION,
        machine_id: machine_id.to_string(),
        host_id: host_id.to_string(),
        mode,
    }
}

fn host(
    machine_id: &str,
    host_id: &str,
    status: HostStatus,
    last_seen_at: Option<DateTime<Utc>>,
    aliases: &[&str],
) -> RegistryHostV1 {
    RegistryHostV1 {
        machine_id: machine_id.to_string(),
        host_id: host_id.to_string(),
        labels: BTreeSet::new(),
        status,
        registered_at: ts(0, 0),
        updated_at: ts(0, 0),
        retired_at: (status == HostStatus::Retired).then(|| ts(1, 0)),
        last_seen_at,
        aliases: aliases
            .iter()
            .map(|alias| RegistryAliasV1 {
                alias_host_id: (*alias).to_string(),
                created_at: ts(0, 0),
                warning: format!("'{alias}' is a permanent alias"),
            })
            .collect(),
        presence: Vec::new(),
    }
}

fn snapshot(hosts: Vec<RegistryHostV1>) -> RegistrySnapshotV1 {
    RegistrySnapshotV1 {
        schema_version: REGISTRY_SNAPSHOT_SCHEMA_VERSION,
        hub_machine_id: Some("hm_hub".to_string()),
        registry_revision: 1,
        hosts,
        workspaces: Vec::new(),
    }
}

fn codes(validation: &crate::routines::RoutinePinValidation) -> Vec<&'static str> {
    validation
        .diagnostics
        .iter()
        .map(|diagnostic| diagnostic.code)
        .collect()
}

#[test]
fn current_registry_resolves_alias_and_keeps_valid_pin_despite_invalid_extra() {
    let now = ts(10, 0);
    let local = identity("hm_local", "local-old", HostMode::Hub);
    let view = RoutineRegistryView::Hub {
        snapshot: snapshot(vec![host(
            "hm_local",
            "local-new",
            HostStatus::Active,
            Some(now - Duration::minutes(5)),
            &["local-old"],
        )]),
    };
    let validation = validate_routine_pins(
        &local,
        RoutineOrigin::Committed,
        &["local-old".to_string(), "missing".to_string()],
        &view,
        now,
        Duration::minutes(5),
    );

    assert!(
        validation.eligible,
        "alias resolves to the local machine_id"
    );
    assert!(codes(&validation).contains(&"host_alias"));
    assert!(codes(&validation).contains(&"host_unknown"));
    assert!(!codes(&validation).contains(&"host_quiet"));
    let unknown = validation
        .diagnostics
        .iter()
        .find(|diagnostic| diagnostic.code == "host_unknown")
        .expect("unknown diagnostic");
    assert_eq!(unknown.severity, RoutineDiagnosticSeverity::Error);

    let one_second_later = validate_routine_pins(
        &local,
        RoutineOrigin::Committed,
        &["local-old".to_string()],
        &view,
        now + Duration::seconds(1),
        Duration::minutes(5),
    );
    assert!(codes(&one_second_later).contains(&"host_quiet"));
}

#[test]
fn stale_registry_is_warning_only_and_exact_local_fallback_remains_eligible() {
    let now = ts(10, 0);
    let local = identity("hm_local", "local", HostMode::Spoke);
    let cache = RegistryCacheV1 {
        schema_version: REGISTRY_CACHE_SCHEMA_VERSION,
        received_at: now - Duration::minutes(20),
        snapshot: snapshot(vec![
            host(
                "hm_local",
                "renamed",
                HostStatus::Active,
                Some(now - Duration::minutes(20)),
                &["old-local"],
            ),
            host(
                "hm_retired",
                "retired",
                HostStatus::Retired,
                Some(now - Duration::minutes(20)),
                &[],
            ),
        ]),
    };
    let view = RoutineRegistryView::Spoke {
        cache: RoutineRegistryCacheView::Stale {
            snapshot: Box::new(cache.snapshot),
            age_seconds: 1_200,
        },
    };
    let validation = validate_routine_pins(
        &local,
        RoutineOrigin::Committed,
        &[
            "local".to_string(),
            "old-local".to_string(),
            "retired".to_string(),
            "missing".to_string(),
        ],
        &view,
        now,
        Duration::minutes(5),
    );

    assert!(validation.eligible);
    for diagnostic in &validation.diagnostics {
        assert_eq!(diagnostic.severity, RoutineDiagnosticSeverity::Warning);
    }
    assert!(codes(&validation).contains(&"registry_cache_stale"));
    assert!(codes(&validation).contains(&"host_alias"));
    assert!(codes(&validation).contains(&"host_retired"));
    assert!(codes(&validation).contains(&"host_unknown"));
    assert!(
        validation
            .diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.code == "host_quiet")
            .all(|diagnostic| diagnostic.stale)
    );
}

#[test]
fn collision_is_an_error_only_for_current_authoritative_data() {
    let now = ts(10, 0);
    let local = identity("hm_a", "a", HostMode::Hub);
    let view = RoutineRegistryView::Hub {
        snapshot: snapshot(vec![
            host("hm_a", "a", HostStatus::Active, Some(now), &["shared"]),
            host("hm_b", "b", HostStatus::Active, Some(now), &["shared"]),
        ]),
    };
    let validation = validate_routine_pins(
        &local,
        RoutineOrigin::Committed,
        &["shared".to_string()],
        &view,
        now,
        Duration::minutes(5),
    );
    assert!(!validation.eligible);
    assert_eq!(validation.diagnostics[0].code, "host_collision");
    assert_eq!(
        validation.diagnostics[0].severity,
        RoutineDiagnosticSeverity::Error
    );
}

#[test]
fn current_unknown_and_retired_pins_are_independently_unusable_errors() {
    let now = ts(10, 0);
    let local = identity("hm_local", "local", HostMode::Hub);
    let view = RoutineRegistryView::Hub {
        snapshot: snapshot(vec![host(
            "hm_retired",
            "retired",
            HostStatus::Retired,
            Some(now),
            &[],
        )]),
    };
    let validation = validate_routine_pins(
        &local,
        RoutineOrigin::Committed,
        &["retired".to_string(), "missing".to_string()],
        &view,
        now,
        Duration::minutes(5),
    );

    assert!(!validation.eligible);
    assert_eq!(codes(&validation), vec!["host_retired", "host_unknown"]);
    assert!(
        validation
            .diagnostics
            .iter()
            .all(|diagnostic| diagnostic.severity == RoutineDiagnosticSeverity::Error)
    );
}

#[test]
fn unregistered_hub_keeps_exact_local_pin_eligible_with_upgrade_warning() {
    let now = ts(10, 0);
    let local = identity("hm_local", "local", HostMode::Hub);
    let view = RoutineRegistryView::Hub {
        snapshot: snapshot(Vec::new()),
    };

    let validation = validate_routine_pins(
        &local,
        RoutineOrigin::Committed,
        &["local".to_string(), "missing".to_string()],
        &view,
        now,
        Duration::minutes(5),
    );

    assert!(
        validation.eligible,
        "exact local pin preserves pre-registry behavior"
    );
    assert_eq!(
        codes(&validation),
        vec!["local_host_unregistered", "host_unknown"]
    );
    assert_eq!(
        validation.diagnostics[0].severity,
        RoutineDiagnosticSeverity::Warning
    );
    assert_eq!(
        validation.diagnostics[1].severity,
        RoutineDiagnosticSeverity::Error
    );
}

#[test]
fn every_unusable_spoke_cache_keeps_exact_local_committed_pin_eligible() {
    let now = ts(10, 0);
    let local = identity("hm_local", "local", HostMode::Spoke);
    let cases = [
        (RoutineRegistryCacheView::Missing, "registry_cache_missing"),
        (
            RoutineRegistryCacheView::Malformed {
                reason: "invalid fixture".to_string(),
            },
            "registry_cache_malformed",
        ),
        (
            RoutineRegistryCacheView::UnsupportedFuture { schema_version: 2 },
            "registry_cache_future_schema",
        ),
    ];

    for (cache, expected_code) in cases {
        let validation = validate_routine_pins(
            &local,
            RoutineOrigin::Committed,
            &["local".to_string()],
            &RoutineRegistryView::Spoke { cache },
            now,
            Duration::minutes(5),
        );
        assert!(validation.eligible, "{expected_code} exact-local fallback");
        assert_eq!(codes(&validation), vec![expected_code]);
        assert_eq!(
            validation.diagnostics[0].severity,
            RoutineDiagnosticSeverity::Warning
        );
    }
}

#[test]
fn cache_loader_preserves_malformed_and_future_bytes_and_uses_strict_age_boundary() {
    let now = ts(10, 0);
    let local = identity("hm_local", "local", HostMode::Spoke);

    for (body, expected) in [
        (b"{not json".as_slice(), "registry_cache_malformed"),
        (
            br#"{"schema_version":2,"received_at":"2026-07-18T12:00:00Z","snapshot":{}}"#
                .as_slice(),
            "registry_cache_future_schema",
        ),
    ] {
        let root = tempdir().expect("temp root");
        let path = root.path().join("registry-cache.json");
        std::fs::write(&path, body).expect("write cache");
        let store = Store::open(&root.path().join("orbit.db")).expect("store");
        let view =
            load_routine_registry_view(root.path(), &store, &local, now, Duration::minutes(5))
                .expect("classified cache");
        assert_eq!(view.status().diagnostics[0].code, expected);
        assert_eq!(std::fs::read(&path).expect("read cache"), body);
    }

    let root = tempdir().expect("temp root");
    let service = RegistryCacheService::new(root.path());
    service
        .refresh(snapshot(Vec::new()), now)
        .expect("seed cache");
    let store = Store::open(&root.path().join("orbit.db")).expect("store");
    let exact = load_routine_registry_view(
        root.path(),
        &store,
        &local,
        now + Duration::minutes(5),
        Duration::minutes(5),
    )
    .expect("exact threshold");
    assert_eq!(exact.status().state, "current");
    let over = load_routine_registry_view(
        root.path(),
        &store,
        &local,
        now + Duration::minutes(5) + Duration::seconds(1),
        Duration::minutes(5),
    )
    .expect("over threshold");
    assert_eq!(over.status().state, "stale");
}

fn loaded_routine(pin: &str) -> LoadedRoutine {
    let yaml = format!(
        "schemaVersion: 1\nname: move-me\nenabled: true\nhosts: [{pin}]\n\
         trigger:\n  cron: \"* * * * *\"\n  missed_run: catch_up_once\n\
         target: job:noop\n"
    );
    let definition: RoutineDefinition = parse_routine_yaml(&yaml).expect("routine");
    LoadedRoutine {
        definition,
        origin: RoutineOrigin::Committed,
        source_workspace: "orbit".to_string(),
        source_orbit_dir: PathBuf::from("/orbit/.orbit"),
        path: PathBuf::from("/orbit/.orbit/routines/move-me.yaml"),
    }
}

#[derive(Default)]
struct FakeDispatch {
    submissions: Cell<u32>,
}

impl RoutineDispatch for FakeDispatch {
    fn submit(&self, _source: &Path, _job: &str, _actor: &str) -> Result<String, OrbitError> {
        let next = self.submissions.get() + 1;
        self.submissions.set(next);
        Ok(format!("run-{next}"))
    }

    fn run_state(&self, _source: &Path, _run_id: &str) -> Option<JobRunState> {
        None
    }
}

#[test]
fn reassignment_preserves_a_state_and_baselines_b_without_backfill() {
    let roots = tempdir().expect("roots");
    let root_a = roots.path().join("a");
    let root_b = roots.path().join("b");
    std::fs::create_dir_all(&root_a).expect("root a");
    std::fs::create_dir_all(&root_b).expect("root b");
    let store_a = Store::open(&root_a.join("orbit.db")).expect("store a");
    let store_b = Store::open(&root_b.join("orbit.db")).expect("store b");
    let now = ts(30, 0);

    store_a
        .routine_record_baseline("move-me", &(now - Duration::hours(2)).to_rfc3339())
        .expect("a baseline");
    store_a.routine_pause("move-me", "test").expect("a pause");
    store_a
        .routine_record_fire_intent(&RoutineFireIntentParams {
            routine_name: "move-me".to_string(),
            slot: (now - Duration::hours(1)).to_rfc3339(),
            attempt: 1,
            source_workspace: "orbit".to_string(),
        })
        .expect("unresolved a fire");
    let before_cursor = store_a.routine_cursor("move-me").expect("cursor");
    let before_fires = store_a.routine_recent_fires("move-me", 10).expect("fires");
    let before_pauses = store_a.routine_pauses().expect("pauses");

    let registry = RoutineRegistryView::Hub {
        snapshot: snapshot(vec![
            host("hm_a", "a", HostStatus::Active, Some(now), &[]),
            host("hm_b", "b", HostStatus::Active, Some(now), &[]),
        ]),
    };
    let moved = RoutineCollection {
        routines: vec![loaded_routine("b")],
        errors: Vec::new(),
    };
    let dispatch_a = FakeDispatch::default();
    let a_reports = run_sweep_core_with_registry(
        &store_a,
        &identity("hm_a", "a", HostMode::Hub),
        &registry,
        &moved,
        &dispatch_a,
        SweepOptions::default(),
        now,
    )
    .expect("a sweep after reassignment");
    assert_eq!(a_reports[0].reason.as_deref(), Some("host_not_pinned"));
    assert_eq!(store_a.routine_cursor("move-me").unwrap(), before_cursor);
    assert_eq!(
        store_a.routine_recent_fires("move-me", 10).unwrap(),
        before_fires
    );
    assert_eq!(store_a.routine_pauses().unwrap(), before_pauses);
    assert_eq!(dispatch_a.submissions.get(), 0);

    let dispatch_b = FakeDispatch::default();
    let first_b = run_sweep_core_with_registry(
        &store_b,
        &identity("hm_b", "b", HostMode::Hub),
        &registry,
        &moved,
        &dispatch_b,
        SweepOptions::default(),
        now,
    )
    .expect("first b sweep");
    assert_eq!(first_b[0].action, "baselined");
    assert!(
        store_b
            .routine_recent_fires("move-me", 10)
            .unwrap()
            .is_empty()
    );

    let second_b = run_sweep_core_with_registry(
        &store_b,
        &identity("hm_b", "b", HostMode::Hub),
        &registry,
        &moved,
        &dispatch_b,
        SweepOptions::default(),
        now + Duration::minutes(1) + Duration::seconds(1),
    )
    .expect("next natural b slot");
    assert_eq!(second_b[0].action, "fired");
    assert_eq!(dispatch_b.submissions.get(), 1);
    assert_eq!(
        store_b.routine_recent_fires("move-me", 10).unwrap().len(),
        1
    );
}

#[test]
fn local_origin_bypasses_missing_cache() {
    let validation = validate_routine_pins(
        &identity("hm_local", "local", HostMode::Spoke),
        RoutineOrigin::Local,
        &["local".to_string()],
        &RoutineRegistryView::Spoke {
            cache: RoutineRegistryCacheView::Missing,
        },
        ts(10, 0),
        Duration::minutes(5),
    );
    assert!(validation.eligible);
    assert!(validation.diagnostics.is_empty());
}
