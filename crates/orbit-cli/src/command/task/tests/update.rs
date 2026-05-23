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
