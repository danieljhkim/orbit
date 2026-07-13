use clap::{CommandFactory, Parser};

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
