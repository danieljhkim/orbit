use clap::{Parser, error::ErrorKind};

use crate::command::Cli;

#[test]
fn task_help_describes_reject_transition_to_rejected() {
    let err = match Cli::try_parse_from(["orbit", "task", "--help"]) {
        Ok(_) => panic!("task help should exit before parsing a subcommand"),
        Err(err) => err,
    };

    assert_eq!(err.kind(), ErrorKind::DisplayHelp);

    let help = err.to_string();
    assert!(
        help.contains("Reject a task (proposed/friction/review/backlog/in-progress -> rejected)"),
        "{help}"
    );
    assert!(!help.contains("proposed → archived"), "{help}");
    assert!(!help.contains("review → backlog"), "{help}");
}
