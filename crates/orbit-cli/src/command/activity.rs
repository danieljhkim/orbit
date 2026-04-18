use std::path::PathBuf;

use clap::{Args, Subcommand};
use orbit_core::command::activity::{ActivityAddParams, ActivityRunParams, ActivityUpdateParams};
use orbit_core::{Activity, OrbitError, OrbitRuntime};
use serde_json::{Value, json};

use crate::command::Execute;

#[derive(Args)]
#[command(about = "Manage activity definitions")]
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
    /// Register a new activity definition
    Add(Box<ActivityAddArgs>),
    /// List all registered activities
    List(ActivityListArgs),
    /// Show details of a specific activity
    Show(ActivityShowArgs),
    /// Update an existing activity
    Update(ActivityUpdateArgs),
    /// Execute an activity immediately
    Run(ActivityRunArgs),
    /// Execute a v2 activity from a YAML path (schemaVersion: 2)
    #[command(name = "run-v2")]
    RunV2(ActivityRunV2Args),
    /// Delete an activity definition
    Delete(ActivityDeleteArgs),
}

impl Execute for ActivitySubcommand {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        match self {
            ActivitySubcommand::Add(args) => (*args).execute(runtime),
            ActivitySubcommand::List(args) => args.execute(runtime),
            ActivitySubcommand::Show(args) => args.execute(runtime),
            ActivitySubcommand::Update(args) => args.execute(runtime),
            ActivitySubcommand::Run(args) => args.execute(runtime),
            ActivitySubcommand::RunV2(args) => args.execute(runtime),
            ActivitySubcommand::Delete(args) => args.execute(runtime),
        }
    }
}

#[derive(Args)]
pub struct ActivityAddArgs {
    #[arg(long)]
    pub id: String,
    #[arg(long = "spec-type", alias = "type", default_value = "agent_invoke")]
    pub spec_type: String,
    #[arg(long)]
    pub description: String,
    #[arg(long)]
    pub input_schema: Option<String>,
    #[arg(long)]
    pub output_schema: Option<String>,
    #[arg(long)]
    pub spec_config: Option<String>,
    #[arg(long)]
    pub workspace_path: Option<String>,
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
        let spec_config = parse_optional_json_object(self.spec_config.as_deref(), "spec_config")?;

        let spec = runtime.add_activity(ActivityAddParams {
            id: self.id,
            spec_type: self.spec_type,
            description: self.description,
            input_schema_json,
            output_schema_json,
            spec_config,
            executor: None,
            workspace_path: self.workspace_path,
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
            let values = specs
                .iter()
                .map(activity_to_signal_json)
                .collect::<Vec<_>>();
            crate::output::json::print_pretty(&Value::Array(values))
        } else if self.json {
            let values = specs.iter().map(activity_to_json).collect::<Vec<_>>();
            crate::output::json::print_pretty(&Value::Array(values))
        } else {
            let mut table =
                crate::output::table::build_table(&["ID", "TYPE", "ACTIVE", "DESCRIPTION"]);
            for spec in &specs {
                use comfy_table::Cell;
                table.add_row(vec![
                    Cell::new(&spec.id),
                    Cell::new(&spec.spec_type),
                    crate::output::color::job_state_color_cell(if spec.is_active {
                        "active"
                    } else {
                        "disabled"
                    }),
                    Cell::new(&spec.description),
                ]);
            }
            println!("{table}");
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
            use crate::output::color::{bold, dimmed, job_state_color};
            println!("{} {}", bold("ID:"), spec.id);
            println!("{} {}", bold("Type:"), spec.spec_type);
            println!("{} {}", bold("Description:"), spec.description);
            println!(
                "{} {}",
                bold("Spec Config:"),
                serde_json::to_string(&spec.spec_config).unwrap_or_else(|_| "{}".to_string())
            );
            if let Some(ref created_by) = spec.created_by {
                println!("{} {}", bold("Created By:"), created_by);
            }
            let active_str = if spec.is_active { "active" } else { "disabled" };
            println!("{} {}", bold("Active:"), job_state_color(active_str));
            println!(
                "{} {}",
                bold("Created:"),
                dimmed(&spec.created_at.to_rfc3339())
            );
            println!(
                "{} {}",
                bold("Updated:"),
                dimmed(&spec.updated_at.to_rfc3339())
            );
            Ok(())
        }
    }
}

#[derive(Args)]
pub struct ActivityUpdateArgs {
    pub id: String,
    #[arg(long)]
    pub description: Option<String>,
    #[arg(long)]
    pub input_schema: Option<String>,
    #[arg(long)]
    pub output_schema: Option<String>,
    #[arg(long)]
    pub spec_config: Option<String>,
    #[arg(long)]
    pub workspace_path: Option<String>,
    #[arg(long)]
    pub clear_workspace_path: bool,
    #[arg(long, conflicts_with = "inactive")]
    pub active: bool,
    #[arg(long, conflicts_with = "active")]
    pub inactive: bool,
    #[arg(long)]
    pub json: bool,
}

impl Execute for ActivityUpdateArgs {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        let input_schema_json = self
            .input_schema
            .as_deref()
            .map(|raw| parse_json_object(raw, "input_schema"))
            .transpose()?;
        let output_schema_json = self
            .output_schema
            .as_deref()
            .map(|raw| parse_json_object(raw, "output_schema"))
            .transpose()?;
        let spec_config = self
            .spec_config
            .as_deref()
            .map(|raw| parse_json_object(raw, "spec_config"))
            .transpose()?;
        let workspace_path = if self.clear_workspace_path {
            Some(None)
        } else {
            self.workspace_path.map(Some)
        };
        let is_active = if self.active {
            Some(true)
        } else if self.inactive {
            Some(false)
        } else {
            None
        };

        let activity = runtime.update_activity(
            &self.id,
            ActivityUpdateParams {
                description: self.description,
                input_schema_json,
                output_schema_json,
                spec_config,
                executor: None,
                workspace_path,
                created_by: None,
                is_active,
            },
        )?;

        if self.json {
            crate::output::json::print_pretty(&activity_to_json(&activity))
        } else {
            println!("Updated activity '{}'", activity.id);
            Ok(())
        }
    }
}

#[derive(Args)]
pub struct ActivityRunArgs {
    pub id: String,
    #[arg(long)]
    pub agent_cli: Option<String>,
    #[arg(long, default_value = "5m")]
    pub timeout: String,
    #[arg(long)]
    pub json: bool,
}

impl Execute for ActivityRunArgs {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        let result = runtime.run_activity_now(ActivityRunParams {
            activity_id: self.id,
            agent_cli: self.agent_cli.unwrap_or_default(),
            timeout_seconds: crate::parse::parse_duration_seconds(&self.timeout)?,
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
pub struct ActivityRunV2Args {
    /// Path to a v2 (schemaVersion:2) activity YAML file.
    pub path: PathBuf,
    /// Optional JSON input passed to the dispatcher.
    #[arg(long, default_value = "null")]
    pub input: String,
    #[arg(long)]
    pub json: bool,
}

impl Execute for ActivityRunV2Args {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        let input: Value = serde_json::from_str(&self.input)
            .map_err(|e| OrbitError::InvalidInput(format!("--input must be valid JSON: {e}")))?;
        let result = runtime.run_activity_v2_from_yaml(&self.path, input)?;
        let audit_jsonl_str = result
            .audit_jsonl
            .as_ref()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|| "-".to_string());
        if self.json {
            crate::output::json::print_pretty(&json!({
                "activity_name": result.activity_name,
                "activity_type": result.activity_type,
                "success": result.success,
                "message": result.message,
                "output": result.output,
                "audit_jsonl": audit_jsonl_str,
                "events_emitted": result.events_emitted,
            }))
        } else {
            println!(
                "activity={};type={};success={};events={};audit_jsonl={}",
                result.activity_name,
                result.activity_type,
                result.success,
                result.events_emitted,
                audit_jsonl_str,
            );
            if let Some(msg) = &result.message {
                println!("message: {msg}");
            }
            println!(
                "output: {}",
                serde_json::to_string_pretty(&result.output).unwrap_or_default()
            );
            Ok(())
        }
    }
}

#[derive(Args)]
pub struct ActivityDeleteArgs {
    pub id: String,
    #[arg(long)]
    pub json: bool,
}

impl Execute for ActivityDeleteArgs {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        runtime.delete_activity(&self.id)?;
        if self.json {
            crate::output::json::print_pretty(&json!({
                "id": self.id,
                "deleted": true,
            }))
        } else {
            println!("Deleted activity '{}'", self.id);
            Ok(())
        }
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
        "input_schema_json": spec.input_schema_json,
        "output_schema_json": spec.output_schema_json,
        "spec_config": spec.spec_config,
        "workspace_path": spec.workspace_path,
        "created_by": spec.created_by,
        "is_active": spec.is_active,
        "created_at": spec.created_at.to_rfc3339(),
        "updated_at": spec.updated_at.to_rfc3339(),
    })
}
