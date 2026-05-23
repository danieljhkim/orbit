use clap::{Parser, error::ErrorKind};

use crate::command::Cli;

fn assert_cli_rejects(args: &[&str], kind: ErrorKind, expected: &str) {
    let error = match Cli::try_parse_from(args.iter().copied()) {
        Ok(_) => panic!("form should be rejected"),
        Err(error) => error,
    };
    assert_eq!(error.kind(), kind, "{error}");
    let message = error.to_string();
    assert!(message.contains(expected), "{message}");
}

#[test]
fn rejects_removed_job_run_inspection_aliases() {
    for (args, retired_subcommand) in [
        (
            &["orbit", "job", "history", "task_auto_pipeline"][..],
            "history",
        ),
        (&["orbit", "job", "run-state", "jrun-1"][..], "run-state"),
    ] {
        assert_cli_rejects(
            args,
            ErrorKind::InvalidSubcommand,
            &format!("unrecognized subcommand '{retired_subcommand}'"),
        );
    }
}

#[test]
fn parses_job_replay_subcommand() {
    assert!(Cli::try_parse_from(["orbit", "job", "replay", "jrun-1", "--json"]).is_ok());
}
