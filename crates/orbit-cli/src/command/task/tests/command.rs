use clap::{Parser, error::ErrorKind};

use crate::command::Cli;

/// The trimmed `orbit task` surface (ORB-10000): 11 subcommands. Lock
/// administration lives under the top-level `orbit locks` command (ORB-00420),
/// not here.
const EXPECTED_TASK_SUBCOMMANDS: [&str; 11] = [
    "add", "artifact", "list", "show", "lint", "update", "start", "archive", "export", "import",
    "reindex",
];

const REMOVED_TASK_SUBCOMMANDS: [&str; 8] = [
    "locks",
    "approve",
    "reject",
    "unarchive",
    "delete",
    "templates",
    "prune-context",
    "review-thread",
];

fn task_help() -> String {
    let err = match Cli::try_parse_from(["orbit", "task", "--help"]) {
        Ok(_) => panic!("task help should exit before parsing a subcommand"),
        Err(err) => err,
    };
    assert_eq!(err.kind(), ErrorKind::DisplayHelp);
    err.to_string()
}

#[test]
fn task_help_lists_exactly_the_trimmed_subcommand_set() {
    let help = task_help();
    for subcommand in EXPECTED_TASK_SUBCOMMANDS {
        assert!(
            help.lines().any(|line| {
                line.trim_start()
                    .strip_prefix(subcommand)
                    .is_some_and(|rest| rest.is_empty() || rest.starts_with(' '))
            }),
            "task help missing `{subcommand}`:\n{help}"
        );
    }
}

#[test]
fn removed_task_subcommands_are_rejected() {
    for subcommand in REMOVED_TASK_SUBCOMMANDS {
        let err = match Cli::try_parse_from(["orbit", "task", subcommand]) {
            Ok(_) => panic!("`orbit task {subcommand}` should not parse"),
            Err(err) => err,
        };
        assert!(
            matches!(
                err.kind(),
                ErrorKind::InvalidSubcommand | ErrorKind::UnknownArgument
            ),
            "`orbit task {subcommand}` should be unknown, got {:?}",
            err.kind()
        );
    }
}

#[test]
fn task_help_describes_update_status_transitions() {
    let help = task_help();
    assert!(
        help.contains("guarded status transitions"),
        "update help should mention guarded status transitions:\n{help}"
    );
    assert!(!help.contains("proposed → archived"), "{help}");
    assert!(!help.contains("review → backlog"), "{help}");
}

#[test]
fn lock_administration_lives_under_top_level_locks_command() {
    // `--locked` was removed from `task list` (ORB-00420).
    let err = match Cli::try_parse_from(["orbit", "task", "list", "--locked"]) {
        Ok(_) => panic!("`task list --locked` should no longer parse"),
        Err(err) => err,
    };
    assert_eq!(err.kind(), ErrorKind::UnknownArgument);

    // Lock administration now lives under the top-level `orbit locks` command.
    Cli::try_parse_from(["orbit", "locks", "list"]).expect("orbit locks list parses");
    Cli::try_parse_from(["orbit", "locks", "list", "--json"])
        .expect("orbit locks list --json parses");
    Cli::try_parse_from(["orbit", "locks", "release", "R-123"])
        .expect("orbit locks release <id> parses");

    let err = match Cli::try_parse_from(["orbit", "locks", "release"]) {
        Ok(_) => panic!("`locks release` requires a reservation id"),
        Err(err) => err,
    };
    assert_eq!(err.kind(), ErrorKind::MissingRequiredArgument);
}

#[test]
fn task_lint_accepts_sweep_and_fix_forms() {
    Cli::try_parse_from(["orbit", "task", "lint"]).expect("bare lint sweeps");
    Cli::try_parse_from(["orbit", "task", "lint", "--fix"]).expect("lint --fix sweeps");
    Cli::try_parse_from(["orbit", "task", "lint", "--write"]).expect("--write aliases --fix");
    Cli::try_parse_from(["orbit", "task", "lint", "ORB-00001", "--fix"]).expect("lint <id> --fix");
    Cli::try_parse_from(["orbit", "task", "lint", "--fix", "--status", "review"])
        .expect("sweep accepts --status");

    let err =
        match Cli::try_parse_from(["orbit", "task", "lint", "ORB-00001", "--status", "review"]) {
            Ok(_) => panic!("--status must be sweep-only"),
            Err(err) => err,
        };
    assert_eq!(err.kind(), ErrorKind::ArgumentConflict);
}
