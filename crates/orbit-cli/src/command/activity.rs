use std::path::PathBuf;

use clap::{Args, Subcommand};
use orbit_core::{OrbitError, OrbitRuntime};
use serde_json::{Value, json};

use crate::command::Execute;

#[derive(Args)]
#[command(about = "List and run v2 activities")]
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
    /// List all registered activities
    List(ActivityListArgs),
    /// Execute an activity from a YAML path
    Run(ActivityRunArgs),
}

impl Execute for ActivitySubcommand {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        match self {
            ActivitySubcommand::List(args) => args.execute(runtime),
            ActivitySubcommand::Run(args) => args.execute(runtime),
        }
    }
}

#[derive(Args)]
pub struct ActivityListArgs {
    #[arg(long)]
    pub json: bool,
    /// Output signal-tier JSON (id, type, description only)
    #[arg(long)]
    pub ops: bool,
}

impl Execute for ActivityListArgs {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        let catalog = runtime
            .v2_activity_catalog()
            .map_err(|err| OrbitError::Store(format!("v2 activity catalog: {err}")))?;

        if self.ops {
            let values: Vec<Value> = catalog
                .names()
                .filter_map(|name| catalog.get(name).map(|spec| v2_signal_json(name, spec)))
                .collect();
            crate::output::json::print_pretty(&Value::Array(values))
        } else if self.json {
            let values: Vec<Value> = catalog
                .names()
                .filter_map(|name| catalog.get(name).map(|spec| v2_full_json(name, spec)))
                .collect();
            crate::output::json::print_pretty(&Value::Array(values))
        } else {
            let mut table = crate::output::table::build_table(&["ID", "TYPE", "DESCRIPTION"]);
            for name in catalog.names() {
                use comfy_table::Cell;
                let Some(spec) = catalog.get(name) else {
                    continue;
                };
                table.add_row(vec![
                    Cell::new(name),
                    Cell::new(v2_type_label(spec)),
                    Cell::new(&spec.description),
                ]);
            }
            println!("{table}");
            Ok(())
        }
    }
}

#[derive(Args)]
#[command(
    after_help = "Examples:\n  orbit activity run crates/orbit-core/assets/activities/worktree_setup.yaml\n  orbit activity run path/to/agent.yaml --input '{\"task_id\":\"T123\"}'\n"
)]
pub struct ActivityRunArgs {
    /// Path to a schemaVersion 2 activity YAML file.
    pub path: PathBuf,
    /// Optional JSON input passed to the dispatcher.
    #[arg(long, default_value = "null")]
    pub input: String,
    /// Explicit execution backend override for `agent_loop` activities (§3.1).
    /// Precedence: this flag > `ORBIT_BACKEND` > `[runtime] backend` > `http`.
    /// Accepted values: `http`, `cli`, `auto`.
    #[arg(long)]
    pub backend: Option<String>,
    #[arg(long)]
    pub json: bool,
}

impl Execute for ActivityRunArgs {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        let input: Value = serde_json::from_str(&self.input)
            .map_err(|e| OrbitError::InvalidInput(format!("--input must be valid JSON: {e}")))?;
        let backend_flag =
            orbit_core::command::backend_resolver::parse_backend_flag(self.backend.as_deref())
                .map_err(OrbitError::InvalidInput)?;
        let result = runtime.run_activity_v2_from_yaml(&self.path, input, backend_flag)?;
        let audit_jsonl_str = result
            .audit_jsonl
            .as_ref()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|| "-".to_string());
        let backend_str = result
            .resolved_backend
            .map(|b| b.as_str().to_string())
            .unwrap_or_else(|| "n/a".to_string());
        if self.json {
            crate::output::json::print_pretty(&json!({
                "activity_name": result.activity_name,
                "activity_type": result.activity_type,
                "resolved_backend": backend_str,
                "success": result.success,
                "message": result.message,
                "output": result.output,
                "audit_jsonl": audit_jsonl_str,
                "events_emitted": result.events_emitted,
            }))
        } else {
            println!(
                "activity={};type={};backend={};success={};events={};audit_jsonl={}",
                result.activity_name,
                result.activity_type,
                backend_str,
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

fn v2_type_label(spec: &orbit_common::types::activity_job::ActivityV2) -> &'static str {
    use orbit_common::types::activity_job::ActivityV2Spec;
    match &spec.spec {
        ActivityV2Spec::AgentLoop(_) => "agent_loop",
        ActivityV2Spec::Groundhog(_) => "groundhog",
        ActivityV2Spec::Deterministic(_) => "deterministic",
        ActivityV2Spec::Shell(_) => "shell",
    }
}

fn v2_full_json(name: &str, spec: &orbit_common::types::activity_job::ActivityV2) -> Value {
    json!({
        "id": name,
        "type": v2_type_label(spec),
        "description": spec.description,
        "input_schema_json": spec.input_schema_json,
        "output_schema_json": spec.output_schema_json,
        "fsProfile": spec.fs_profile,
        "schemaVersion": 2,
    })
}

fn v2_signal_json(name: &str, spec: &orbit_common::types::activity_job::ActivityV2) -> Value {
    json!({
        "id": name,
        "type": v2_type_label(spec),
        "description": spec.description,
    })
}
