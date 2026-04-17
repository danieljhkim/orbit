use chrono::{DateTime, Utc};
use orbit_types::{
    ActorIdentity, OrbitError, OrbitId, ReviewThread, TaskComment, TaskComplexity, TaskPriority,
    TaskType,
};
use serde::{Deserialize, Serialize};
use serde_yaml::{Mapping, Value as YamlValue};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct TaskFileDocument {
    #[serde(rename = "schema_version")]
    pub(super) schema_version: u8,
    pub(super) id: String,
    #[serde(default)]
    pub(super) parent_id: Option<OrbitId>,
    #[serde(rename = "type", default = "default_task_type")]
    pub(super) task_type: TaskType,
    pub(super) priority: TaskPriority,
    #[serde(default)]
    pub(super) complexity: Option<TaskComplexity>,
    pub(super) title: String,
    #[serde(default)]
    pub(super) description: String,
    #[serde(default)]
    pub(super) acceptance_criteria: Vec<String>,
    #[serde(default)]
    pub(super) context_files: Vec<String>,
    #[serde(default)]
    pub(super) workspace_path: Option<String>,
    #[serde(default)]
    pub(super) repo_root: Option<String>,
    #[serde(default)]
    pub(super) created_by: Option<String>,
    #[serde(default)]
    pub(super) planned_by: Option<String>,
    #[serde(default)]
    pub(super) implemented_by: Option<String>,
    #[serde(default)]
    pub(super) agent: Option<String>,
    #[serde(default)]
    pub(super) model: Option<String>,
    /// Legacy field — kept for deserialization of existing YAML files only.
    #[serde(default, skip_serializing)]
    pub(super) actor_identity: ActorIdentity,
    /// Legacy field — kept for deserialization of existing YAML files only.
    #[serde(default, skip_serializing)]
    pub(super) assigned_to: Option<String>,
    /// Legacy field — kept for deserialization of existing YAML files only.
    #[serde(default, skip_serializing)]
    pub(super) proposed_by: Option<String>,
    #[serde(default)]
    pub(super) pr_number: Option<String>,
    #[serde(default)]
    pub(super) pr_status: Option<String>,
    #[serde(default)]
    pub(super) source_task_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) batch_id: Option<String>,
    pub(super) created_at: DateTime<Utc>,
    pub(super) updated_at: DateTime<Utc>,
    #[serde(default)]
    pub(super) history: Vec<orbit_types::TaskHistoryEntry>,
    #[serde(default)]
    pub(super) comments: Vec<TaskComment>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(super) review_threads: Vec<ReviewThread>,
}

pub(super) fn default_task_type() -> TaskType {
    TaskType::Task
}

pub(super) fn serialize_task_doc_yaml(doc: &TaskFileDocument) -> Result<String, OrbitError> {
    let mut yaml = String::new();
    yaml.push_str(&yaml_field("schema_version", &doc.schema_version)?);

    yaml.push_str(&yaml_section("identity"));
    yaml.push_str(&yaml_field("id", &doc.id)?);
    if let Some(ref parent_id) = doc.parent_id {
        yaml.push_str(&yaml_field("parent_id", parent_id)?);
    }
    yaml.push_str(&yaml_field("type", &doc.task_type)?);
    yaml.push_str(&yaml_field("priority", &doc.priority)?);
    if let Some(complexity) = doc.complexity {
        yaml.push_str(&yaml_field("complexity", &complexity)?);
    }

    yaml.push_str(&yaml_section("content"));
    yaml.push_str(&yaml_field("title", &doc.title)?);
    yaml.push_str(&yaml_field("description", &doc.description)?);
    yaml.push_str(&yaml_field(
        "acceptance_criteria",
        &doc.acceptance_criteria,
    )?);

    yaml.push_str(&yaml_section("context"));
    yaml.push_str(&yaml_field("context_files", &doc.context_files)?);
    yaml.push_str(&yaml_field("workspace_path", &doc.workspace_path)?);
    yaml.push_str(&yaml_field("repo_root", &doc.repo_root)?);

    yaml.push_str(&yaml_section("ownership"));
    yaml.push_str(&yaml_field("created_by", &doc.created_by)?);
    yaml.push_str(&yaml_field("planned_by", &doc.planned_by)?);
    yaml.push_str(&yaml_field("implemented_by", &doc.implemented_by)?);

    yaml.push_str(&yaml_section("implementation"));
    yaml.push_str(&yaml_field("agent", &doc.agent)?);
    yaml.push_str(&yaml_field("model", &doc.model)?);
    yaml.push_str(&yaml_field("pr_number", &doc.pr_number)?);
    yaml.push_str(&yaml_field("pr_status", &doc.pr_status)?);

    if doc.source_task_id.is_some() || doc.batch_id.is_some() {
        yaml.push_str(&yaml_section("attribution"));
        yaml.push_str(&yaml_field("source_task_id", &doc.source_task_id)?);
        if doc.batch_id.is_some() {
            yaml.push_str(&yaml_field("batch_id", &doc.batch_id)?);
        }
    }

    yaml.push_str(&yaml_section("timestamps"));
    yaml.push_str(&yaml_field("created_at", &doc.created_at)?);
    yaml.push_str(&yaml_field("updated_at", &doc.updated_at)?);

    yaml.push_str(&yaml_section("audit trail"));
    yaml.push_str(&yaml_field("history", &doc.history)?);
    yaml.push_str(&yaml_field("comments", &doc.comments)?);

    if !doc.review_threads.is_empty() {
        yaml.push_str(&yaml_section("review"));
        yaml.push_str(&yaml_field("review_threads", &doc.review_threads)?);
    }

    Ok(yaml)
}

fn yaml_section(name: &str) -> String {
    format!("\n# ---- {name} ----\n")
}

fn yaml_field(key: &str, value: &impl Serialize) -> Result<String, OrbitError> {
    let mut mapping = Mapping::new();
    mapping.insert(
        YamlValue::String(key.to_string()),
        serde_yaml::to_value(value).map_err(|e| OrbitError::Store(e.to_string()))?,
    );
    serde_yaml::to_string(&mapping).map_err(|e| OrbitError::Store(e.to_string()))
}
