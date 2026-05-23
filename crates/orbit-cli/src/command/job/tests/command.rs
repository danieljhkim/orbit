use clap::Parser;

use crate::command::Cli;

#[test]
fn rejects_removed_job_run_inspection_aliases() {
    assert!(Cli::try_parse_from(["orbit", "job", "history", "task_auto_pipeline"]).is_err());
    assert!(Cli::try_parse_from(["orbit", "job", "run-state", "jrun-1"]).is_err());
}

#[test]
fn parses_job_replay_subcommand() {
    assert!(Cli::try_parse_from(["orbit", "job", "replay", "jrun-1", "--json"]).is_ok());
}
