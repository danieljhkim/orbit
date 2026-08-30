use clap::{CommandFactory, Parser};

use crate::command::task::TaskPublicationSubcommand;
use crate::command::{Cli, Commands};

use super::super::TaskSubcommand;

#[test]
fn task_publication_help_covers_the_complete_operator_lifecycle() {
    let command = Cli::command();
    let publication = command
        .find_subcommand("task")
        .and_then(|task| task.find_subcommand("publication"))
        .expect("task publication command");
    let help = publication.clone().render_long_help().to_string();
    for verb in ["publish", "status", "inspect", "restore"] {
        assert!(help.contains(verb), "missing {verb} from help:\n{help}");
    }

    let publish = publication
        .find_subcommand("publish")
        .expect("publish command")
        .clone()
        .render_long_help()
        .to_string();
    assert!(publish.contains("safe default"), "{publish}");
    assert!(publish.contains("allow-unscanned-attachments"), "{publish}");

    let restore = publication
        .find_subcommand("restore")
        .expect("restore command")
        .clone()
        .render_long_help()
        .to_string();
    assert!(restore.contains("--confirm"), "{restore}");
    assert!(restore.contains("--allow-identical-retry"), "{restore}");
}

#[test]
fn task_publication_json_forms_parse_for_every_surface() {
    for verb in ["publish", "status"] {
        let cli = Cli::try_parse_from(["orbit", "task", "publication", verb, "--json"])
            .expect("parse owner publication command");
        let Commands::Task(task) = cli.command else {
            panic!("expected task command");
        };
        let TaskSubcommand::Publication(_) = task.command else {
            panic!("expected task publication command");
        };
    }

    let pairing = [
        "--workspace-id",
        "ws_example",
        "--source-remote",
        "ssh://source.test/example.git",
        "--publication-id",
        "pub_example",
        "--authority-machine-id",
        "hm_example",
        "--remote",
        "ssh://publication.test/example.git",
    ];
    for verb in ["inspect", "restore"] {
        let mut args = vec!["orbit", "task", "publication", verb];
        args.extend(pairing);
        if verb == "restore" {
            args.push("--confirm");
        }
        args.push("--json");
        let cli = Cli::try_parse_from(args).expect("parse consumer publication command");
        let Commands::Task(task) = cli.command else {
            panic!("expected task command");
        };
        let TaskSubcommand::Publication(publication) = task.command else {
            panic!("expected task publication command");
        };
        assert!(matches!(
            publication.command,
            TaskPublicationSubcommand::Inspect(_) | TaskPublicationSubcommand::Restore(_)
        ));
    }
}
