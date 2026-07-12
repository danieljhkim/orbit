use clap::{CommandFactory, Parser};

use crate::command::task::TaskSubcommand;
use crate::command::{Cli, Commands};

#[test]
fn task_add_parses_repeat_and_comma_delimited_lists() {
    let cli = Cli::parse_from([
        "orbit",
        "task",
        "add",
        "--title",
        "List parsing",
        "--acceptance-criteria",
        "first,second",
        "--acceptance-criteria",
        "third",
        "--tag",
        "cli,surface",
        "--tag",
        "test",
        "--dependencies",
        "ORB-00001,ORB-00002",
        "--dependency",
        "ORB-00003",
        "--context",
        "file:one.rs,file:two.rs",
        "--context",
        "dir:three",
    ]);

    let Commands::Task(task) = cli.command else {
        panic!("expected task command");
    };
    let TaskSubcommand::Add(args) = task.command else {
        panic!("expected task add command");
    };

    assert_eq!(args.acceptance_criteria, ["first", "second", "third"]);
    assert_eq!(args.tags, ["cli", "surface", "test"]);
    assert_eq!(args.dependencies, ["ORB-00001", "ORB-00002", "ORB-00003"]);
    assert_eq!(args.context, ["file:one.rs", "file:two.rs", "dir:three"]);
}

#[test]
fn task_add_status_only_advertises_creation_legal_values() {
    assert!(
        Cli::try_parse_from([
            "orbit",
            "task",
            "add",
            "--title",
            "Bad status",
            "--status",
            "done",
        ])
        .is_err()
    );
    let mut command = Cli::command();
    let add = command
        .find_subcommand_mut("task")
        .expect("task command")
        .find_subcommand_mut("add")
        .expect("task add command");
    let rendered = add.render_long_help().to_string();

    for legal in ["- proposed:", "- backlog:", "- someday:"] {
        assert!(rendered.contains(legal), "{rendered}");
    }
    for illegal in [
        "archived",
        "friction",
        "done",
        "review",
        "rejected",
        "blocked",
        "in-progress",
    ] {
        assert!(
            !rendered.contains(illegal),
            "{illegal} leaked into help: {rendered}"
        );
    }
}

#[test]
fn removed_task_flags_are_rejected() {
    for args in [
        vec!["task", "add", "--title", "Removed", "--agent", "codex"],
        vec!["task", "add", "--title", "Removed", "--comment", "legacy"],
        vec![
            "task",
            "add",
            "--title",
            "Removed",
            "--instructions",
            "legacy",
        ],
        vec!["task", "update", "ORB-00001", "--agent", "codex"],
        vec!["task", "start", "ORB-00001", "--agent", "codex"],
        vec![
            "task",
            "artifact",
            "put",
            "ORB-00001",
            "summary.md",
            "--agent",
            "codex",
        ],
    ] {
        let mut argv = vec!["orbit"];
        argv.extend(args);
        assert!(Cli::try_parse_from(argv).is_err());
    }
}
