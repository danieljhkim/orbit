use clap::{Args, Subcommand};
use orbit_core::command::job::JobAddParams;
use orbit_core::{Job, OrbitError, OrbitRuntime};
use serde_json::{Value, json};

use crate::command::Execute;

#[derive(Args)]
pub struct JobCommand {
    #[command(subcommand)]
    pub command: JobSubcommand,
}

impl Execute for JobCommand {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        self.command.execute(runtime)
    }
}

#[derive(Subcommand)]
pub enum JobSubcommand {
    Add(Box<JobAddArgs>),
    List(JobListArgs),
    Show(JobShowArgs),
    Delete(JobDeleteArgs),
}

impl Execute for JobSubcommand {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        match self {
            JobSubcommand::Add(args) => (*args).execute(runtime),
            JobSubcommand::List(args) => args.execute(runtime),
            JobSubcommand::Show(args) => args.execute(runtime),
            JobSubcommand::Delete(args) => args.execute(runtime),
        }
    }
}

#[derive(Args)]
pub struct JobAddArgs {
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
    pub identity: Option<String>,
    #[arg(long)]
    pub assigned_to: Option<String>,
    #[arg(long)]
    pub created_by: Option<String>,
    #[arg(long)]
    pub json: bool,
}

impl Execute for JobAddArgs {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        let input_schema_json =
            parse_optional_json_object(self.input_schema.as_deref(), "input_schema")?;
        let output_schema_json =
            parse_optional_json_object(self.output_schema.as_deref(), "output_schema")?;
        let skill_refs = parse_csv(&self.skill_refs);

        let spec = runtime.add_job(JobAddParams {
            id: self.id,
            spec_type: self.spec_type,
            description: self.description,
            input_schema_json,
            output_schema_json,
            artifact_path_template: self.artifact_path_template,
            skill_refs,
            identity_id: self.identity,
            assigned_to: self.assigned_to,
            created_by: self.created_by,
        })?;

        if self.json {
            crate::output::json::print_pretty(&job_to_json(&spec))
        } else {
            println!("{}", spec.id);
            Ok(())
        }
    }
}

#[derive(Args)]
pub struct JobListArgs {
    #[arg(long)]
    pub all: bool,
    #[arg(long)]
    pub json: bool,
}

impl Execute for JobListArgs {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        let specs = runtime.list_jobs(self.all)?;
        if self.json {
            let values = specs.iter().map(job_to_json).collect::<Vec<_>>();
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
pub struct JobShowArgs {
    pub id: String,
    #[arg(long)]
    pub json: bool,
}

impl Execute for JobShowArgs {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        let spec = runtime.show_job(&self.id)?;
        if self.json {
            crate::output::json::print_pretty(&job_to_json(&spec))
        } else {
            println!("ID:                  {}", spec.id);
            println!("Type:                {}", spec.spec_type);
            println!("Description:         {}", spec.description);
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
pub struct JobDeleteArgs {
    pub id: String,
}

impl Execute for JobDeleteArgs {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        runtime.delete_job(&self.id)?;
        println!("Deleted job '{}'", self.id);
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

fn job_to_json(spec: &Job) -> Value {
    json!({
        "id": spec.id,
        "type": spec.spec_type,
        "description": spec.description,
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
