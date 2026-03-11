use clap::{Args, Subcommand};
use orbit_core::command::activity::{ActivityAddParams, ActivityRunParams};
use orbit_core::{Activity, OrbitError, OrbitRuntime};
use serde_json::{Value, json};

use crate::command::Execute;

#[derive(Args)]
pub struct ActivityCommand {
    #[command(subcommand)]
    pub command: ActivitySubcommand,
}

impl Execute for ActivityCommand {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        self.command.execute(runtime)
    }
}

#[derive(Subcommand)]
pub enum ActivitySubcommand {
    Add(Box<ActivityAddArgs>),
    List(ActivityListArgs),
    Show(ActivityShowArgs),
    Run(ActivityRunArgs),
    Delete(ActivityDeleteArgs),
}

impl Execute for ActivitySubcommand {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        match self {
            ActivitySubcommand::Add(args) => (*args).execute(runtime),
            ActivitySubcommand::List(args) => args.execute(runtime),
            ActivitySubcommand::Show(args) => args.execute(runtime),
            ActivitySubcommand::Run(args) => args.execute(runtime),
            ActivitySubcommand::Delete(args) => args.execute(runtime),
        }
    }
}

#[derive(Args)]
pub struct ActivityAddArgs {
    #[arg(long)]
    pub id: String,
    #[arg(long = "type", default_value = "general")]
    pub spec_type: String,
    #[arg(long)]
    pub description: String,
    #[arg(long, default_value = "")]
    pub instruction: String,
    #[arg(long)]
    pub input_schema: Option<String>,
    #[arg(long)]
    pub output_schema: Option<String>,
    #[arg(long)]
    pub artifact_path_template: Option<String>,
    #[arg(long, default_value = "")]
    pub skill_refs: String,
    #[arg(long)]
    pub identity: Option<String>,
    #[arg(long)]
    pub assigned_to: Option<String>,
    #[arg(long)]
    pub created_by: Option<String>,
    #[arg(long)]
    pub json: bool,
}

impl Execute for ActivityAddArgs {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        let input_schema_json =
            parse_optional_json_object(self.input_schema.as_deref(), "input_schema")?;
        let output_schema_json =
            parse_optional_json_object(self.output_schema.as_deref(), "output_schema")?;
        let skill_refs = parse_csv(&self.skill_refs);

        let spec = runtime.add_activity(ActivityAddParams {
            id: self.id,
            spec_type: self.spec_type,
            description: self.description,
            instruction: self.instruction,
            input_schema_json,
            output_schema_json,
            artifact_path_template: self.artifact_path_template,
            skill_refs,
            identity_id: self.identity,
            assigned_to: self.assigned_to,
            created_by: self.created_by,
        })?;

        if self.json {
            crate::output::json::print_pretty(&activity_to_json(&spec))
        } else {
            println!("{}", spec.id);
            Ok(())
        }
    }
}

#[derive(Args)]
pub struct ActivityListArgs {
    #[arg(long)]
    pub all: bool,
    #[arg(long)]
    pub json: bool,
    /// Output signal-tier JSON (id, type, description, is_active only)
    #[arg(long)]
    pub ops: bool,
}

impl Execute for ActivityListArgs {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        let specs = runtime.list_activities(self.all)?;
        if self.ops {
            let values = specs.iter().map(activity_to_signal_json).collect::<Vec<_>>();
            crate::output::json::print_pretty(&Value::Array(values))
        } else if self.json {
            let values = specs.iter().map(activity_to_json).collect::<Vec<_>>();
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
pub struct ActivityShowArgs {
    pub id: String,
    #[arg(long)]
    pub json: bool,
}

impl Execute for ActivityShowArgs {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        let spec = runtime.show_activity(&self.id)?;
        if self.json {
            crate::output::json::print_pretty(&activity_to_json(&spec))
        } else {
            println!("ID:                  {}", spec.id);
            println!("Type:                {}", spec.spec_type);
            println!("Description:         {}", spec.description);
            if !spec.instruction.is_empty() {
                println!("Instruction:         {}", spec.instruction);
            }
            println!(
                "Artifact Template:   {}",
                spec.artifact_path_template.unwrap_or_default()
            );
            println!("Skill Refs:          {}", spec.skill_refs.join(","));
            if let Some(ref identity_id) = spec.identity_id {
                println!("Identity:            {}", identity_id);
            }
            if let Some(ref assigned_to) = spec.assigned_to {
                println!("Assigned To:         {}", assigned_to);
            }
            if let Some(ref created_by) = spec.created_by {
                println!("Created By:          {}", created_by);
            }
            println!("Active:              {}", spec.is_active);
            println!("Created:             {}", spec.created_at.to_rfc3339());
            println!("Updated:             {}", spec.updated_at.to_rfc3339());
            Ok(())
        }
    }
}

#[derive(Args)]
pub struct ActivityRunArgs {
    pub id: String,
    #[arg(long)]
    pub agent_cli: String,
    #[arg(long, default_value = "5m")]
    pub timeout: String,
    #[arg(long)]
    pub json: bool,
}

impl Execute for ActivityRunArgs {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        let result = runtime.run_activity_now(ActivityRunParams {
            activity_id: self.id,
            agent_cli: self.agent_cli,
            timeout_seconds: parse_duration_seconds(&self.timeout)?,
        })?;

        if self.json {
            crate::output::json::print_pretty(&json!({
                "activity_id": result.activity_id,
                "state": result.state.to_string(),
                "duration_ms": result.duration_ms,
                "error_code": result.error_code,
                "error_message": result.error_message,
            }))
        } else {
            let error_code = result.error_code.unwrap_or_else(|| "-".to_string());
            let error_message = result
                .error_message
                .unwrap_or_else(|| "-".to_string())
                .replace('\n', " ");
            println!(
                "activity_id={};state={};duration_ms={};error_code={};error_message={}",
                result.activity_id,
                result.state,
                result.duration_ms.unwrap_or_default(),
                error_code,
                error_message
            );
            Ok(())
        }
    }
}

#[derive(Args)]
pub struct ActivityDeleteArgs {
    pub id: String,
}

impl Execute for ActivityDeleteArgs {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        runtime.delete_activity(&self.id)?;
        println!("Deleted activity '{}'", self.id);
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
    crate::parse::csv_to_vec(raw)
}

fn parse_duration_seconds(raw: &str) -> Result<u64, OrbitError> {
    let value = raw.trim();
    if value.is_empty() {
        return Err(OrbitError::InvalidInput(
            "duration must not be empty".to_string(),
        ));
    }

    let split_at = value
        .find(|c: char| c.is_alphabetic())
        .ok_or_else(|| OrbitError::InvalidInput(format!("invalid duration: {raw}")))?;
    let (num_raw, unit_raw) = value.split_at(split_at);

    let num: u64 = num_raw
        .parse()
        .map_err(|_| OrbitError::InvalidInput(format!("invalid duration number: {raw}")))?;

    let seconds = match unit_raw {
        "s" => num,
        "m" => num.saturating_mul(60),
        "h" => num.saturating_mul(3600),
        "d" => num.saturating_mul(86400),
        "w" => num.saturating_mul(604800),
        _ => {
            return Err(OrbitError::InvalidInput(format!(
                "invalid duration unit: {unit_raw} (expected s/m/h/d/w)"
            )));
        }
    };

    Ok(seconds)
}

fn activity_to_signal_json(spec: &Activity) -> Value {
    json!({
        "id": spec.id,
        "type": spec.spec_type,
        "description": spec.description,
        "is_active": spec.is_active,
    })
}

fn activity_to_json(spec: &Activity) -> Value {
    json!({
        "id": spec.id,
        "type": spec.spec_type,
        "description": spec.description,
        "instruction": spec.instruction,
        "input_schema_json": spec.input_schema_json,
        "output_schema_json": spec.output_schema_json,
        "artifact_path_template": spec.artifact_path_template,
        "skill_refs": spec.skill_refs,
        "identity_id": spec.identity_id,
        "assigned_to": spec.assigned_to,
        "created_by": spec.created_by,
        "is_active": spec.is_active,
        "created_at": spec.created_at.to_rfc3339(),
        "updated_at": spec.updated_at.to_rfc3339(),
    })
}
