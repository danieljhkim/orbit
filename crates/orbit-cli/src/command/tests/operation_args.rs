//! The clap adapter is noun-agnostic [ORB-10358].
//!
//! ADR-0209 bearing 1 claims that adding a verb costs a registry entry and a
//! handler, and that migrating the *next* noun costs a registry — not new
//! surface code. This module is the executable proof of the CLI half: it
//! declares a synthetic noun with a synthetic verb, feeds it to the same
//! adapter `orbit friction` uses, and checks that a complete, correct command
//! line falls out. Not one line below is friction-specific, and not one line of
//! `operation_args.rs` had to change to accept a new noun.

use clap::Command;
use orbit_common::operation::{
    CliArgKind, CliBinding, CliRender, Description, McpExposure, OperationSpec, ParamSpec,
    ParamType,
};
use orbit_common::types::McpToolPlacement;
use serde_json::json;

use super::super::operation_args::{augment_subcommands, invocation_from_matches};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WidgetVerb {
    Poke,
    List,
}

/// A hypothetical new verb: the *only* thing an author writes.
const POKE: OperationSpec<WidgetVerb> = OperationSpec {
    verb: WidgetVerb::Poke,
    name: "poke",
    tool_name: "orbit.widget.poke",
    tool_description: "Poke a widget",
    cli_about: "Poke a widget",
    params: &[
        ParamSpec {
            name: "id",
            param_type: ParamType::String,
            required: true,
            mcp_description: Some(Description::Static("widget ID")),
            cli: Some(CliBinding {
                kind: CliArgKind::Positional,
                help: Description::Static("Widget id"),
            }),
        },
        ParamSpec {
            name: "labels",
            param_type: ParamType::StringList,
            required: false,
            mcp_description: Some(Description::Static("Labels")),
            cli: Some(CliBinding {
                kind: CliArgKind::Flag {
                    long: "label",
                    delimiter: Some(','),
                },
                help: Description::Static("Widget label; repeat or comma-separate"),
            }),
        },
        ParamSpec {
            name: "force",
            param_type: ParamType::String,
            required: false,
            mcp_description: None,
            cli: Some(CliBinding {
                kind: CliArgKind::Flag {
                    long: "force",
                    delimiter: None,
                },
                help: Description::Static("Force the poke"),
            }),
        },
    ],
    rejects_agent_field: false,
    mcp: McpExposure::AgentOperator(McpToolPlacement::Owner),
    cli_json_flag: true,
    cli_render: CliRender::Record,
};

const LIST: OperationSpec<WidgetVerb> = OperationSpec {
    verb: WidgetVerb::List,
    name: "list",
    tool_name: "orbit.widget.list",
    tool_description: "List widgets",
    cli_about: "List widgets",
    params: &[ParamSpec {
        name: "limit",
        param_type: ParamType::Integer,
        required: false,
        mcp_description: Some(Description::Static("Max widgets")),
        cli: Some(CliBinding {
            kind: CliArgKind::Flag {
                long: "limit",
                delimiter: None,
            },
            help: Description::Static("Optional maximum number of widgets"),
        }),
    }],
    rejects_agent_field: false,
    mcp: McpExposure::Inactive,
    cli_json_flag: false,
    cli_render: CliRender::RecordTable,
};

const WIDGET_OPERATIONS: &[OperationSpec<WidgetVerb>] = &[POKE, LIST];

fn widget_command() -> Command {
    augment_subcommands(
        Command::new("widget").subcommand_required(true),
        WIDGET_OPERATIONS,
    )
}

fn parse(args: &[&str]) -> super::super::operation_args::Invocation<WidgetVerb> {
    let matches = widget_command()
        .try_get_matches_from(args)
        .expect("argv parses");
    invocation_from_matches(WIDGET_OPERATIONS, "widget", &matches).expect("invocation resolves")
}

#[test]
fn a_registry_entry_alone_produces_a_working_subcommand() {
    let parsed = parse(&["widget", "poke", "W-1", "--label", "a,b", "--json"]);

    assert_eq!(parsed.spec.verb, WidgetVerb::Poke);
    assert_eq!(parsed.spec.tool_name, "orbit.widget.poke");
    assert_eq!(parsed.input, json!({ "id": "W-1", "labels": ["a", "b"] }));
    assert!(parsed.json);
    assert_eq!(parsed.target_id(), Some("W-1"));
}

#[test]
fn help_text_is_assembled_from_the_registry_entry() {
    let help = widget_command()
        .try_get_matches_from(["widget", "poke", "--help"])
        .expect_err("--help exits before parsing")
        .to_string();

    assert!(help.contains("Poke a widget"), "{help}");
    assert!(help.contains("<ID>"), "{help}");
    assert!(help.contains("Widget id"), "{help}");
    assert!(help.contains("--label <LABELS>"), "{help}");
    assert!(
        help.contains("Widget label; repeat or comma-separate"),
        "{help}"
    );
    assert!(help.contains("--force <FORCE>"), "{help}");
    assert!(help.contains("--json"), "{help}");
}

#[test]
fn required_parameters_are_enforced_from_the_registry_entry() {
    assert!(
        widget_command()
            .try_get_matches_from(["widget", "poke"])
            .is_err(),
        "the spec marks `id` required"
    );
}

#[test]
fn verbs_that_opt_out_of_json_get_no_json_flag() {
    assert!(
        widget_command()
            .try_get_matches_from(["widget", "list", "--json"])
            .is_err(),
        "`list` sets cli_json_flag: false"
    );
    assert!(!parse(&["widget", "list"]).json);
}

#[test]
fn integer_parameters_parse_and_reject_from_the_registry_entry() {
    assert_eq!(
        parse(&["widget", "list", "--limit", "7"]).input,
        json!({ "limit": 7 })
    );
    assert!(
        widget_command()
            .try_get_matches_from(["widget", "list", "--limit", "many"])
            .is_err()
    );
}

#[test]
fn unknown_verbs_are_rejected_against_the_registry() {
    assert!(
        widget_command()
            .try_get_matches_from(["widget", "prod"])
            .is_err()
    );
}
