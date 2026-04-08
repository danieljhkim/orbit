use std::path::Path;

use clap::{Args, Subcommand, ValueEnum};
use orbit_core::{OrbitError, OrbitRuntime};
use serde_json::{Value, json};

use crate::command::Execute;

#[derive(Args)]
#[command(about = "Manage and run Orbit tools")]
pub struct ToolCommand {
    #[command(subcommand)]
    pub command: ToolSubcommand,
}

impl Execute for ToolCommand {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        self.command.execute(runtime)
    }
}

#[derive(Subcommand)]
pub enum ToolSubcommand {
    /// List all registered tools
    List(ToolListArgs),
    /// Show detailed information about a tool
    Show(ToolShowArgs),
    /// Execute a tool
    Run(ToolRunArgs),
    /// Register an external tool
    Add(ToolAddArgs),
    /// Remove an external tool
    Remove(ToolRemoveArgs),
    /// Enable a disabled tool
    Enable(ToolEnableArgs),
    /// Disable a tool
    Disable(ToolDisableArgs),
    /// Validate tool health
    Doctor,
}

impl Execute for ToolSubcommand {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        match self {
            ToolSubcommand::List(args) => args.execute(runtime),
            ToolSubcommand::Show(args) => args.execute(runtime),
            ToolSubcommand::Run(args) => args.execute(runtime),
            ToolSubcommand::Add(args) => args.execute(runtime),
            ToolSubcommand::Remove(args) => args.execute(runtime),
            ToolSubcommand::Enable(args) => args.execute(runtime),
            ToolSubcommand::Disable(args) => args.execute(runtime),
            ToolSubcommand::Doctor => execute_doctor(runtime),
        }
    }
}

// --- List ---

#[derive(Args)]
pub struct ToolListArgs {
    /// Output as JSON
    #[arg(long)]
    pub json: bool,
}

impl Execute for ToolListArgs {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        let tools = runtime.list_tools()?;

        if self.json {
            let json_tools: Vec<Value> = tools
                .iter()
                .map(|t| {
                    json!({
                        "name": t.name,
                        "description": t.description,
                        "enabled": t.enabled,
                        "builtin": t.builtin,
                    })
                })
                .collect();
            crate::output::json::print_pretty(&Value::Array(json_tools))
        } else {
            let mut table =
                crate::output::table::build_table(&["NAME", "ENABLED", "BUILTIN", "DESCRIPTION"]);
            for tool in &tools {
                use comfy_table::Cell;
                table.add_row(vec![
                    Cell::new(&tool.name),
                    crate::output::color::job_state_color_cell(if tool.enabled {
                        "active"
                    } else {
                        "disabled"
                    }),
                    Cell::new(if tool.builtin { "yes" } else { "no" }),
                    Cell::new(&tool.description),
                ]);
            }
            println!("{table}");
            Ok(())
        }
    }
}

// --- Show ---

#[derive(Args)]
pub struct ToolShowArgs {
    /// Tool name
    pub name: String,
}

impl Execute for ToolShowArgs {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        let tool = runtime.show_tool(&self.name)?;

        use crate::output::color::{bold, job_state_color};
        println!("{} {}", bold("Name:"), tool.name);
        println!("{} {}", bold("Description:"), tool.description);
        println!(
            "{} {}",
            bold("Builtin:"),
            if tool.builtin { "yes" } else { "no" }
        );
        println!(
            "{} {}",
            bold("Enabled:"),
            job_state_color(if tool.enabled { "active" } else { "disabled" })
        );

        if tool.parameters.is_empty() {
            println!("{} (none)", bold("Parameters:"));
        } else {
            println!("{}", bold("Parameters:"));
            let mut table =
                crate::output::table::build_table(&["NAME", "TYPE", "REQUIRED", "DESCRIPTION"]);
            for p in &tool.parameters {
                let req = if p.required { "required" } else { "optional" };
                table.add_row(vec![
                    p.name.clone(),
                    p.param_type.clone(),
                    req.to_string(),
                    p.description.clone(),
                ]);
            }
            println!("{table}");
        }

        Ok(())
    }
}

// --- Run ---

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
    /// Execution timeout (e.g. "30s", "5000ms")
    #[arg(long)]
    pub timeout: Option<String>,
    /// Validate without executing
    #[arg(long)]
    pub dry_run: bool,
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

        let output = runtime.execute_tool_command(&self.name, input)?;

        match self.output {
            OutputFormat::Json => crate::output::json::print_pretty(&output),
            OutputFormat::Text => {
                println!("{}", output);
                Ok(())
            }
        }
    }
}

// --- Add ---

#[derive(Args)]
pub struct ToolAddArgs {
    /// Path to the external tool executable
    pub path: String,
    /// Tool name (inferred from filename if omitted)
    #[arg(long)]
    pub name: Option<String>,
    /// Tool description
    #[arg(long, default_value = "")]
    pub description: String,
}

impl Execute for ToolAddArgs {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        let name = self.name.unwrap_or_else(|| {
            Path::new(&self.path)
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("unknown")
                .to_string()
        });

        runtime.add_tool(&name, &self.path, &self.description)?;
        println!("Added tool '{name}' from {}", self.path);
        Ok(())
    }
}

// --- Remove ---

#[derive(Args)]
pub struct ToolRemoveArgs {
    /// Tool name to remove
    pub name: String,
}

impl Execute for ToolRemoveArgs {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        runtime.remove_tool(&self.name)?;
        println!("Removed tool '{}'", self.name);
        Ok(())
    }
}

// --- Enable ---

#[derive(Args)]
pub struct ToolEnableArgs {
    /// Tool name to enable
    pub name: String,
}

impl Execute for ToolEnableArgs {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        runtime.enable_tool(&self.name)?;
        println!("Enabled tool '{}'", self.name);
        Ok(())
    }
}

// --- Disable ---

#[derive(Args)]
pub struct ToolDisableArgs {
    /// Tool name to disable
    pub name: String,
}

impl Execute for ToolDisableArgs {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        runtime.disable_tool(&self.name)?;
        println!("Disabled tool '{}'", self.name);
        Ok(())
    }
}

// --- Doctor ---

fn execute_doctor(runtime: &OrbitRuntime) -> Result<(), OrbitError> {
    use orbit_core::command::tool::DoctorStatus;

    let results = runtime.doctor()?;
    let mut issues = 0;

    let mut table = crate::output::table::build_table(&["TOOL", "STATUS", "DETAILS"]);
    for r in &results {
        let status_str = match r.status {
            DoctorStatus::Ok => "ok",
            DoctorStatus::Warning => "warning",
            DoctorStatus::Error => "ERROR",
        };
        if r.status != DoctorStatus::Ok {
            issues += 1;
        }
        use comfy_table::Cell;
        table.add_row(vec![
            Cell::new(&r.tool_name),
            crate::output::color::doctor_status_color_cell(status_str),
            Cell::new(&r.message),
        ]);
    }
    println!("{table}");

    if issues == 0 {
        println!(
            "\n{}",
            crate::output::color::job_state_color("All tools healthy.")
        );
    } else {
        eprintln!("\n{} issue(s) found.", issues);
    }

    Ok(())
}
