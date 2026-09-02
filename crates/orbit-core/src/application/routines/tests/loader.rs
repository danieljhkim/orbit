//! Origin-aware routine loading tests [ORB-10258]: committed definitions under
//! `.orbit/routines/` must pin a host and fail closed when they do not; local
//! definitions under `.orbit/routines/local/` are implicit to the loading host,
//! may not name another host, and load with no registry/network; and a name
//! defined by more than one origin fails deterministically naming both sources.
//!
//! These drive `collect_routines` over a real seeded source workspace (the same
//! discovery path the sweep uses), so origin resolution, fail-before-dispatch,
//! and duplicate handling are exercised end-to-end rather than at the parser.

use std::fs;
use std::path::{Path, PathBuf};

use chrono::Utc;
use orbit_types::workspace::{Workspace, WorkspaceStatus};
use tempfile::{TempDir, tempdir};

use crate::OrbitRuntime;
use crate::application::routines::loader::{
    LoadedRoutine, RoutineCollection, RoutineOrigin, collect_routines,
};

const HOST: &str = "test-host";

const NOOP_JOB: &str = "schemaVersion: 2\n\
kind: Job\n\
metadata:\n  name: noop\n\
spec:\n  state: enabled\n  kind: workflow\n  max_active_runs: 1\n  \
steps:\n    - id: noop\n      target: activity:worktree_setup\n      \
default_input:\n        task_id: \"qa\"\n";

/// A seeded global root with one active source workspace `polaris`. Returns the
/// tempdir (kept alive by the caller), the global root, and the workspace's
/// `.orbit` dir so tests can drop routine files under `routines/` and
/// `routines/local/` before collecting.
struct SourceWorkspace {
    _tmp: TempDir,
    workspace: Workspace,
    runtime: OrbitRuntime,
    routines_dir: PathBuf,
    local_dir: PathBuf,
}

fn seed_source_workspace() -> SourceWorkspace {
    let tmp = tempdir().unwrap();
    let global = tmp.path().join("global");
    let ws_root = tmp.path().join("polaris");
    let ws_orbit = ws_root.join(".orbit");
    let routines_dir = ws_orbit.join("routines");
    let local_dir = routines_dir.join("local");
    fs::create_dir_all(global.join("state")).unwrap();
    fs::create_dir_all(&local_dir).unwrap();
    fs::create_dir_all(ws_orbit.join("resources/jobs")).unwrap();

    fs::write(
        ws_orbit.join("config.toml"),
        "[routines]\nrole = \"source\"\n",
    )
    .unwrap();
    fs::write(ws_orbit.join("resources/jobs/noop.yaml"), NOOP_JOB).unwrap();

    let workspace = Workspace {
        id: "ws-1".to_string(),
        name: "polaris".to_string(),
        owner_machine_id: None,
        git_remote: None,
        ship_mode: None,
        base_branch: "agent-main".to_string(),
        status: WorkspaceStatus::Active,
        created_at: Utc::now(),
        updated_at: Utc::now(),
    };
    let runtime = OrbitRuntime::from_roots(&global, &ws_orbit).unwrap();

    SourceWorkspace {
        _tmp: tmp,
        workspace,
        runtime,
        routines_dir,
        local_dir,
    }
}

fn write_routine(dir: &Path, file: &str, body: &str) {
    fs::write(dir.join(file), body).unwrap();
}

/// Collect against `HOST`, the same seam the sweep and status projections use.
fn collect(ws: &SourceWorkspace) -> RoutineCollection {
    collect_routines(&[(ws.workspace.clone(), ws.runtime.clone())], HOST)
}

fn find<'a>(collection: &'a RoutineCollection, name: &str) -> Option<&'a LoadedRoutine> {
    collection
        .routines
        .iter()
        .find(|r| r.definition.name == name)
}

fn committed(name: &str, hosts: &str) -> String {
    format!(
        "schemaVersion: 1\nname: {name}\nhosts: {hosts}\n\
         trigger: {{ cron: \"* * * * *\" }}\ntarget: job:noop\n"
    )
}

fn local(name: &str, hosts_line: &str) -> String {
    format!(
        "schemaVersion: 1\nname: {name}\n{hosts_line}\
         trigger: {{ cron: \"* * * * *\" }}\ntarget: job:noop\n"
    )
}

// ---- committed origin -----------------------------------------------------

#[test]
fn committed_pinned_routine_loads_with_committed_origin() {
    let ws = seed_source_workspace();
    write_routine(
        &ws.routines_dir,
        "pinned.yaml",
        &committed("committed-pinned", &format!("[{HOST}]")),
    );

    let collection = collect(&ws);
    let routine = find(&collection, "committed-pinned").expect("committed routine loads");
    assert_eq!(routine.origin, RoutineOrigin::Committed);
    assert_eq!(routine.definition.hosts, vec![HOST.to_string()]);
    assert!(collection.errors.is_empty(), "{:?}", collection.errors);
}

#[test]
fn disabled_committed_routine_still_loads() {
    let ws = seed_source_workspace();
    write_routine(
        &ws.routines_dir,
        "disabled.yaml",
        &format!(
            "schemaVersion: 1\nname: committed-disabled\nenabled: false\nhosts: [{HOST}]\n\
             trigger: {{ cron: \"* * * * *\" }}\ntarget: job:noop\n"
        ),
    );

    let collection = collect(&ws);
    let routine = find(&collection, "committed-disabled").expect("disabled routine loads");
    assert_eq!(routine.origin, RoutineOrigin::Committed);
    assert!(!routine.definition.enabled);
}

#[test]
fn committed_missing_hosts_fails_before_dispatch() {
    let ws = seed_source_workspace();
    write_routine(
        &ws.routines_dir,
        "nohosts.yaml",
        // No `hosts:` at all — a committed definition must pin a host.
        "schemaVersion: 1\nname: committed-nohosts\n\
         trigger: { cron: \"* * * * *\" }\ntarget: job:noop\n",
    );

    let collection = collect(&ws);
    assert!(
        find(&collection, "committed-nohosts").is_none(),
        "unpinned committed routine must not load (never reaches dispatch)"
    );
    assert!(
        collection
            .errors
            .iter()
            .any(|e| e.message.contains("must pin at least one host")),
        "{:?}",
        collection.errors
    );
}

#[test]
fn committed_blank_hosts_fails_closed() {
    let ws = seed_source_workspace();
    write_routine(
        &ws.routines_dir,
        "blank.yaml",
        &committed("committed-blank", "[\"   \"]"),
    );

    let collection = collect(&ws);
    assert!(find(&collection, "committed-blank").is_none());
    assert!(
        collection
            .errors
            .iter()
            .any(|e| e.message.contains("empty entries")),
        "{:?}",
        collection.errors
    );
}

// ---- local origin ---------------------------------------------------------

#[test]
fn local_without_hosts_loads_offline_pinned_to_this_host() {
    let ws = seed_source_workspace();
    // No registry cache, no network — discovery reads only the local registry.
    write_routine(&ws.local_dir, "personal.yaml", &local("local-nohost", ""));

    let collection = collect(&ws);
    let routine = find(&collection, "local-nohost").expect("local routine loads without a host");
    assert_eq!(routine.origin, RoutineOrigin::Local);
    // Normalized to an implicit pin on the loading host so the sweep fires it.
    assert_eq!(routine.definition.hosts, vec![HOST.to_string()]);
    assert!(collection.errors.is_empty(), "{:?}", collection.errors);
}

#[test]
fn local_naming_the_loading_host_is_accepted() {
    let ws = seed_source_workspace();
    write_routine(
        &ws.local_dir,
        "explicit.yaml",
        &local("local-explicit", &format!("hosts: [{HOST}]\n")),
    );

    let collection = collect(&ws);
    let routine =
        find(&collection, "local-explicit").expect("local routine naming this host loads");
    assert_eq!(routine.origin, RoutineOrigin::Local);
    assert_eq!(routine.definition.hosts, vec![HOST.to_string()]);
}

#[test]
fn local_remote_pin_is_refused_naming_file_and_routine() {
    let ws = seed_source_workspace();
    write_routine(
        &ws.local_dir,
        "remote.yaml",
        &local("local-remote", "hosts: [other-host]\n"),
    );

    let collection = collect(&ws);
    assert!(
        find(&collection, "local-remote").is_none(),
        "a local definition naming another host must not load"
    );
    let error = collection
        .errors
        .iter()
        .find(|e| e.message.contains("local-remote"))
        .expect("error names the routine");
    assert!(
        error.message.contains("other-host"),
        "error names the offending pin: {}",
        error.message
    );
    assert!(
        error
            .path
            .as_ref()
            .is_some_and(|p| p.ends_with("remote.yaml")),
        "error names the file: {:?}",
        error.path
    );
}

// ---- cross-origin duplicate names -----------------------------------------

#[test]
fn duplicate_name_across_committed_and_local_fails_deterministically() {
    let ws = seed_source_workspace();
    write_routine(
        &ws.routines_dir,
        "dup.yaml",
        &committed("dup-name", &format!("[{HOST}]")),
    );
    write_routine(&ws.local_dir, "dup.yaml", &local("dup-name", ""));

    let collection = collect(&ws);
    // Neither definition may silently shadow the other: both are dropped.
    assert!(
        find(&collection, "dup-name").is_none(),
        "a cross-origin name collision drops every colliding definition"
    );
    let collision_errors: Vec<&str> = collection
        .errors
        .iter()
        .filter(|e| e.message.contains("dup-name"))
        .map(|e| e.message.as_str())
        .collect();
    assert_eq!(
        collision_errors.len(),
        2,
        "one error row per colliding definition: {collision_errors:?}"
    );
    // Both origins are reported in each row so the conflict is diagnosable.
    for message in &collision_errors {
        assert!(
            message.contains("committed origin") && message.contains("local origin"),
            "collision names both sources: {message}"
        );
    }
}
