use clap::{Args, Subcommand};
use orbit_core::{OrbitError, OrbitRuntime};
use serde_json::{Map, Value};

use crate::command::Execute;

#[derive(Args)]
#[command(about = "Report, list, and triage Orbit friction records")]
pub struct FrictionCommand {
    #[command(subcommand)]
    pub command: FrictionSubcommand,
}

#[derive(Subcommand)]
pub enum FrictionSubcommand {
    /// Append an Orbit friction report
    Add(FrictionAddArgs),
    /// List Orbit friction records
    List(FrictionListArgs),
    /// Show a single Orbit friction record
    Show(FrictionShowArgs),
    /// Compute friction rates
    Stats(FrictionStatsArgs),
    /// List configured friction taxonomy tags
    Tags(FrictionTagsArgs),
    /// Update triage metadata for an Orbit friction record
    Update(FrictionUpdateArgs),
    /// Mark an Orbit friction record as resolved
    Resolve(FrictionResolveArgs),
}

#[derive(Args)]
pub struct FrictionAddArgs {
    /// Markdown body describing what happened and why it caused friction
    #[arg(long)]
    pub body: String,
    /// Friction taxonomy tag; repeat or comma-separate for multiple tags
    #[arg(long = "tag", value_delimiter = ',')]
    pub tags: Vec<String>,
    /// Optional task ID being worked on when friction occurred
    #[arg(long)]
    pub during_task: Option<String>,
    /// Agent family to attribute the record to (`codex`, `claude`, `gemini`, or `grok`)
    #[arg(long)]
    pub model: String,
    /// Output as JSON
    #[arg(long)]
    pub json: bool,
}

#[derive(Args)]
pub struct FrictionListArgs {
    /// Optional model filter
    #[arg(long)]
    pub model: Option<String>,
    /// Optional status filter: open, triaged, or resolved
    #[arg(long)]
    pub status: Option<String>,
    /// Optional tag filter
    #[arg(long)]
    pub tag: Option<String>,
    /// Optional YYYY-MM month filter for reported records
    #[arg(long)]
    pub month: Option<String>,
    /// Optional case-insensitive query over id, model, tags, status, task, and body
    #[arg(long)]
    pub q: Option<String>,
    /// Optional RFC3339 lower bound for created_at
    #[arg(long)]
    pub from: Option<String>,
    /// Optional RFC3339 upper bound for created_at
    #[arg(long)]
    pub to: Option<String>,
    /// Optional maximum number of records to return
    #[arg(long)]
    pub limit: Option<usize>,
    /// Optional number of records to skip
    #[arg(long)]
    pub offset: Option<usize>,
    /// Output as JSON
    #[arg(long)]
    pub json: bool,
}

#[derive(Args)]
pub struct FrictionShowArgs {
    /// Friction record id, e.g. F2026-05-001
    pub id: String,
    /// Output as JSON
    #[arg(long)]
    pub json: bool,
}

#[derive(Args)]
pub struct FrictionStatsArgs {
    /// Output as JSON
    #[arg(long)]
    pub json: bool,
}

#[derive(Args)]
pub struct FrictionTagsArgs {
    /// Output as JSON
    #[arg(long)]
    pub json: bool,
}

#[derive(Args)]
pub struct FrictionUpdateArgs {
    /// Friction record id, e.g. F2026-05-001
    pub id: String,
    /// Optional status: open, triaged, or resolved
    #[arg(long)]
    pub status: Option<String>,
    /// Optional replacement taxonomy tag; repeat or comma-separate for multiple tags
    #[arg(long = "tag", value_delimiter = ',')]
    pub tags: Vec<String>,
    /// Optional replacement markdown body
    #[arg(long)]
    pub body: Option<String>,
    /// Output as JSON
    #[arg(long)]
    pub json: bool,
}

#[derive(Args)]
pub struct FrictionResolveArgs {
    /// Friction record id, e.g. F2026-05-001
    pub id: String,
    /// Output as JSON
    #[arg(long)]
    pub json: bool,
}

impl Execute for FrictionCommand {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        self.command.execute(runtime)
    }
}

impl Execute for FrictionSubcommand {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        match self {
            Self::Add(args) => args.execute(runtime),
            Self::List(args) => args.execute(runtime),
            Self::Show(args) => args.execute(runtime),
            Self::Stats(args) => args.execute(runtime),
            Self::Tags(args) => args.execute(runtime),
            Self::Update(args) => args.execute(runtime),
            Self::Resolve(args) => args.execute(runtime),
        }
    }
}

impl Execute for FrictionAddArgs {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        let mut input = Map::new();
        input.insert("body".to_string(), Value::String(self.body));
        insert_string_list(&mut input, "tags", self.tags);
        insert_optional_string(&mut input, "during_task", self.during_task);
        input.insert("model".to_string(), Value::String(self.model));
        let value = runtime.run_tool("orbit.friction.add", Value::Object(input))?;
        print_record_or_json(&value, self.json)
    }
}

impl Execute for FrictionListArgs {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        let mut input = Map::new();
        insert_optional_string(&mut input, "model", self.model);
        insert_optional_string(&mut input, "status", self.status);
        insert_optional_string(&mut input, "tag", self.tag);
        insert_optional_string(&mut input, "month", self.month);
        insert_optional_string(&mut input, "q", self.q);
        insert_optional_string(&mut input, "from", self.from);
        insert_optional_string(&mut input, "to", self.to);
        insert_optional_usize(&mut input, "limit", self.limit);
        insert_optional_usize(&mut input, "offset", self.offset);
        let value = runtime.run_tool("orbit.friction.list", Value::Object(input))?;
        if self.json {
            crate::output::json::print_pretty(&value)
        } else {
            print_records_table(&value)
        }
    }
}

impl Execute for FrictionShowArgs {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        let value = runtime.run_tool("orbit.friction.show", id_input(self.id))?;
        print_record_or_json(&value, self.json)
    }
}

impl Execute for FrictionStatsArgs {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        let value = runtime.run_tool("orbit.friction.stats", Value::Object(Map::new()))?;
        crate::output::json::print_pretty(&value)
    }
}

impl Execute for FrictionTagsArgs {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        let value = runtime.run_tool("orbit.friction.tags", Value::Object(Map::new()))?;
        if self.json {
            crate::output::json::print_pretty(&value)
        } else {
            print_tags(&value)
        }
    }
}

impl Execute for FrictionUpdateArgs {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        let mut input = Map::new();
        input.insert("id".to_string(), Value::String(self.id));
        insert_optional_string(&mut input, "status", self.status);
        insert_string_list(&mut input, "tags", self.tags);
        insert_optional_string(&mut input, "body", self.body);
        let value = runtime.run_tool("orbit.friction.update", Value::Object(input))?;
        print_record_or_json(&value, self.json)
    }
}

impl Execute for FrictionResolveArgs {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        let value = runtime.run_tool("orbit.friction.resolve", id_input(self.id))?;
        print_record_or_json(&value, self.json)
    }
}

fn id_input(id: String) -> Value {
    Value::Object(Map::from_iter([("id".to_string(), Value::String(id))]))
}

fn insert_optional_string(input: &mut Map<String, Value>, key: &str, value: Option<String>) {
    if let Some(value) = value
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
    {
        input.insert(key.to_string(), Value::String(value));
    }
}

fn insert_optional_usize(input: &mut Map<String, Value>, key: &str, value: Option<usize>) {
    if let Some(value) = value {
        input.insert(key.to_string(), Value::from(value));
    }
}

fn insert_string_list(input: &mut Map<String, Value>, key: &str, values: Vec<String>) {
    let values = values
        .into_iter()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .map(Value::String)
        .collect::<Vec<_>>();
    if !values.is_empty() {
        input.insert(key.to_string(), Value::Array(values));
    }
}

fn print_records_table(value: &Value) -> Result<(), OrbitError> {
    let Some(records) = value.as_array() else {
        return crate::output::json::print_pretty(value);
    };

    let mut table =
        crate::output::table::build_table(&["ID", "STATUS", "MODEL", "TAGS", "TASK", "TITLE"]);
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
    println!("{table}");
    Ok(())
}

fn print_record_or_json(value: &Value, json: bool) -> Result<(), OrbitError> {
    if json {
        return crate::output::json::print_pretty(value);
    }

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
