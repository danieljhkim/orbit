use clap::{Parser, error::ErrorKind};

use crate::command::Cli;

/// The trimmed `orbit task` surface (ORB-10000): 12 subcommands.
const EXPECTED_TASK_SUBCOMMANDS: [&str; 12] = [
    "add",
    "artifact",
    "list",
    "show",
    "lint",
    "update",
    "start",
    "archive",
    "review-thread",
    "export",
    "import",
    "reindex",
];

const REMOVED_TASK_SUBCOMMANDS: [&str; 7] = [
    "locks",
    "approve",
    "reject",
    "unarchive",
    "delete",
    "templates",
    "prune-context",
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
fn task_list_locked_conflicts_with_status_filters() {
    let err =
        match Cli::try_parse_from(["orbit", "task", "list", "--locked", "--status", "backlog"]) {
            Ok(_) => panic!("--locked must conflict with --status"),
            Err(err) => err,
        };
    assert_eq!(err.kind(), ErrorKind::ArgumentConflict);

    let err = match Cli::try_parse_from(["orbit", "task", "list", "--locked", "--all"]) {
        Ok(_) => panic!("--locked must conflict with --all"),
        Err(err) => err,
    };
    assert_eq!(err.kind(), ErrorKind::ArgumentConflict);

    Cli::try_parse_from(["orbit", "task", "list", "--locked", "--json"])
        .expect("--locked composes with --json");
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
