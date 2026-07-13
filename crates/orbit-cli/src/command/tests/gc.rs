use clap::{CommandFactory, Parser};
use tempfile::tempdir;

use orbit_common::types::TaskStatus;
use orbit_core::OrbitRuntime;
use orbit_core::command::task::{TaskAddParams, TaskUpdateParams};

use super::super::gc::GcCommand;
use super::super::{Cli, Commands, Execute, gc::GcTargetArg};

#[test]
fn gc_help_lists_every_target_and_uniform_flags() {
    let mut root = Cli::command();
    let root_help = root.render_long_help().to_string();
    assert!(root_help.contains("gc          Plan or apply garbage collection"));
    let help = root
        .find_subcommand_mut("gc")
        .expect("gc command")
        .render_long_help()
        .to_string();
    for value in [
        "worktrees",
        "runs",
        "logs",
        "diagnostics",
        "audit",
        "skills",
        "tasks",
        "all",
        "--apply",
        "--json",
        "--retention",
        "--include-rejected",
        "--success-retention-days",
        "--failure-retention-days",
        "--archive-after-days",
        "--purge-after-days",
        "--failure-archive-after-days",
        "--failure-purge-after-days",
        "--workspace",
        "--global",
    ] {
        assert!(help.contains(value), "missing `{value}` from help:\n{help}");
    }
}

#[test]
fn gc_parses_qualified_run_retention_overrides() {
    let cli = Cli::parse_from([
        "orbit",
        "gc",
        "runs",
        "--archive-after-days",
        "7",
        "--purge-after-days",
        "30",
        "--failure-archive-after-days",
        "14",
        "--failure-purge-after-days",
        "90",
    ]);
    match cli.command {
        Commands::Gc(command) => {
            assert_eq!(command.archive_after_days, Some(7));
            assert_eq!(command.purge_after_days, Some(30));
            assert_eq!(command.failure_archive_after_days, Some(14));
            assert_eq!(command.failure_purge_after_days, Some(90));
        }
        _ => panic!("expected gc command"),
    }
}

#[test]
fn gc_parses_qualified_worktree_retention_overrides() {
    let cli = Cli::parse_from([
        "orbit",
        "gc",
        "worktrees",
        "--success-retention-days",
        "2",
        "--failure-retention-days",
        "30",
    ]);
    match cli.command {
        Commands::Gc(command) => {
            assert_eq!(command.success_retention_days, Some(2));
            assert_eq!(command.failure_retention_days, Some(30));
        }
        _ => panic!("expected gc command"),
    }
}

#[test]
fn gc_audit_metadata_tracks_target_and_mutation_gate() {
    let cli = Cli::parse_from(["orbit", "gc", "worktrees", "--apply"]);
    let meta = crate::audit_middleware::extract_command_meta(&cli.command);
    assert_eq!(meta.command, "gc");
    assert_eq!(meta.subcommand.as_deref(), Some("worktrees"));
    assert_eq!(meta.target_type.as_deref(), Some("gc_target"));
    assert_eq!(meta.target_id.as_deref(), Some("worktrees"));
    assert!(
        meta.arguments_json
            .as_deref()
            .is_some_and(|arguments| arguments.contains("\"apply\":true"))
    );
}

#[test]
fn gc_is_plan_only_by_default_and_parses_target() {
    let cli = Cli::parse_from(["orbit", "gc", "runs", "--retention", "30d", "--json"]);
    match cli.command {
        Commands::Gc(command) => {
            assert_eq!(command.target, GcTargetArg::Runs);
            assert!(!command.apply);
            assert!(command.json);
            assert_eq!(command.retention.as_deref(), Some("30d"));
        }
        _ => panic!("expected gc command"),
    }
}

// ORB-10183 P1: run GC is workspace-only; `--global` must be refused before a
// runtime is built or any state is scanned/mutated.
#[test]
fn gc_runs_rejects_global_scope_before_any_mutation() {
    use super::super::gc::GcCommand;
    use orbit_core::{OrbitError, OrbitRuntime};

    let temp = tempfile::tempdir().expect("tempdir");
    let global = temp.path().join("global");
    let orbit = temp.path().join("repo/.orbit");
    std::fs::create_dir_all(&global).expect("global root");
    std::fs::create_dir_all(&orbit).expect("workspace root");
    std::fs::write(orbit.join("config.toml"), "").expect("config");
    let runtime = OrbitRuntime::from_roots(&global, &orbit).expect("runtime");

    let command = GcCommand {
        target: GcTargetArg::Runs,
        apply: true,
        json: false,
        retention: None,
        include_rejected: false,
        success_retention_days: None,
        failure_retention_days: None,
        archive_after_days: None,
        purge_after_days: None,
        failure_archive_after_days: None,
        failure_purge_after_days: None,
        workspace: None,
        global: true,
    };
    let error = command
        .execute(&runtime)
        .expect_err("runs --global must be rejected");
    assert!(
        matches!(error, OrbitError::InvalidInput(_)),
        "expected InvalidInput, got {error:?}"
    );
    // Refused before constructing a collector runtime, acquiring the GC lock, or
    // writing a manifest: no GC state is created under the global root.
    assert!(
        !global.join("state/gc").exists(),
        "no GC manifest/lock state must be created on rejection"
    );
}

#[test]
fn gc_skills_rejects_workspace_selection_without_mutation() {
    use std::fs;

    use orbit_core::OrbitRuntime;
    use orbit_core::command::skill_ownership::{
        GeneratedFile, GeneratedSkill, reconcile_managed_skills,
    };

    let runtime = OrbitRuntime::in_memory().expect("runtime");
    let skills_root = runtime.paths().global_dir.join("skills");
    fs::create_dir_all(&skills_root).expect("skills root");

    // Seed then retire `orbit`, and materialize its generated tree, so a genuine
    // retirement-removal candidate exists on disk before we invoke GC.
    let contents = b"---\nname: orbit\ndescription: d\n---\n".to_vec();
    let files = vec![GeneratedFile {
        relative_path: "SKILL.md".to_string(),
        contents: contents.clone(),
    }];
    let seeded =
        GeneratedSkill::from_files("orbit", Some("1".to_string()), &files).expect("fingerprint");
    reconcile_managed_skills(&skills_root, &[seeded]).expect("seed");
    fs::create_dir_all(skills_root.join("orbit")).expect("dir");
    fs::write(skills_root.join("orbit").join("SKILL.md"), &contents).expect("file");
    reconcile_managed_skills(&skills_root, &[]).expect("retire");

    // `gc skills` is global-only; an explicit `--workspace` selector must be
    // rejected with InvalidInput before any planning or mutation runs.
    let cli = Cli::parse_from([
        "orbit",
        "gc",
        "skills",
        "--workspace",
        "ws_example",
        "--apply",
    ]);
    let Commands::Gc(command) = cli.command else {
        panic!("expected gc command");
    };
    let error = command
        .execute(&runtime)
        .expect_err("workspace selection must be rejected");
    assert!(
        matches!(error, orbit_core::OrbitError::InvalidInput(_)),
        "expected InvalidInput, got {error:?}"
    );

    // No planning or mutation occurred: the retired generated directory that a
    // global GC apply would have reclaimed is untouched.
    assert!(
        skills_root.join("orbit").join("SKILL.md").exists(),
        "gc skills --workspace must not mutate global state"
    );
}

#[test]
fn gc_scope_flags_conflict() {
    let error =
        match Cli::try_parse_from(["orbit", "gc", "tasks", "--workspace", "here", "--global"]) {
            Ok(_) => panic!("scope flags must conflict"),
            Err(error) => error,
        };
    assert!(error.to_string().contains("cannot be used with"));
}

// ORB-10188 P1: the rejected-task opt-in must be reachable from the operator
// surface, not just an internal builder. The tasks-only `--include-rejected`
// flag parses onto the command.
#[test]
fn gc_tasks_parses_include_rejected_flag() {
    let cli = Cli::parse_from(["orbit", "gc", "tasks", "--include-rejected"]);
    match cli.command {
        Commands::Gc(command) => {
            assert_eq!(command.target, GcTargetArg::Tasks);
            assert!(command.include_rejected);
        }
        _ => panic!("expected gc command"),
    }
    // Default (flag omitted) is done-only.
    let cli = Cli::parse_from(["orbit", "gc", "tasks"]);
    match cli.command {
        Commands::Gc(command) => assert!(!command.include_rejected),
        _ => panic!("expected gc command"),
    }
}

// `--include-rejected` is meaningful only for the `tasks` target. Any other
// target must refuse it with InvalidInput before constructing a collector
// runtime or mutating any state.
#[test]
fn gc_include_rejected_rejected_for_non_task_targets() {
    use orbit_core::OrbitError;

    let (_root, runtime, _repo) = test_runtime();
    let command = tasks_gc_command(GcTargetArg::Worktrees, false, true, None);
    let error = command
        .execute(&runtime)
        .expect_err("--include-rejected on a non-tasks target must be rejected");
    assert!(
        matches!(error, OrbitError::InvalidInput(_)),
        "expected InvalidInput, got {error:?}"
    );
}

// End-to-end through the CLI `execute` path: a rejected task older than
// retention is left untouched by default (done-only) and archived only when
// `--include-rejected` is opted in. A done task is archived in both cases,
// proving the default terminal set is `done`-only, not empty.
#[test]
fn gc_tasks_apply_excludes_rejected_by_default() {
    let (_root, runtime, repo) = test_runtime();
    let done = drive_to_done(&runtime, &repo, "done task");
    let rejected = add_task(&runtime, &repo, "rejected task");
    set_status(&runtime, &rejected.id, TaskStatus::Rejected);

    // apply, default (done-only): `retention 0s` makes any past terminal
    // transition eligible under the real clock.
    tasks_gc_command(GcTargetArg::Tasks, true, false, Some("0s"))
        .execute(&runtime)
        .expect("gc tasks apply");

    assert_eq!(
        runtime.get_task(&done.id).expect("done task").status,
        TaskStatus::Archived,
        "done task must be archived by default"
    );
    assert_eq!(
        runtime
            .get_task(&rejected.id)
            .expect("rejected task")
            .status,
        TaskStatus::Rejected,
        "rejected task must be retained when the opt-in is off"
    );
}

#[test]
fn gc_tasks_apply_includes_rejected_when_opted_in() {
    let (_root, runtime, repo) = test_runtime();
    let done = drive_to_done(&runtime, &repo, "done task");
    let rejected = add_task(&runtime, &repo, "rejected task");
    set_status(&runtime, &rejected.id, TaskStatus::Rejected);

    tasks_gc_command(GcTargetArg::Tasks, true, true, Some("0s"))
        .execute(&runtime)
        .expect("gc tasks --include-rejected apply");

    assert_eq!(
        runtime.get_task(&done.id).expect("done task").status,
        TaskStatus::Archived,
        "done task must be archived"
    );
    assert_eq!(
        runtime
            .get_task(&rejected.id)
            .expect("rejected task")
            .status,
        TaskStatus::Archived,
        "rejected task must be archived once --include-rejected is set"
    );
}

// Plan mode never mutates: even with the opt-in on and an eligible rejected
// task, a non-apply run leaves the task in `rejected`.
#[test]
fn gc_tasks_plan_never_mutates_even_with_opt_in() {
    let (_root, runtime, repo) = test_runtime();
    let rejected = add_task(&runtime, &repo, "rejected task");
    set_status(&runtime, &rejected.id, TaskStatus::Rejected);

    tasks_gc_command(GcTargetArg::Tasks, false, true, Some("0s"))
        .execute(&runtime)
        .expect("gc tasks --include-rejected plan");

    assert_eq!(
        runtime
            .get_task(&rejected.id)
            .expect("rejected task")
            .status,
        TaskStatus::Rejected,
        "plan mode must not archive anything"
    );
}

fn test_runtime() -> (tempfile::TempDir, OrbitRuntime, std::path::PathBuf) {
    let root = tempdir().expect("tempdir");
    let global_root = root.path().join("global");
    let repo_root = root.path().join("repo");
    let workspace_root = repo_root.join(".orbit");
    std::fs::create_dir_all(&global_root).expect("global root");
    std::fs::create_dir_all(&workspace_root).expect("workspace root");
    let runtime =
        OrbitRuntime::from_roots(&global_root, &workspace_root).expect("build test runtime");
    (root, runtime, repo_root)
}

fn tasks_gc_command(
    target: GcTargetArg,
    apply: bool,
    include_rejected: bool,
    retention: Option<&str>,
) -> GcCommand {
    GcCommand {
        target,
        apply,
        json: false,
        retention: retention.map(str::to_string),
        include_rejected,
        success_retention_days: None,
        failure_retention_days: None,
        archive_after_days: None,
        purge_after_days: None,
        failure_archive_after_days: None,
        failure_purge_after_days: None,
        workspace: None,
        global: false,
    }
}

fn add_task(
    runtime: &OrbitRuntime,
    repo_root: &std::path::Path,
    title: &str,
) -> orbit_common::types::Task {
    runtime
        .add_task(TaskAddParams {
            title: title.to_string(),
            description: "Exercise CLI task GC.".to_string(),
            acceptance_criteria: vec!["archives when eligible".to_string()],
            workspace_path: Some(repo_root.to_string_lossy().into_owned()),
            ..Default::default()
        })
        .expect("add task")
}

fn set_status(runtime: &OrbitRuntime, id: &str, status: TaskStatus) {
    runtime
        .update_task(
            id,
            TaskUpdateParams {
                status: Some(status),
                ..Default::default()
            },
        )
        .expect("update status");
}

/// Walks a fresh task to `done`, satisfying the plan and execution-summary
/// lifecycle guards along the way (mirrors the core collector fixture).
fn drive_to_done(
    runtime: &OrbitRuntime,
    repo_root: &std::path::Path,
    title: &str,
) -> orbit_common::types::Task {
    let task = add_task(runtime, repo_root, title);
    set_status(runtime, &task.id, TaskStatus::Backlog);
    runtime
        .update_task(
            &task.id,
            TaskUpdateParams {
                plan: Some("1) do 2) verify".to_string()),
                status: Some(TaskStatus::InProgress),
                ..Default::default()
            },
        )
        .expect("in-progress with plan");
    runtime
        .update_task(
            &task.id,
            TaskUpdateParams {
                execution_summary: Some("did it; verified".to_string()),
                status: Some(TaskStatus::Review),
                ..Default::default()
            },
        )
        .expect("review with summary");
    set_status(runtime, &task.id, TaskStatus::Done);
    task
}
