use clap::{CommandFactory, Parser};

use super::super::{Cli, Commands};
use crate::command::gc::GcTarget;

#[test]
fn gc_worktrees_defaults_to_dry_run_and_accepts_scopes() {
    let cli = Cli::parse_from([
        "orbit",
        "gc",
        "worktrees",
        "--run",
        "jrun-1",
        "--older-than-hours",
        "24",
    ]);
    let Commands::Gc(command) = cli.command else {
        panic!("expected gc");
    };
    let GcTarget::Worktrees(args) = command.target;
    assert!(!args.yes);
    assert_eq!(args.run.as_deref(), Some("jrun-1"));
    assert_eq!(args.older_than_hours, Some(24));
}

#[test]
fn gc_help_exposes_positional_worktree_class() {
    let help = Cli::command().render_long_help().to_string();
    assert!(help.contains("gc          Inspect and explicitly reap"));

    let error = match Cli::try_parse_from(["orbit", "gc"]) {
        Ok(_) => panic!("target is required"),
        Err(error) => error,
    };
    assert!(error.to_string().contains("<COMMAND>"));
}

#[test]
fn gc_rejects_yes_with_explicit_dry_run() {
    let error = match Cli::try_parse_from(["orbit", "gc", "worktrees", "--yes", "--dry-run"]) {
        Ok(_) => panic!("destructive and dry-run flags conflict"),
        Err(error) => error,
    };
    assert!(error.to_string().contains("cannot be used with"));
}
