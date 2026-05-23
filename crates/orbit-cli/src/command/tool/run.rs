use clap::{Args, ValueEnum};
use orbit_core::{OrbitError, OrbitRuntime};
use serde_json::{Map, Value};

use crate::command::Execute;

#[derive(Clone, ValueEnum, Default)]
pub enum OutputFormat {
    #[default]
    Json,
    Text,
}

#[derive(Args)]
pub struct ToolRunArgs {
    /// Tool name
    pub name: String,
    /// JSON input for the tool (use --input-file to avoid shell escaping issues with rich content)
    #[arg(long)]
    pub input: Option<String>,
    /// Path to a JSON file to use as input (bypasses shell escaping; preferred for markdown or multi-line content)
    #[arg(long, conflicts_with = "input")]
    pub input_file: Option<String>,
    /// Deprecated explicit agent family for provenance attribution (prefer --model)
    #[arg(long)]
    pub agent: Option<String>,
    /// Exact agent model for provenance attribution (overrides ORBIT_AGENT_MODEL)
    #[arg(long)]
    pub model: Option<String>,
    /// Execution timeout (e.g. "30s", "5000ms")
    #[arg(long)]
    pub timeout: Option<String>,
    /// Validate without executing
    #[arg(long)]
    pub dry_run: bool,
    /// Comma-separated top-level fields to keep from object output
    #[arg(long, value_delimiter = ',', conflicts_with = "full")]
    pub fields: Vec<String>,
    /// Return the tool's full unfiltered JSON output
    #[arg(long)]
    pub full: bool,
    /// Pretty-print JSON output for human debugging
    #[arg(long)]
    pub pretty: bool,
    /// Output format
    #[arg(long, value_enum, default_value_t = OutputFormat::Json)]
    pub output: OutputFormat,
}

impl Execute for ToolRunArgs {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        let input: Value = if let Some(path) = &self.input_file {
            let raw = std::fs::read_to_string(path).map_err(|e| {
                OrbitError::InvalidInput(format!("cannot read input file '{path}': {e}"))
            })?;
            serde_json::from_str(&raw)
                .map_err(|e| OrbitError::InvalidInput(format!("invalid JSON in '{path}': {e}")))?
        } else {
            match &self.input {
                Some(raw) => serde_json::from_str(raw)
                    .map_err(|e| OrbitError::InvalidInput(format!("invalid JSON input: {e}")))?,
                None => Value::Object(Default::default()),
            }
        };

        if self.dry_run {
            let result = runtime.run_tool_dry_run(&self.name, &input)?;
            println!("Tool:           {}", result.tool_name);
            println!(
                "Policy:         {}",
                if result.policy_allowed {
                    "allowed"
                } else {
                    "denied"
                }
            );
            if result.missing_params.is_empty() {
                println!("Missing params: (none)");
            } else {
                println!("Missing params: {}", result.missing_params.join(", "));
            }
            return Ok(());
        }

        let output =
            runtime.execute_tool_command(&self.name, input.clone(), self.agent, self.model)?;
        let output = shape_tool_output(&self.name, &input, output, self.full, &self.fields);

        match self.output {
            OutputFormat::Json => {
                if self.pretty {
                    crate::output::json::print_pretty(&output)
                } else {
                    crate::output::json::print(&output)
                }
            }
            OutputFormat::Text => {
                println!("{}", output);
                Ok(())
            }
        }
    }
}

const MINIMAL_TASK_FIELDS: &[&str] = &[
    "id",
    "title",
    "status",
    "priority",
    "type",
    "dependencies",
    "resolved_dependencies",
    "implemented_by",
    "created_at",
    "updated_at",
];

pub(super) fn shape_tool_output(
    tool_name: &str,
    input: &Value,
    output: Value,
    full: bool,
    fields: &[String],
) -> Value {
    if full {
        return output;
    }

    if !fields.is_empty() {
        return filter_top_level_fields(output, fields);
    }

    if should_project_minimal_task_output(tool_name, input) {
        return filter_top_level_fields(
            output,
            &MINIMAL_TASK_FIELDS
                .iter()
                .map(|field| (*field).to_string())
                .collect::<Vec<_>>(),
        );
    }

    output
}

fn should_project_minimal_task_output(tool_name: &str, input: &Value) -> bool {
    if !matches!(
        tool_name,
        "orbit.task.list"
            | "orbit.task.show"
            | "orbit.task.add"
            | "orbit.task.artifact.put"
            | "orbit.task.update"
    ) {
        return false;
    }

    if tool_name == "orbit.task.show"
        && (input.get("field").is_some() || input.get("fields").is_some())
    {
        return false;
    }

    true
}

fn filter_top_level_fields(value: Value, fields: &[String]) -> Value {
    match value {
        Value::Object(map) => Value::Object(select_fields(map, fields)),
        Value::Array(items) => Value::Array(
            items
                .into_iter()
                .map(|item| match item {
                    Value::Object(map) => Value::Object(select_fields(map, fields)),
                    other => other,
                })
                .collect(),
        ),
        other => other,
    }
}

fn select_fields(map: Map<String, Value>, fields: &[String]) -> Map<String, Value> {
    let mut selected = Map::new();
    for field in fields {
        if let Some(value) = map.get(field) {
            selected.insert(field.clone(), value.clone());
        }
    }
    selected
}
