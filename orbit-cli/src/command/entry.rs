use clap::{Args, Subcommand};
use orbit_core::command::entry::EntryAddParams;
use orbit_core::{AuthorType, EntityType, EntryType, OrbitError, OrbitRuntime};
use serde_json::{Value, json};

use crate::command::Execute;

#[derive(Args)]
pub struct EntryCommand {
    #[command(subcommand)]
    pub command: EntrySubcommand,
}

impl Execute for EntryCommand {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        self.command.execute(runtime)
    }
}

#[derive(Subcommand)]
pub enum EntrySubcommand {
    /// Append a new entry to an entity journal
    Add(EntryAddArgs),
    /// List entries for an entity in deterministic sequence order
    List(EntryListArgs),
}

impl Execute for EntrySubcommand {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        match self {
            EntrySubcommand::Add(args) => args.execute(runtime),
            EntrySubcommand::List(args) => args.execute(runtime),
        }
    }
}

#[derive(Args)]
pub struct EntryAddArgs {
    #[arg(long, value_enum)]
    pub entity_type: EntityType,
    #[arg(long)]
    pub entity_id: String,
    #[arg(long)]
    pub session_id: Option<String>,
    #[arg(long, value_enum)]
    pub entry_type: EntryType,
    #[arg(long, value_enum)]
    pub author_type: AuthorType,
    #[arg(long)]
    pub author_id: String,
    #[arg(long)]
    pub author_model: Option<String>,
    #[arg(long)]
    pub body: String,
    /// Output as JSON
    #[arg(long)]
    pub json: bool,
}

impl Execute for EntryAddArgs {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        let entry = runtime.add_entry(EntryAddParams {
            entity_type: self.entity_type,
            entity_id: self.entity_id,
            session_id: self.session_id,
            entry_type: self.entry_type,
            author_type: self.author_type,
            author_id: self.author_id,
            author_model: self.author_model,
            body: self.body,
        })?;

        if self.json {
            crate::output::json::print_pretty(&entry_to_json(&entry))
        } else {
            println!("{}", entry.id);
            Ok(())
        }
    }
}

#[derive(Args)]
pub struct EntryListArgs {
    #[arg(long, value_enum)]
    pub entity_type: EntityType,
    #[arg(long)]
    pub entity_id: String,
    /// Output as JSON
    #[arg(long)]
    pub json: bool,
}

impl Execute for EntryListArgs {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        let entries = runtime.list_entries(self.entity_type, &self.entity_id)?;

        if self.json {
            let rows = entries.iter().map(entry_to_json).collect::<Vec<_>>();
            crate::output::json::print_pretty(&Value::Array(rows))
        } else {
            println!(
                "{:<6} {:<10} {:<8} {:<26} BODY",
                "SEQ", "TYPE", "AUTHOR", "CREATED"
            );
            for entry in &entries {
                println!(
                    "{:<6} {:<10} {:<8} {:<26} {}",
                    entry.sequence_number,
                    entry.entry_type,
                    entry.author_type,
                    entry.created_at.to_rfc3339(),
                    entry.body
                );
            }
            Ok(())
        }
    }
}

fn entry_to_json(entry: &orbit_core::Entry) -> Value {
    json!({
        "id": entry.id,
        "entity_type": entry.entity_type.to_string(),
        "entity_id": entry.entity_id,
        "session_id": entry.session_id,
        "sequence_number": entry.sequence_number,
        "entry_type": entry.entry_type.to_string(),
        "author_type": entry.author_type.to_string(),
        "author_id": entry.author_id,
        "author_model": entry.author_model,
        "body": entry.body,
        "created_at": entry.created_at.to_rfc3339(),
    })
}
