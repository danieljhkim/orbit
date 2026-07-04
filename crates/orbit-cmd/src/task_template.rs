use orbit_common::types::{OrbitError, TaskPriority, TaskType};
use serde::Deserialize;

use orbit_core::OrbitRuntime;

// ---------------------------------------------------------------------------
// Built-in templates (embedded at compile time)
// ---------------------------------------------------------------------------

const BUILTIN_TEMPLATES: [(&str, &str); 4] = [
    (
        "bug-fix",
        include_str!("../assets/task_templates/bug-fix.yaml"),
    ),
    ("chore", include_str!("../assets/task_templates/chore.yaml")),
    (
        "feature",
        include_str!("../assets/task_templates/feature.yaml"),
    ),
    ("spike", include_str!("../assets/task_templates/spike.yaml")),
];

// ---------------------------------------------------------------------------
// Template type
// ---------------------------------------------------------------------------

/// A resolved task template ready for use.
#[derive(Debug, Clone)]
pub struct TaskTemplate {
    pub name: String,
    pub description: String,
    pub task_type: TaskType,
    pub priority: TaskPriority,
    pub description_template: String,
    pub plan_template: String,
    pub instructions_template: String,
    /// True if this template came from the built-in set; false if user-defined.
    pub builtin: bool,
}

/// Raw YAML deserialization shape.
#[derive(Debug, Deserialize)]
struct RawTemplate {
    name: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    task_type: Option<TaskType>,
    #[serde(default)]
    priority: Option<RawPriority>,
    #[serde(default)]
    description_template: String,
    #[serde(default)]
    plan_template: String,
    #[serde(default)]
    instructions_template: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
enum RawPriority {
    Low,
    Medium,
    High,
    Critical,
}

impl From<RawPriority> for TaskPriority {
    fn from(v: RawPriority) -> Self {
        match v {
            RawPriority::Low => TaskPriority::Low,
            RawPriority::Medium => TaskPriority::Medium,
            RawPriority::High => TaskPriority::High,
            RawPriority::Critical => TaskPriority::Critical,
        }
    }
}

// ---------------------------------------------------------------------------
// Parsing helpers
// ---------------------------------------------------------------------------

fn parse_template(yaml: &str, builtin: bool) -> Result<TaskTemplate, OrbitError> {
    let raw: RawTemplate = serde_yaml::from_str(yaml)
        .map_err(|e| OrbitError::InvalidInput(format!("failed to parse task template: {e}")))?;

    Ok(TaskTemplate {
        name: raw.name,
        description: raw.description,
        task_type: raw.task_type.unwrap_or(TaskType::Chore),
        priority: raw.priority.map(Into::into).unwrap_or(TaskPriority::Medium),
        description_template: raw.description_template,
        plan_template: raw.plan_template,
        instructions_template: raw.instructions_template,
        builtin,
    })
}

// ---------------------------------------------------------------------------
// OrbitRuntime methods
// ---------------------------------------------------------------------------

/// Task-template command surface for [`OrbitRuntime`] (extension trait —
/// the implementation moved out of orbit-core in [ORB-10016]).
pub trait TaskTemplateCommands {
    /// Returns the path where user-defined templates are stored:
    /// `<data_root>/task_templates/`
    fn task_templates_dir(&self) -> std::path::PathBuf;

    /// List all available templates: built-ins first, then user-defined ones.
    ///
    /// User-defined templates with the same name as a built-in override the built-in.
    fn list_task_templates(&self) -> Result<Vec<TaskTemplate>, OrbitError>;

    /// Look up a single template by name.
    fn get_task_template(&self, name: &str) -> Result<TaskTemplate, OrbitError>;
}

impl TaskTemplateCommands for OrbitRuntime {
    fn task_templates_dir(&self) -> std::path::PathBuf {
        self.data_root().join("task_templates")
    }

    fn list_task_templates(&self) -> Result<Vec<TaskTemplate>, OrbitError> {
        let mut templates: Vec<TaskTemplate> = Vec::new();

        // Load built-ins first.
        for (_, yaml) in BUILTIN_TEMPLATES {
            templates.push(parse_template(yaml, true)?);
        }

        // Load user-defined templates and override/extend.
        let user_dir = self.task_templates_dir();
        if user_dir.is_dir() {
            let mut entries = std::fs::read_dir(&user_dir)
                .map_err(|e| OrbitError::Io(format!("failed to read task_templates dir: {e}")))?
                .filter_map(|entry| entry.ok())
                .filter(|entry| {
                    let p = entry.path();
                    p.extension()
                        .is_some_and(|ext| ext == "yaml" || ext == "yml")
                })
                .collect::<Vec<_>>();
            entries.sort_by_key(|e| e.file_name());

            for entry in entries {
                let path = entry.path();
                let yaml = std::fs::read_to_string(&path).map_err(|e| {
                    OrbitError::Io(format!("failed to read template '{}': {e}", path.display()))
                })?;
                let tmpl = parse_template(&yaml, false).map_err(|e| {
                    OrbitError::InvalidInput(format!("invalid template '{}': {e}", path.display()))
                })?;
                // User-defined templates override built-ins of the same name.
                if let Some(existing) = templates.iter_mut().find(|t| t.name == tmpl.name) {
                    *existing = tmpl;
                } else {
                    templates.push(tmpl);
                }
            }
        }

        Ok(templates)
    }

    fn get_task_template(&self, name: &str) -> Result<TaskTemplate, OrbitError> {
        // Check user-defined first (they take priority).
        let user_dir = self.task_templates_dir();
        if user_dir.is_dir() {
            for ext in ["yaml", "yml"] {
                let path = user_dir.join(format!("{name}.{ext}"));
                if path.is_file() {
                    let yaml = std::fs::read_to_string(&path).map_err(|e| {
                        OrbitError::Io(format!("failed to read template '{}': {e}", path.display()))
                    })?;
                    return parse_template(&yaml, false);
                }
            }
        }

        // Fall back to built-ins.
        for (id, yaml) in BUILTIN_TEMPLATES {
            if id == name {
                return parse_template(yaml, true);
            }
        }

        let available = self
            .list_task_templates()?
            .into_iter()
            .map(|template| template.name)
            .collect::<Vec<_>>()
            .join(", ");
        Err(OrbitError::InvalidInput(format!(
            "task template '{name}' not found; available templates: {available}"
        )))
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------
