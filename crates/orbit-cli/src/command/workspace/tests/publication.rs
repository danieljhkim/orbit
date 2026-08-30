use clap::{CommandFactory, Parser};

use crate::command::workspace::WorkspacePublicationSubcommand;
use crate::command::{Cli, Commands};

use super::super::WorkspaceSubcommand;

#[test]
fn workspace_publication_help_covers_explicit_binding_lifecycle() {
    let command = Cli::command();
    let publication = command
        .find_subcommand("workspace")
        .and_then(|workspace| workspace.find_subcommand("publication"))
        .expect("workspace publication command");
    let help = publication.clone().render_long_help().to_string();
    for verb in ["bind", "show", "rebind", "remove"] {
        assert!(help.contains(verb), "missing {verb} from help:\n{help}");
    }
    let remove = publication
        .find_subcommand("remove")
        .expect("remove command")
        .clone()
        .render_long_help()
        .to_string();
    assert!(remove.contains("--confirm"), "{remove}");
}

#[test]
fn workspace_publication_json_forms_parse_for_every_surface() {
    for verb in ["bind", "rebind"] {
        let cli = Cli::try_parse_from([
            "orbit",
            "workspace",
            "publication",
            verb,
            "--remote",
            "ssh://publication.test/example.git",
            "--publication-id",
            "pub_example",
            "--json",
        ])
        .expect("parse publication binding mutation");
        let Commands::Workspace(workspace) = cli.command else {
            panic!("expected workspace command");
        };
        let WorkspaceSubcommand::Publication(publication) = workspace.command else {
            panic!("expected publication command");
        };
        assert!(matches!(
            publication.command,
            WorkspacePublicationSubcommand::Bind(_) | WorkspacePublicationSubcommand::Rebind(_)
        ));
    }

    for args in [
        vec!["orbit", "workspace", "publication", "show", "--json"],
        vec![
            "orbit",
            "workspace",
            "publication",
            "remove",
            "--confirm",
            "--json",
        ],
    ] {
        Cli::try_parse_from(args).expect("parse publication binding read/remove");
    }
}
