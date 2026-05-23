use std::fs;
use std::path::{Path, PathBuf};

use clap::{Args, Subcommand, ValueEnum};
use orbit_common::types::ToolParam;
use orbit_core::command::tool::ToolInfo;
use orbit_core::{OrbitError, OrbitRuntime};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use crate::command::Execute;

const TOOL_COMMAND_AFTER_HELP: &str = "\
Examples:
  orbit tool scaffold ./plugins/hello_orbit.py --name demo.hello
  orbit tool add ./plugins/hello_orbit.py
  orbit tool show demo.hello
";
const EXTERNAL_TOOL_TEMPLATE: &str =
    include_str!("../../../assets/tool_templates/external_tool.py.tmpl");
const SCAFFOLD_DEFAULT_DESCRIPTION: &str =
    "Return a greeting and optionally echo Orbit tool context.";

#[derive(Args)]
#[command(
    about = "Manage and run Orbit tools, including external MCP plugins",
    after_help = TOOL_COMMAND_AFTER_HELP
)]
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
    /// Register an external tool or MCP plugin
    Add(ToolAddArgs),
    /// Generate a starter external tool plugin
    Scaffold(ToolScaffoldArgs),
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
            ToolSubcommand::Scaffold(args) => args.execute(runtime),
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
    /// Include inactive tools that are hidden from the default agent surface
    #[arg(long, alias = "include-hidden")]
    pub all: bool,
}

impl Execute for ToolListArgs {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        let tools = if self.all {
            runtime.list_all_tools()?
        } else {
            runtime.list_tools()?
        };

        if self.json {
            let json_tools: Vec<Value> = tools
                .iter()
                .map(|t| {
                    json!({
                        "name": t.name,
                        "description": t.description,
                        "enabled": t.enabled,
                        "active": t.active,
                        "status": tool_status(t),
                        "builtin": t.builtin,
                        "parameters": &t.parameters,
                    })
                })
                .collect();
            crate::output::json::print_pretty(&Value::Array(json_tools))
        } else {
            let mut table = crate::output::table::build_table(&[
                "NAME",
                "STATUS",
                "BUILTIN",
                "REQUIRED INPUT",
                "DESCRIPTION",
            ]);
            for tool in &tools {
                use comfy_table::Cell;
                table.add_row(vec![
                    Cell::new(&tool.name),
                    crate::output::color::job_state_color_cell(tool_status(tool)),
                    Cell::new(if tool.builtin { "yes" } else { "no" }),
                    Cell::new(format_required_tool_input_summary(&tool.parameters)),
                    Cell::new(&tool.description),
                ]);
            }
            println!("{table}");
            Ok(())
        }
    }
}

fn tool_status(tool: &ToolInfo) -> &'static str {
    if !tool.active {
        "inactive"
    } else if !tool.enabled {
        "disabled"
    } else {
        "active"
    }
}

fn format_required_tool_input_summary(parameters: &[ToolParam]) -> String {
    let required_inputs = parameters
        .iter()
        .filter(|param| param.required)
        .map(format_tool_param_signature)
        .collect::<Vec<_>>();

    if required_inputs.is_empty() {
        "-".to_string()
    } else {
        required_inputs.join(", ")
    }
}

fn format_tool_param_signature(param: &ToolParam) -> String {
    if param.param_type.trim().is_empty() {
        param.name.clone()
    } else {
        format!("{}:{}", param.name, display_param_type(&param.param_type))
    }
}

fn display_param_type(param_type: &str) -> &str {
    match param_type {
        "string_list" => "string|string[]",
        other => other,
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
            bold("Status:"),
            job_state_color(tool_status(&tool))
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

// --- Add ---

#[derive(Args)]
pub struct ToolAddArgs {
    /// Path to the external tool executable
    pub path: String,
    /// Tool name (overrides the manifest or filename-derived default)
    #[arg(long)]
    pub name: Option<String>,
    /// Tool description (overrides the manifest description)
    #[arg(long, default_value = "")]
    pub description: String,
    /// Path to a sidecar plugin manifest (`*.orbit-tool.yaml`, `*.yml`, or `*.json`)
    #[arg(long)]
    pub manifest: Option<String>,
}

impl Execute for ToolAddArgs {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        let manifest_path = resolve_manifest_path(Path::new(&self.path), self.manifest.as_deref());
        let manifest = manifest_path
            .as_deref()
            .map(load_external_tool_manifest)
            .transpose()?;

        let name = self
            .name
            .or_else(|| manifest.as_ref().map(|entry| entry.name.clone()))
            .unwrap_or_else(|| infer_tool_name(Path::new(&self.path)));
        if name.trim().is_empty() {
            return Err(OrbitError::InvalidInput(
                "tool name must not be empty".to_string(),
            ));
        }

        let description = if self.description.trim().is_empty() {
            manifest
                .as_ref()
                .map(|entry| entry.description.clone())
                .unwrap_or_default()
        } else {
            self.description.trim().to_string()
        };
        let parameters = manifest.map(|entry| entry.parameters).unwrap_or_default();

        runtime.add_tool(&name, &self.path, &description, parameters)?;
        println!("Added tool '{name}' from {}", self.path);
        if let Some(path) = manifest_path {
            println!("Loaded plugin manifest from {}", path.display());
        }
        Ok(())
    }
}

// --- Scaffold ---

#[derive(Args)]
pub struct ToolScaffoldArgs {
    /// Path to the starter executable to create
    pub path: String,
    /// Tool name to place in the generated manifest
    #[arg(long)]
    pub name: Option<String>,
    /// Tool description to place in the generated manifest
    #[arg(long, default_value = SCAFFOLD_DEFAULT_DESCRIPTION)]
    pub description: String,
    /// Overwrite existing files
    #[arg(long)]
    pub force: bool,
}

impl Execute for ToolScaffoldArgs {
    fn execute(self, _runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        let script_path = PathBuf::from(&self.path);
        let manifest_path = sidecar_manifest_path(&script_path);
        let tool_name = self
            .name
            .unwrap_or_else(|| infer_tool_name(script_path.as_path()));
        let description = self.description.trim().to_string();

        ensure_scaffold_targets_clear(&script_path, &manifest_path, self.force)?;

        if let Some(parent) = script_path.parent() {
            fs::create_dir_all(parent)
                .map_err(|error| OrbitError::Io(format!("create {}: {error}", parent.display())))?;
        }

        let script = EXTERNAL_TOOL_TEMPLATE.replace("__ORBIT_TOOL_NAME__", &tool_name);
        fs::write(&script_path, script)
            .map_err(|error| OrbitError::Io(format!("write {}: {error}", script_path.display())))?;
        make_executable(&script_path)?;

        let manifest = ExternalToolManifest {
            schema_version: 1,
            name: tool_name.clone(),
            description,
            parameters: scaffold_parameters(),
        };
        let manifest_yaml = serde_yaml::to_string(&manifest)
            .map_err(|error| OrbitError::InvalidInput(format!("serialize manifest: {error}")))?;
        fs::write(&manifest_path, manifest_yaml).map_err(|error| {
            OrbitError::Io(format!("write {}: {error}", manifest_path.display()))
        })?;

        println!("Created starter plugin:");
        println!("  executable: {}", script_path.display());
        println!("  manifest:   {}", manifest_path.display());
        println!("\nNext steps:");
        println!("  orbit tool add {}", script_path.display());
        println!("  orbit tool show {}", tool_name);
        println!("  orbit mcp serve");
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

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ExternalToolManifest {
    #[serde(rename = "schemaVersion", default = "default_manifest_schema_version")]
    schema_version: u32,
    name: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    parameters: Vec<ToolParam>,
}

fn default_manifest_schema_version() -> u32 {
    1
}

fn resolve_manifest_path(tool_path: &Path, explicit: Option<&str>) -> Option<PathBuf> {
    if let Some(path) = explicit {
        return Some(PathBuf::from(path));
    }

    manifest_candidates(tool_path)
        .into_iter()
        .find(|candidate| candidate.exists())
}

fn manifest_candidates(tool_path: &Path) -> Vec<PathBuf> {
    vec![
        sidecar_manifest_path_with_extension(tool_path, "yaml"),
        sidecar_manifest_path_with_extension(tool_path, "yml"),
        sidecar_manifest_path_with_extension(tool_path, "json"),
    ]
}

fn sidecar_manifest_path(tool_path: &Path) -> PathBuf {
    sidecar_manifest_path_with_extension(tool_path, "yaml")
}

fn sidecar_manifest_path_with_extension(tool_path: &Path, extension: &str) -> PathBuf {
    let parent = tool_path.parent().unwrap_or_else(|| Path::new("."));
    let stem = tool_path
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("external-tool");
    parent.join(format!("{stem}.orbit-tool.{extension}"))
}

fn infer_tool_name(path: &Path) -> String {
    path.file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("external-tool")
        .to_string()
}

fn load_external_tool_manifest(path: &Path) -> Result<ExternalToolManifest, OrbitError> {
    let raw = fs::read_to_string(path).map_err(|error| {
        OrbitError::InvalidInput(format!("cannot read {}: {error}", path.display()))
    })?;
    let manifest = match path.extension().and_then(|value| value.to_str()) {
        Some("json") => serde_json::from_str::<ExternalToolManifest>(&raw).map_err(|error| {
            OrbitError::InvalidInput(format!(
                "invalid manifest JSON '{}': {error}",
                path.display()
            ))
        })?,
        _ => serde_yaml::from_str::<ExternalToolManifest>(&raw).map_err(|error| {
            OrbitError::InvalidInput(format!(
                "invalid manifest YAML '{}': {error}",
                path.display()
            ))
        })?,
    };
    validate_manifest(path, manifest)
}

fn validate_manifest(
    path: &Path,
    manifest: ExternalToolManifest,
) -> Result<ExternalToolManifest, OrbitError> {
    if manifest.schema_version != 1 {
        return Err(OrbitError::InvalidInput(format!(
            "unsupported plugin manifest schemaVersion {} in '{}'",
            manifest.schema_version,
            path.display()
        )));
    }
    if manifest.name.trim().is_empty() {
        return Err(OrbitError::InvalidInput(format!(
            "plugin manifest '{}' must define a non-empty name",
            path.display()
        )));
    }
    if manifest
        .parameters
        .iter()
        .any(|parameter| parameter.name.trim().is_empty())
    {
        return Err(OrbitError::InvalidInput(format!(
            "plugin manifest '{}' contains a parameter with an empty name",
            path.display()
        )));
    }
    Ok(manifest)
}

fn scaffold_parameters() -> Vec<ToolParam> {
    vec![
        ToolParam {
            name: "name".to_string(),
            description: "Greeting target echoed back by the example plugin.".to_string(),
            param_type: "string".to_string(),
            required: false,
        },
        ToolParam {
            name: "include_context".to_string(),
            description: "When true, include ORBIT_TOOL_* context values in the response."
                .to_string(),
            param_type: "boolean".to_string(),
            required: false,
        },
    ]
}

fn ensure_scaffold_targets_clear(
    script_path: &Path,
    manifest_path: &Path,
    force: bool,
) -> Result<(), OrbitError> {
    if force {
        return Ok(());
    }

    for path in [script_path, manifest_path] {
        if path.exists() {
            return Err(OrbitError::InvalidInput(format!(
                "refusing to overwrite existing file '{}'; rerun with --force",
                path.display()
            )));
        }
    }
    Ok(())
}

fn make_executable(path: &Path) -> Result<(), OrbitError> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        let metadata = fs::metadata(path)
            .map_err(|error| OrbitError::Io(format!("stat {}: {error}", path.display())))?;
        let mut permissions = metadata.permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(path, permissions)
            .map_err(|error| OrbitError::Io(format!("chmod {}: {error}", path.display())))?;
    }

    #[cfg(not(unix))]
    {
        let _ = path;
    }

    Ok(())
}
