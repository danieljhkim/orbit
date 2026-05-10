use chrono::{DateTime, Utc};
use orbit_common::types::{
    ActorIdentity, ExternalRef, OrbitError, OrbitId, ReviewThread, TaskComment, TaskComplexity,
    TaskPriority, TaskType, push_external_ref_if_missing,
};
use serde::{Deserialize, Serialize, de};
use serde_yaml::{Mapping, Value as YamlValue};

#[derive(Debug, Clone, Serialize)]
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
    pub(super) dependencies: Vec<OrbitId>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(super) tags: Vec<String>,
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
    pub(super) pr_status: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(super) external_refs: Vec<ExternalRef>,
    #[serde(default)]
    pub(super) source_task_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) batch_id: Option<String>,
    pub(super) created_at: DateTime<Utc>,
    pub(super) updated_at: DateTime<Utc>,
    #[serde(default)]
    pub(super) history: Vec<orbit_common::types::TaskHistoryEntry>,
    #[serde(default)]
    pub(super) comments: Vec<TaskComment>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(super) review_threads: Vec<ReviewThread>,
}

impl<'de> Deserialize<'de> for TaskFileDocument {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct RawTaskFileDocument {
            #[serde(rename = "schema_version")]
            schema_version: u8,
            id: String,
            #[serde(default)]
            parent_id: Option<OrbitId>,
            #[serde(rename = "type", default = "default_task_type")]
            task_type: TaskType,
            priority: TaskPriority,
            #[serde(default)]
            complexity: Option<TaskComplexity>,
            title: String,
            #[serde(default)]
            description: String,
            #[serde(default)]
            acceptance_criteria: Vec<String>,
            #[serde(default)]
            dependencies: Vec<OrbitId>,
            #[serde(default)]
            tags: Vec<String>,
            #[serde(default)]
            context_files: Vec<String>,
            #[serde(default)]
            workspace_path: Option<String>,
            #[serde(default)]
            repo_root: Option<String>,
            #[serde(default)]
            created_by: Option<String>,
            #[serde(default)]
            planned_by: Option<String>,
            #[serde(default)]
            implemented_by: Option<String>,
            #[serde(default)]
            agent: Option<String>,
            #[serde(default)]
            model: Option<String>,
            #[serde(default)]
            actor_identity: ActorIdentity,
            #[serde(default)]
            assigned_to: Option<String>,
            #[serde(default)]
            proposed_by: Option<String>,
            #[serde(default)]
            pr_number: Option<String>,
            #[serde(default)]
            pr_status: Option<String>,
            #[serde(default)]
            external_refs: Vec<ExternalRef>,
            #[serde(default)]
            source_task_id: Option<String>,
            #[serde(default)]
            batch_id: Option<String>,
            created_at: DateTime<Utc>,
            updated_at: DateTime<Utc>,
            #[serde(default)]
            history: Vec<orbit_common::types::TaskHistoryEntry>,
            #[serde(default)]
            comments: Vec<TaskComment>,
            #[serde(default)]
            review_threads: Vec<ReviewThread>,
        }

        let raw = RawTaskFileDocument::deserialize(deserializer)?;
        let mut external_refs = raw.external_refs;
        if let Some(pr_number) = raw
            .pr_number
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
        {
            // Legacy YAML read compatibility only: `pr_number` is task identity
            // linkage and now normalizes into `external_refs`. `pr_status`
            // remains a separate workflow-state field.
            push_external_ref_if_missing(
                &mut external_refs,
                ExternalRef::github_pr(pr_number).map_err(de::Error::custom)?,
            );
        }

        Ok(TaskFileDocument {
            schema_version: raw.schema_version,
            id: raw.id,
            parent_id: raw.parent_id,
            task_type: raw.task_type,
            priority: raw.priority,
            complexity: raw.complexity,
            title: raw.title,
            description: raw.description,
            acceptance_criteria: raw.acceptance_criteria,
            dependencies: raw.dependencies,
            tags: raw.tags,
            context_files: raw.context_files,
            workspace_path: raw.workspace_path,
            repo_root: raw.repo_root,
            created_by: raw.created_by,
            planned_by: raw.planned_by,
            implemented_by: raw.implemented_by,
            agent: raw.agent,
            model: raw.model,
            actor_identity: raw.actor_identity,
            assigned_to: raw.assigned_to,
            proposed_by: raw.proposed_by,
            pr_status: raw.pr_status,
            external_refs,
            source_task_id: raw.source_task_id,
            batch_id: raw.batch_id,
            created_at: raw.created_at,
            updated_at: raw.updated_at,
            history: raw.history,
            comments: raw.comments,
            review_threads: raw.review_threads,
        })
    }
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
    yaml.push_str(&yaml_field("dependencies", &doc.dependencies)?);
    if !doc.tags.is_empty() {
        yaml.push_str(&yaml_field("tags", &doc.tags)?);
    }

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
    yaml.push_str(&yaml_field("pr_status", &doc.pr_status)?);
    if !doc.external_refs.is_empty() {
        yaml.push_str(&yaml_field("external_refs", &doc.external_refs)?);
    }

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
