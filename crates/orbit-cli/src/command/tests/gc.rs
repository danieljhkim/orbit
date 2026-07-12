use clap::{CommandFactory, Parser};

use super::super::{Cli, Commands, gc::GcTargetArg};

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
        "--workspace",
        "--global",
    ] {
        assert!(help.contains(value), "missing `{value}` from help:\n{help}");
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

#[test]
fn gc_scope_flags_conflict() {
    let error =
        match Cli::try_parse_from(["orbit", "gc", "tasks", "--workspace", "here", "--global"]) {
            Ok(_) => panic!("scope flags must conflict"),
            Err(error) => error,
        };
    assert!(error.to_string().contains("cannot be used with"));
}
