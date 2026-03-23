use orbit_types::{OrbitError, TaskPriority, TaskType};
use serde::Deserialize;

use crate::OrbitRuntime;

// ---------------------------------------------------------------------------
// Built-in templates (embedded at compile time)
// ---------------------------------------------------------------------------

const BUILTIN_TEMPLATES: [(&str, &str); 4] = [
    (
        "bug-fix",
        include_str!("../../assets/task_templates/bug-fix.yaml"),
    ),
    (
        "chore",
        include_str!("../../assets/task_templates/chore.yaml"),
    ),
    (
        "feature",
        include_str!("../../assets/task_templates/feature.yaml"),
    ),
    (
        "spike",
        include_str!("../../assets/task_templates/spike.yaml"),
    ),
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
    task_type: Option<RawTaskType>,
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
enum RawTaskType {
    Task,
    Feature,
    Issue,
    Bug,
    Chore,
    Refactor,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
enum RawPriority {
    Low,
    Medium,
    High,
    Critical,
}

impl From<RawTaskType> for TaskType {
    fn from(v: RawTaskType) -> Self {
        match v {
            RawTaskType::Task => TaskType::Task,
            RawTaskType::Feature => TaskType::Feature,
            RawTaskType::Issue => TaskType::Issue,
            RawTaskType::Bug => TaskType::Bug,
            RawTaskType::Chore => TaskType::Chore,
            RawTaskType::Refactor => TaskType::Refactor,
        }
    }
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
        task_type: raw.task_type.map(Into::into).unwrap_or(TaskType::Task),
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

impl OrbitRuntime {
    /// Returns the path where user-defined templates are stored:
    /// `<data_root>/task_templates/`
    pub fn task_templates_dir(&self) -> std::path::PathBuf {
        self.data_root_path().join("task_templates")
    }

    /// List all available templates: built-ins first, then user-defined ones.
    ///
    /// User-defined templates with the same name as a built-in override the built-in.
    pub fn list_task_templates(&self) -> Result<Vec<TaskTemplate>, OrbitError> {
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

    /// Look up a single template by name.
    pub fn get_task_template(&self, name: &str) -> Result<TaskTemplate, OrbitError> {
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

        Err(OrbitError::InvalidInput(format!(
            "task template '{name}' not found; run `orbit task templates list` to see available templates"
        )))
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::OrbitRuntime;

    #[test]
    fn builtin_templates_parse_without_error() {
        for (name, yaml) in BUILTIN_TEMPLATES {
            let tmpl = parse_template(yaml, true)
                .unwrap_or_else(|e| panic!("built-in template '{name}' failed to parse: {e}"));
            assert_eq!(tmpl.name, name, "template name should match file key");
            assert!(
                !tmpl.description_template.is_empty(),
                "description_template should not be empty for '{name}'"
            );
            assert!(
                !tmpl.plan_template.is_empty(),
                "plan_template should not be empty for '{name}'"
            );
            assert!(
                !tmpl.instructions_template.is_empty(),
                "instructions_template should not be empty for '{name}'"
            );
            assert!(tmpl.builtin);
        }
    }

    #[test]
    fn list_task_templates_returns_all_builtins() {
        let runtime = OrbitRuntime::in_memory().expect("runtime");
        let templates = runtime.list_task_templates().expect("list");
        assert_eq!(templates.len(), 4);
        let names: Vec<&str> = templates.iter().map(|t| t.name.as_str()).collect();
        assert!(names.contains(&"bug-fix"));
        assert!(names.contains(&"feature"));
        assert!(names.contains(&"spike"));
        assert!(names.contains(&"chore"));
    }

    #[test]
    fn get_task_template_returns_builtin() {
        let runtime = OrbitRuntime::in_memory().expect("runtime");
        let tmpl = runtime.get_task_template("feature").expect("get");
        assert_eq!(tmpl.name, "feature");
        assert!(tmpl.builtin);
        assert_eq!(tmpl.task_type, TaskType::Feature);
        assert_eq!(tmpl.priority, TaskPriority::Medium);
    }

    #[test]
    fn get_task_template_not_found() {
        let runtime = OrbitRuntime::in_memory().expect("runtime");
        let result = runtime.get_task_template("nonexistent");
        assert!(matches!(result, Err(OrbitError::InvalidInput(_))));
    }

    #[test]
    fn user_defined_template_overrides_builtin() {
        let dir = tempfile::tempdir().expect("tempdir");
        let data_root = dir.path();
        let tpl_dir = data_root.join("task_templates");
        std::fs::create_dir_all(&tpl_dir).expect("create dir");
        std::fs::write(
            tpl_dir.join("feature.yaml"),
            "name: feature\ndescription: custom\ntask_type: chore\npriority: high\ndescription_template: custom desc\nplan_template: custom plan\ninstructions_template: custom instructions\n",
        )
        .expect("write");

        let runtime = OrbitRuntime::from_data_root(data_root).expect("runtime");
        let tmpl = runtime.get_task_template("feature").expect("get");
        assert!(!tmpl.builtin);
        assert_eq!(tmpl.task_type, TaskType::Chore);
        assert_eq!(tmpl.priority, TaskPriority::High);
    }

    #[test]
    fn user_defined_template_appears_in_list() {
        let dir = tempfile::tempdir().expect("tempdir");
        let data_root = dir.path();
        let tpl_dir = data_root.join("task_templates");
        std::fs::create_dir_all(&tpl_dir).expect("create dir");
        std::fs::write(
            tpl_dir.join("my-custom.yaml"),
            "name: my-custom\ndescription: my custom template\ndescription_template: desc\nplan_template: plan\ninstructions_template: instr\n",
        )
        .expect("write");

        let runtime = OrbitRuntime::from_data_root(data_root).expect("runtime");
        let templates = runtime.list_task_templates().expect("list");
        // 4 builtins + 1 user-defined
        assert_eq!(templates.len(), 5);
        let custom = templates
            .iter()
            .find(|t| t.name == "my-custom")
            .expect("custom");
        assert!(!custom.builtin);
    }

    #[test]
    fn list_templates_without_user_dir_returns_only_builtins() {
        let dir = tempfile::tempdir().expect("tempdir");
        // data_root exists, but task_templates/ subdirectory does NOT
        let runtime = OrbitRuntime::from_data_root(dir.path()).expect("runtime");
        let templates = runtime.list_task_templates().expect("list");
        assert_eq!(templates.len(), 4);
        assert!(templates.iter().all(|t| t.builtin));
    }
}
