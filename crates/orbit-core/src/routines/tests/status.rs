use std::fs;

use orbit_common::protocol::yaml::parse_routine_yaml;
use tempfile::tempdir;

use super::super::loader::{LoadedRoutine, RoutineOrigin};
use super::super::status::{RoutineToggleOutcome, set_routine_enabled};

fn loaded(path: std::path::PathBuf) -> LoadedRoutine {
    let raw = fs::read_to_string(&path).expect("read fixture");
    LoadedRoutine {
        definition: parse_routine_yaml(&raw).expect("parse fixture"),
        origin: RoutineOrigin::Committed,
        source_workspace: "polaris".to_string(),
        source_orbit_dir: path.parent().expect("fixture parent").to_path_buf(),
        path,
    }
}

#[test]
fn routine_toggle_preserves_comments_and_every_field_but_enabled() {
    let root = tempdir().expect("temp root");
    let path = root.path().join("nightly.yaml");
    fs::write(
        &path,
        "schemaVersion: 1\nname: nightly\ndescription: keep this text\nenabled: true # reviewed\nhosts: [host-a]\ntrigger:\n  cron: '0 2 * * *'\ntarget: job:nightly\n",
    )
    .expect("write fixture");
    let routine = loaded(path.clone());

    assert_eq!(
        set_routine_enabled(&routine, "host-a", true, false).expect("disable"),
        RoutineToggleOutcome::Changed
    );
    let changed = fs::read_to_string(&path).expect("read changed fixture");
    assert!(changed.contains("enabled: false # reviewed"));
    assert!(changed.contains("description: keep this text"));
    assert!(!parse_routine_yaml(&changed).expect("parse changed").enabled);
}

#[test]
fn routine_toggle_inserts_missing_default_and_rejects_stale_duplicate() {
    let root = tempdir().expect("temp root");
    let path = root.path().join("nightly.yaml");
    fs::write(
        &path,
        "schemaVersion: 1\nname: nightly\nhosts: [host-a]\ntrigger:\n  cron: '0 2 * * *'\ntarget: job:nightly\n",
    )
    .expect("write fixture");
    let routine = loaded(path.clone());

    assert_eq!(
        set_routine_enabled(&routine, "host-a", true, false).expect("first disable"),
        RoutineToggleOutcome::Changed
    );
    let after_first = fs::read_to_string(&path).expect("read first result");
    assert_eq!(
        set_routine_enabled(&routine, "host-a", true, false).expect("stale duplicate"),
        RoutineToggleOutcome::Conflict {
            actual_enabled: false
        }
    );
    assert_eq!(
        fs::read_to_string(&path).expect("read duplicate result"),
        after_first,
        "stale duplicate must not rewrite the definition"
    );
}

#[test]
fn routine_toggle_reports_an_exact_noop_without_rewriting() {
    let root = tempdir().expect("temp root");
    let path = root.path().join("nightly.yaml");
    fs::write(
        &path,
        "schemaVersion: 1\nname: nightly\nenabled: false\nhosts: [host-a]\ntrigger:\n  cron: '0 2 * * *'\ntarget: job:nightly\n",
    )
    .expect("write fixture");
    let routine = loaded(path.clone());
    let before = fs::metadata(&path)
        .expect("fixture metadata")
        .modified()
        .expect("fixture mtime");

    assert_eq!(
        set_routine_enabled(&routine, "host-a", false, false).expect("noop"),
        RoutineToggleOutcome::Unchanged
    );
    assert_eq!(
        fs::metadata(&path)
            .expect("fixture metadata after noop")
            .modified()
            .expect("fixture mtime after noop"),
        before
    );
}
