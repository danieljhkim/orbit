//! The friction CLI, derived from the friction operation registry.
//!
//! ADR-0209 bearing 1 pilot [ORB-10358]. Each friction verb used to be a clap
//! `Args` struct plus an `Execute` impl that hand-built the tool input — the
//! same names, help strings, and field mapping the MCP tool file and the
//! dashboard handler each restated. All of that is now declared once in
//! `orbit_common::friction::operations`, and the clap and input derivation is the
//! noun-agnostic adapter in [`super::operation_args`].
//!
//! Adding a friction verb requires no edit here. What is left in this file is
//! genuinely friction-specific: how to render a friction response as text.

use clap::{ArgMatches, Args, Command, FromArgMatches, Subcommand};
use orbit_common::friction::{FRICTION_OPERATIONS, FrictionVerb, friction_operation};
use orbit_common::operation::CliRender;
use orbit_core::{OrbitError, OrbitRuntime};
use serde_json::Value;

use super::operation_args::{Invocation, augment_subcommands, invocation_from_matches};
use crate::command::Execute;

/// One parsed friction verb invocation.
pub type FrictionInvocation = Invocation<FrictionVerb>;

#[derive(Args)]
#[command(about = "Report, list, and triage Orbit friction records")]
pub struct FrictionCommand {
    #[command(subcommand)]
    pub command: FrictionInvocation,
}

impl Subcommand for FrictionInvocation {
    fn augment_subcommands(cmd: Command) -> Command {
        augment_subcommands(cmd, FRICTION_OPERATIONS)
    }

    fn augment_subcommands_for_update(cmd: Command) -> Command {
        <Self as Subcommand>::augment_subcommands(cmd)
    }

    fn has_subcommand(name: &str) -> bool {
        friction_operation(name).is_some()
    }
}

impl FromArgMatches for FrictionInvocation {
    fn from_arg_matches(matches: &ArgMatches) -> Result<Self, clap::Error> {
        invocation_from_matches(FRICTION_OPERATIONS, "friction", matches)
    }

    fn update_from_arg_matches(&mut self, matches: &ArgMatches) -> Result<(), clap::Error> {
        *self = Self::from_arg_matches(matches)?;
        Ok(())
    }
}

impl Execute for FrictionCommand {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        let FrictionInvocation { spec, input, json } = self.command;
        let value = runtime.run_tool(spec.tool_name, input)?;
        render(&value, spec.cli_render, json)
    }
}

/// Render a friction response, honoring the spec's declared default rendering.
fn render(value: &Value, kind: CliRender, json: bool) -> Result<(), OrbitError> {
    match kind {
        CliRender::Record if !json => print_record(value),
        CliRender::RecordTable if !json => print_records_table(value),
        CliRender::TagList if !json => print_tags(value),
        // `AlwaysJson` responses have no useful flat rendering, and `--json`
        // asks for this branch explicitly.
        _ => crate::output::json::print_pretty(value),
    }
}

fn print_records_table(value: &Value) -> Result<(), OrbitError> {
    let Some(records) = value.as_array() else {
        return crate::output::json::print_pretty(value);
    };

    use crate::output::table::{Column, Table};
    // `orbit friction show <id>` prints a record in full.
    let mut table = Table::new(vec![
        Column::new("ID").fixed(),
        Column::new("STATUS").fixed(),
        Column::new("MODEL").fixed(),
        Column::new("TAGS"),
        Column::new("TASK").fixed(),
        Column::new("TITLE"),
    ])
    .empty_message("no friction records matching the given filters");
    for record in records {
        table.add_row(vec![
            value_string(record, "id"),
            value_string(record, "status"),
            value_string(record, "model"),
            value_string_list(record, "tags"),
            value_string(record, "during_task"),
            value_string(record, "title"),
        ]);
    }
    table.print();
    Ok(())
}

fn print_record(value: &Value) -> Result<(), OrbitError> {
    if !value.is_object() {
        return crate::output::json::print_pretty(value);
    }

    println!("ID: {}", value_string(value, "id"));
    println!("Status: {}", value_string(value, "status"));
    println!("Model: {}", value_string(value, "model"));
    let tags = value_string_list(value, "tags");
    if !tags.is_empty() {
        println!("Tags: {tags}");
    }
    let task = value_string(value, "during_task");
    if !task.is_empty() {
        println!("Task: {task}");
    }
    let path = value_string(value, "path");
    if !path.is_empty() {
        println!("Path: {path}");
    }
    let body = value_string(value, "body");
    if !body.is_empty() {
        println!("\n{body}");
    }
    Ok(())
}

fn print_tags(value: &Value) -> Result<(), OrbitError> {
    let Some(tags) = value.as_array() else {
        return crate::output::json::print_pretty(value);
    };
    for tag in tags {
        match tag {
            Value::String(name) => println!("{name}"),
            Value::Object(object) => {
                let name = object
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                let description = object
                    .get("description")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                if description.is_empty() {
                    println!("{name}");
                } else {
                    println!("{name}\t{description}");
                }
            }
            _ => println!("{tag}"),
        }
    }
    Ok(())
}

fn value_string(value: &Value, key: &str) -> String {
    value
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string()
}

fn value_string_list(value: &Value, key: &str) -> String {
    value
        .get(key)
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .collect::<Vec<_>>()
                .join(", ")
        })
        .unwrap_or_default()
}
