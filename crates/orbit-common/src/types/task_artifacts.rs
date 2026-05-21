use std::collections::{BTreeMap, BTreeSet};
use std::path::{Component, Path};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::types::{
    ExternalRef, OrbitError, OrbitId, ReviewThreadStatus, TaskComplexity, TaskPriority, TaskStatus,
    TaskType, is_valid_adr_id, is_valid_friction_id, is_valid_learning_id,
};

pub const TASK_ARTIFACT_SCHEMA_VERSION: u32 = 1;
pub const ORB_TASK_ID_PREFIX: &str = "ORB-";
pub const ORB_TASK_ID_WIDTH: usize = 5;
pub const ORB_TASK_ID_MAX: u32 = 99_999;

pub const TASK_ENVELOPE_FILE_NAME: &str = "task.yaml";
pub const TASK_DESCRIPTION_FILE_NAME: &str = "description.md";
pub const TASK_ACCEPTANCE_FILE_NAME: &str = "acceptance.md";
pub const TASK_PLAN_FILE_NAME: &str = "plan.md";
pub const TASK_EXECUTION_SUMMARY_FILE_NAME: &str = "execution-summary.md";
pub const TASK_EVENTS_FILE_NAME: &str = "events.jsonl";
pub const TASK_COMMENTS_FILE_NAME: &str = "comments.jsonl";
pub const TASK_REVIEW_THREADS_DIR_NAME: &str = "review-threads";
pub const TASK_ARTIFACTS_DIR_NAME: &str = "artifacts";
pub const TASK_ARTIFACT_MANIFEST_FILE_NAME: &str = "manifest.yaml";
pub const TASK_ARTIFACT_FILES_DIR_NAME: &str = "files";

pub fn is_valid_orb_task_id(id: &str) -> bool {
    let Some(suffix) = id.strip_prefix(ORB_TASK_ID_PREFIX) else {
        return false;
    };
    suffix.len() == ORB_TASK_ID_WIDTH && suffix.chars().all(|character| character.is_ascii_digit())
}

pub fn validate_orb_task_id(id: &str) -> Result<(), OrbitError> {
    if is_valid_orb_task_id(id) {
        return Ok(());
    }
    Err(OrbitError::InvalidInput(format!(
        "task id '{id}' must match ORB-00000"
    )))
}

pub fn format_orb_task_id(value: u32) -> Result<String, OrbitError> {
    if value > ORB_TASK_ID_MAX {
        return Err(OrbitError::InvalidInput(format!(
            "task id value {value} exceeds maximum {ORB_TASK_ID_MAX}"
        )));
    }
    Ok(format!("{ORB_TASK_ID_PREFIX}{value:0ORB_TASK_ID_WIDTH$}"))
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct TaskEnvelopeV2 {
    pub schema_version: u32,
    pub id: OrbitId,
    pub title: String,
    pub status: TaskStatus,
    #[serde(rename = "type")]
    pub task_type: TaskType,
    pub priority: TaskPriority,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub complexity: Option<TaskComplexity>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub job_run_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub crew: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub relations: Vec<TaskRelation>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub context_files: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub external_refs: Vec<ExternalRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_by: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub planned_by: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub implemented_by: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl TaskEnvelopeV2 {
    pub fn validate(&self) -> Result<(), OrbitError> {
        validate_schema_version(self.schema_version, "task envelope")?;
        validate_orb_task_id(&self.id)?;
        if self.title.trim().is_empty() {
            return Err(OrbitError::InvalidInput(
                "task title must not be empty".to_string(),
            ));
        }
        validate_task_relations_for_source(&self.id, &self.relations, &[])
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[serde(rename_all = "snake_case")]
pub enum TaskRelationType {
    BlockedBy,
    ChildOf,
    SpawnedFrom,
    RegressionFrom,
    Supersedes,
    RelatedTo,
    Produces,
    Resolves,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct TaskRelation {
    #[serde(rename = "type")]
    pub relation_type: TaskRelationType,
    pub target: OrbitId,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct TaskRelationEdge {
    pub source: OrbitId,
    #[serde(rename = "type")]
    pub relation_type: TaskRelationType,
    pub target: OrbitId,
}

pub fn validate_task_relations_for_source(
    source_id: &str,
    relations: &[TaskRelation],
    existing_edges: &[TaskRelationEdge],
) -> Result<(), OrbitError> {
    // Existing edges are assumed acyclic; this check only rejects new edges that close a cycle.
    validate_orb_task_id(source_id)?;
    let mut seen = BTreeSet::new();
    for edge in existing_edges {
        validate_orb_task_id(&edge.source)?;
        validate_orb_task_id(&edge.target)?;
        if edge.source == edge.target {
            return Err(OrbitError::InvalidInput(format!(
                "task relation from '{}' must not target itself",
                edge.source
            )));
        }
    }
    for relation in relations {
        validate_target_for(relation.relation_type, &relation.target)?;
        if relation.target == source_id {
            return Err(OrbitError::InvalidInput(format!(
                "task relation from '{source_id}' must not target itself"
            )));
        }
        let key = (relation.relation_type, relation.target.as_str());
        if !seen.insert(key) {
            return Err(OrbitError::InvalidInput(format!(
                "duplicate task relation {:?} -> '{}'",
                relation.relation_type, relation.target
            )));
        }
    }

    for relation in relations {
        if let Some(family) = cyclic_relation_family(relation.relation_type) {
            let mut graph = relation_graph_for_family(family, existing_edges);
            for candidate in relations {
                if cyclic_relation_family(candidate.relation_type) == Some(family) {
                    graph
                        .entry(source_id.to_string())
                        .or_default()
                        .insert(candidate.target.clone());
                }
            }
            if reaches(&graph, &relation.target, source_id) {
                return Err(OrbitError::InvalidInput(format!(
                    "task relation {:?} from '{}' to '{}' would create a cycle",
                    relation.relation_type, source_id, relation.target
                )));
            }
        }
    }

    Ok(())
}

fn validate_target_for(relation_type: TaskRelationType, target: &str) -> Result<(), OrbitError> {
    match relation_type {
        TaskRelationType::Produces | TaskRelationType::Resolves => {
            if is_valid_orb_task_id(target)
                || is_valid_friction_id(target)
                || is_valid_learning_id(target)
                || is_valid_adr_id(target)
            {
                return Ok(());
            }
            Err(OrbitError::InvalidInput(format!(
                "relation target '{target}' must be an ORB-/F-/L-/ADR- id"
            )))
        }
        TaskRelationType::BlockedBy
        | TaskRelationType::ChildOf
        | TaskRelationType::SpawnedFrom
        | TaskRelationType::RegressionFrom
        | TaskRelationType::Supersedes
        | TaskRelationType::RelatedTo => validate_orb_task_id(target),
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct TaskEventRowV2 {
    pub schema_version: u32,
    pub event_id: String,
    pub at: DateTime<Utc>,
    pub by: String,
    #[serde(rename = "type")]
    pub event_type: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from_status: Option<TaskStatus>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub to_status: Option<TaskStatus>,
}

impl TaskEventRowV2 {
    pub fn validate(&self) -> Result<(), OrbitError> {
        validate_schema_version(self.schema_version, "task event row")?;
        if self.event_id.trim().is_empty() {
            return Err(OrbitError::InvalidInput(
                "task event id must not be empty".to_string(),
            ));
        }
        if self.by.trim().is_empty() {
            return Err(OrbitError::InvalidInput(
                "task event actor must not be empty".to_string(),
            ));
        }
        if self.event_type.trim().is_empty() {
            return Err(OrbitError::InvalidInput(
                "task event type must not be empty".to_string(),
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct TaskCommentRowV2 {
    pub schema_version: u32,
    pub comment_id: String,
    pub at: DateTime<Utc>,
    pub by: String,
    pub body: String,
}

impl TaskCommentRowV2 {
    pub fn validate(&self) -> Result<(), OrbitError> {
        validate_schema_version(self.schema_version, "task comment row")?;
        if self.comment_id.trim().is_empty() {
            return Err(OrbitError::InvalidInput(
                "task comment id must not be empty".to_string(),
            ));
        }
        if self.by.trim().is_empty() {
            return Err(OrbitError::InvalidInput(
                "task comment actor must not be empty".to_string(),
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ReviewThreadMetadataV2 {
    pub schema_version: u32,
    pub thread_id: String,
    pub status: ReviewThreadStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub line: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub github_thread_id: Option<u64>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub messages: Vec<ReviewThreadMessageMetadataV2>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl ReviewThreadMetadataV2 {
    pub fn validate(&self) -> Result<(), OrbitError> {
        validate_schema_version(self.schema_version, "review thread metadata")?;
        if self.thread_id.trim().is_empty() {
            return Err(OrbitError::InvalidInput(
                "review thread id must not be empty".to_string(),
            ));
        }
        if let Some(path) = &self.path {
            validate_relative_artifact_path(path)?;
        }
        for message in &self.messages {
            message.validate()?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ReviewThreadMessageMetadataV2 {
    pub message_id: String,
    pub at: DateTime<Utc>,
    pub by: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub github_comment_id: Option<u64>,
}

impl ReviewThreadMessageMetadataV2 {
    pub fn validate(&self) -> Result<(), OrbitError> {
        if self.message_id.trim().is_empty() {
            return Err(OrbitError::InvalidInput(
                "review thread message id must not be empty".to_string(),
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ArtifactManifestV2 {
    pub schema_version: u32,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub files: Vec<ArtifactManifestFileV2>,
}

impl ArtifactManifestV2 {
    pub fn validate(&self) -> Result<(), OrbitError> {
        validate_schema_version(self.schema_version, "artifact manifest")?;
        for file in &self.files {
            file.validate()?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ArtifactManifestFileV2 {
    pub path: String,
    pub blob: String,
    /// Lowercase hex SHA-256 digest; writers should format bytes with `{:x}`.
    pub sha256: String,
    pub media_type: String,
    pub size_bytes: u64,
    pub created_by: String,
    pub created_at: DateTime<Utc>,
}

impl ArtifactManifestFileV2 {
    pub fn validate(&self) -> Result<(), OrbitError> {
        validate_relative_artifact_path(&self.path)?;
        validate_relative_artifact_path(&self.blob)?;
        if !is_sha256_hex(&self.sha256) {
            return Err(OrbitError::InvalidInput(
                "artifact sha256 must be 64 lowercase hexadecimal characters".to_string(),
            ));
        }
        if self.media_type.trim().is_empty() {
            return Err(OrbitError::InvalidInput(
                "artifact media type must not be empty".to_string(),
            ));
        }
        if self.created_by.trim().is_empty() {
            return Err(OrbitError::InvalidInput(
                "artifact created_by must not be empty".to_string(),
            ));
        }
        Ok(())
    }
}

pub fn validate_relative_artifact_path(path: &str) -> Result<(), OrbitError> {
    let trimmed = path.trim();
    if trimmed.is_empty() {
        return Err(OrbitError::InvalidInput(
            "artifact path must not be empty".to_string(),
        ));
    }
    let path = Path::new(trimmed);
    if path.is_absolute() {
        return Err(OrbitError::InvalidInput(format!(
            "artifact path '{trimmed}' must be relative"
        )));
    }
    if trimmed.contains('\\') {
        return Err(OrbitError::InvalidInput(format!(
            "artifact path '{trimmed}' must use slash separators"
        )));
    }
    let mut has_normal_component = false;
    for component in path.components() {
        match component {
            Component::ParentDir => {
                return Err(OrbitError::InvalidInput(format!(
                    "artifact path '{trimmed}' must not contain '..'"
                )));
            }
            Component::RootDir | Component::Prefix(_) => {
                return Err(OrbitError::InvalidInput(format!(
                    "artifact path '{trimmed}' must be relative"
                )));
            }
            Component::CurDir => {
                // Manifest paths are stored canonical; writers may strip a leading "./" before validation.
                return Err(OrbitError::InvalidInput(format!(
                    "artifact path '{trimmed}' must not contain '.' components"
                )));
            }
            Component::Normal(_) => {
                has_normal_component = true;
            }
        }
    }
    if !has_normal_component {
        return Err(OrbitError::InvalidInput(
            "artifact path must contain at least one path component".to_string(),
        ));
    }
    Ok(())
}

fn is_sha256_hex(value: &str) -> bool {
    value.len() == 64
        && value
            .as_bytes()
            .iter()
            .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
}

fn validate_schema_version(schema_version: u32, label: &str) -> Result<(), OrbitError> {
    if schema_version == TASK_ARTIFACT_SCHEMA_VERSION {
        return Ok(());
    }
    Err(OrbitError::InvalidInput(format!(
        "{label} schema_version must be {TASK_ARTIFACT_SCHEMA_VERSION}, got {schema_version}"
    )))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RelationCycleFamily {
    Blocking,
    Hierarchy,
}

fn cyclic_relation_family(relation_type: TaskRelationType) -> Option<RelationCycleFamily> {
    match relation_type {
        TaskRelationType::BlockedBy => Some(RelationCycleFamily::Blocking),
        TaskRelationType::ChildOf => Some(RelationCycleFamily::Hierarchy),
        // Temporal and associative relation types are queryable metadata, not reachability families.
        TaskRelationType::SpawnedFrom => None,
        TaskRelationType::RegressionFrom
        | TaskRelationType::Supersedes
        | TaskRelationType::RelatedTo
        | TaskRelationType::Produces
        | TaskRelationType::Resolves => None,
    }
}

fn relation_graph_for_family(
    family: RelationCycleFamily,
    edges: &[TaskRelationEdge],
) -> BTreeMap<OrbitId, BTreeSet<OrbitId>> {
    let mut graph = BTreeMap::new();
    for edge in edges {
        if cyclic_relation_family(edge.relation_type) == Some(family) {
            graph
                .entry(edge.source.clone())
                .or_insert_with(BTreeSet::new)
                .insert(edge.target.clone());
        }
    }
    graph
}

fn reaches(graph: &BTreeMap<OrbitId, BTreeSet<OrbitId>>, start: &str, target: &str) -> bool {
    let mut stack = vec![start.to_string()];
    let mut seen = BTreeSet::new();
    while let Some(current) = stack.pop() {
        if current == target {
            return true;
        }
        if !seen.insert(current.clone()) {
            continue;
        }
        if let Some(next) = graph.get(&current) {
            stack.extend(next.iter().cloned());
        }
    }
    false
}
