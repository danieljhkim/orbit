use clap::{Args, Subcommand};
use orbit_core::command::work::WorkAddParams;
use orbit_core::{OrbitError, OrbitRuntime, Work};
use serde_json::{Value, json};

use crate::command::Execute;

#[derive(Args)]
pub struct WorkCommand {
    #[command(subcommand)]
    pub command: WorkSubcommand,
}

impl Execute for WorkCommand {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        self.command.execute(runtime)
    }
}

#[derive(Subcommand)]
pub enum WorkSubcommand {
    Add(WorkAddArgs),
    List(WorkListArgs),
    Show(WorkShowArgs),
    Delete(WorkDeleteArgs),
}

impl Execute for WorkSubcommand {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        match self {
            WorkSubcommand::Add(args) => args.execute(runtime),
            WorkSubcommand::List(args) => args.execute(runtime),
            WorkSubcommand::Show(args) => args.execute(runtime),
            WorkSubcommand::Delete(args) => args.execute(runtime),
        }
    }
}

#[derive(Args)]
pub struct WorkAddArgs {
    #[arg(long)]
    pub id: String,
    #[arg(long = "type", default_value = "general")]
    pub spec_type: String,
    #[arg(long)]
    pub description: String,
    #[arg(long)]
    pub input_schema: Option<String>,
    #[arg(long)]
    pub output_schema: Option<String>,
    #[arg(long)]
    pub artifact_path_template: Option<String>,
    #[arg(long, default_value = "")]
    pub skill_refs: String,
    #[arg(long)]
    pub json: bool,
}

impl Execute for WorkAddArgs {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        let input_schema_json =
            parse_optional_json_object(self.input_schema.as_deref(), "input_schema")?;
        let output_schema_json =
            parse_optional_json_object(self.output_schema.as_deref(), "output_schema")?;
        let skill_refs = parse_csv(&self.skill_refs);

        let spec = runtime.add_work(WorkAddParams {
            id: self.id,
            spec_type: self.spec_type,
            description: self.description,
            input_schema_json,
            output_schema_json,
            artifact_path_template: self.artifact_path_template,
            skill_refs,
        })?;

        if self.json {
            crate::output::json::print_pretty(&work_to_json(&spec))
        } else {
            println!("{}", spec.id);
            Ok(())
        }
    }
}

#[derive(Args)]
pub struct WorkListArgs {
    #[arg(long)]
    pub all: bool,
    #[arg(long)]
    pub json: bool,
}

impl Execute for WorkListArgs {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        let specs = runtime.list_works(self.all)?;
        if self.json {
            let values = specs.iter().map(work_to_json).collect::<Vec<_>>();
            crate::output::json::print_pretty(&Value::Array(values))
        } else {
            println!("{:<24} {:<14} {:<8} DESCRIPTION", "ID", "TYPE", "ACTIVE");
            for spec in &specs {
                println!(
                    "{:<24} {:<14} {:<8} {}",
                    spec.id, spec.spec_type, spec.is_active, spec.description
                );
            }
            Ok(())
        }
    }
}

#[derive(Args)]
pub struct WorkShowArgs {
    pub id: String,
    #[arg(long)]
    pub json: bool,
}

impl Execute for WorkShowArgs {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        let spec = runtime.show_work(&self.id)?;
        if self.json {
            crate::output::json::print_pretty(&work_to_json(&spec))
        } else {
            println!("ID:                  {}", spec.id);
            println!("Type:                {}", spec.spec_type);
            println!("Description:         {}", spec.description);
            println!(
                "Artifact Template:   {}",
                spec.artifact_path_template.unwrap_or_default()
            );
            println!("Skill Refs:          {}", spec.skill_refs.join(","));
            println!("Active:              {}", spec.is_active);
            println!("Created:             {}", spec.created_at.to_rfc3339());
            println!("Updated:             {}", spec.updated_at.to_rfc3339());
            Ok(())
        }
    }
}

#[derive(Args)]
pub struct WorkDeleteArgs {
    pub id: String,
}

impl Execute for WorkDeleteArgs {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        runtime.delete_work(&self.id)?;
        println!("Deleted work '{}'", self.id);
        Ok(())
    }
}

fn parse_json_object(raw: &str, field: &str) -> Result<Value, OrbitError> {
    let value = serde_json::from_str::<Value>(raw)
        .map_err(|e| OrbitError::InvalidInput(format!("{field} must be valid JSON: {e}")))?;
    if !value.is_object() {
        return Err(OrbitError::InvalidInput(format!(
            "{field} must be a JSON object"
        )));
    }
    Ok(value)
}

fn parse_optional_json_object(raw: Option<&str>, field: &str) -> Result<Value, OrbitError> {
    match raw {
        None => Ok(json!({})),
        Some(value) if value.trim().is_empty() => Ok(json!({})),
        Some(value) => parse_json_object(value, field),
    }
}

fn parse_csv(raw: &str) -> Vec<String> {
    raw.split(',')
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(ToString::to_string)
        .collect()
}

fn work_to_json(spec: &Work) -> Value {
    json!({
        "id": spec.id,
        "type": spec.spec_type,
        "description": spec.description,
        "input_schema_json": spec.input_schema_json,
        "output_schema_json": spec.output_schema_json,
        "artifact_path_template": spec.artifact_path_template,
        "skill_refs": spec.skill_refs,
        "is_active": spec.is_active,
        "created_at": spec.created_at.to_rfc3339(),
        "updated_at": spec.updated_at.to_rfc3339(),
    })
}
