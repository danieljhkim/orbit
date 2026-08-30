use clap::Parser;

use crate::command::{Cli, Commands};

use super::super::TaskSubcommand;

#[test]
fn task_update_accepts_context_files_alias() {
    let cli = Cli::try_parse_from([
        "orbit",
        "task",
        "update",
        "ORB-00001",
        "--context-files",
        "file:src/lib.rs,dir:tests",
        "--json",
    ])
    .expect("parse task update with context-files");

    let Commands::Task(task) = cli.command else {
        panic!("expected task command");
    };
    let TaskSubcommand::Update(args) = task.command else {
        panic!("expected task update command");
    };

    assert_eq!(args.id, "ORB-00001");
    assert_eq!(
        args.context_files.as_deref(),
        Some("file:src/lib.rs,dir:tests")
    );
    assert!(args.json);
}

#[test]
fn task_update_acceptance_criteria_does_not_split_on_commas() {
    let cli = Cli::try_parse_from([
        "orbit",
        "task",
        "update",
        "ORB-00001",
        "--acceptance-criteria",
        "given X, then Y",
        "--acceptance-criteria",
        "given A, then B",
    ])
    .expect("parse task update acceptance criteria");

    let Commands::Task(task) = cli.command else {
        panic!("expected task command");
    };
    let TaskSubcommand::Update(args) = task.command else {
        panic!("expected task update command");
    };

    assert_eq!(
        args.acceptance_criteria,
        ["given X, then Y", "given A, then B"]
    );
}

#[test]
fn task_update_complexity_uses_add_spellings() {
    for complexity in ["low", "medium", "hard"] {
        let cli = Cli::try_parse_from([
            "orbit",
            "task",
            "update",
            "ORB-00001",
            "--complexity",
            complexity,
        ])
        .expect("parse task update complexity");

        let Commands::Task(task) = cli.command else {
            panic!("expected task command");
        };
        let TaskSubcommand::Update(args) = task.command else {
            panic!("expected task update command");
        };

        assert_eq!(args.complexity.expect("complexity").to_string(), complexity);
    }
}

#[test]
fn task_update_rejects_required_tools() {
    let result = Cli::try_parse_from([
        "orbit",
        "task",
        "update",
        "ORB-00001",
        "--required-tools",
        "proc.spawn,orbit.task.show",
    ]);
    let Err(error) = result else {
        panic!("required tools are creation-only");
    };

    assert!(error.to_string().contains("--required-tools"), "{error}");
}
