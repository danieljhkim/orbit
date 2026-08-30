use clap::Parser;

use crate::command::auto_task::AutoTaskSubcommand;
use crate::command::{Cli, Commands};

#[test]
fn auto_task_add_accepts_required_tools_and_legacy_alias() {
    for flag in ["--required-tools", "--required-tool"] {
        let cli = Cli::try_parse_from([
            "orbit",
            "auto-task",
            "add",
            "--name",
            "required-tools",
            "--every-minutes",
            "5",
            "--title",
            "Required tools",
            flag,
            "proc.spawn,orbit.task.show",
        ])
        .expect("parse auto-task add required tools");

        let Commands::AutoTask(auto_task) = cli.command else {
            panic!("expected auto-task command");
        };
        let AutoTaskSubcommand::Add(args) = auto_task.command else {
            panic!("expected auto-task add command");
        };

        assert_eq!(args.required_tools, ["proc.spawn", "orbit.task.show"]);
    }
}

#[test]
fn auto_task_update_accepts_required_tools() {
    let cli = Cli::try_parse_from([
        "orbit",
        "auto-task",
        "update",
        "required-tools",
        "--required-tools",
        "proc.spawn,orbit.task.show",
    ])
    .expect("parse auto-task update required tools");

    let Commands::AutoTask(auto_task) = cli.command else {
        panic!("expected auto-task command");
    };
    let AutoTaskSubcommand::Update(args) = auto_task.command else {
        panic!("expected auto-task update command");
    };

    assert_eq!(
        args.required_tools.as_deref(),
        Some("proc.spawn,orbit.task.show")
    );
}
