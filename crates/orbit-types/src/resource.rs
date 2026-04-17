use std::collections::HashMap;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::{
    ApprovalPolicy, FilesystemPolicy, JobScheduleState, JobStep, NetworkPolicy, ProcessPolicy,
    ToolPolicy, default_job_max_active_runs, default_max_iterations,
};

pub const RESOURCE_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum ResourceKind {
    Job,
    Activity,
    Policy,
    Executor,
}

impl std::fmt::Display for ResourceKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ResourceKind::Job => write!(f, "Job"),
            ResourceKind::Activity => write!(f, "Activity"),
            ResourceKind::Policy => write!(f, "Policy"),
            ResourceKind::Executor => write!(f, "Executor"),
        }
    }
}

impl std::str::FromStr for ResourceKind {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "job" | "jobs" => Ok(ResourceKind::Job),
            "activity" | "activities" => Ok(ResourceKind::Activity),
            "policy" | "policies" => Ok(ResourceKind::Policy),
            "executor" | "executors" => Ok(ResourceKind::Executor),
            _ => Err(format!("unknown resource kind: {s}")),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ResourceMetadata {
    pub name: String,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub labels: HashMap<String, String>,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub annotations: HashMap<String, String>,
}

impl ResourceMetadata {
    pub fn named(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            labels: HashMap::new(),
            annotations: HashMap::new(),
        }
    }
}

/// Header-only parse for routing `orbit apply` to the right store.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResourceHeader {
    #[serde(rename = "schemaVersion")]
    pub schema_version: u32,
    pub kind: ResourceKind,
    pub metadata: ResourceMetadata,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ResourceEnvelope<T> {
    #[serde(rename = "schemaVersion")]
    pub schema_version: u32,
    pub kind: ResourceKind,
    pub metadata: ResourceMetadata,
    pub spec: T,
}

impl<T> ResourceEnvelope<T> {
    pub fn new(kind: ResourceKind, name: impl Into<String>, spec: T) -> Self {
        Self {
            schema_version: RESOURCE_SCHEMA_VERSION,
            kind,
            metadata: ResourceMetadata::named(name),
            spec,
        }
    }

    pub fn header(&self) -> ResourceHeader {
        ResourceHeader {
            schema_version: self.schema_version,
            kind: self.kind.clone(),
            metadata: self.metadata.clone(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct JobResourceSpec {
    pub state: JobScheduleState,
    #[serde(default)]
    pub default_input: Option<Value>,
    #[serde(default = "default_job_max_active_runs")]
    pub max_active_runs: u32,
    #[serde(default = "default_max_iterations")]
    pub max_iterations: u32,
    pub steps: Vec<JobStep>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub policy: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ActivityResourceSpec {
    pub spec_type: String,
    pub description: String,
    #[serde(default)]
    pub input_schema_json: Value,
    #[serde(default)]
    pub output_schema_json: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub executor: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_by: Option<String>,
    #[serde(default = "default_true", skip_serializing_if = "is_true")]
    pub is_active: bool,
    #[serde(flatten)]
    pub spec_config: Map<String, Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PolicyResourceSpec {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub filesystem: Option<FilesystemPolicy>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub network: Option<NetworkPolicy>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub process: Option<ProcessPolicy>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub approval: Option<ApprovalPolicy>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tools: Option<ToolPolicy>,
    #[serde(default = "Utc::now")]
    pub created_at: DateTime<Utc>,
    #[serde(default = "Utc::now")]
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ExecutorResourceSpec {
    pub executor_type: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stdout_format: Option<String>,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub models: HashMap<String, String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_seconds: Option<u64>,
    #[serde(default)]
    pub env: HashMap<String, String>,
    #[serde(default = "Utc::now")]
    pub created_at: DateTime<Utc>,
    #[serde(default = "Utc::now")]
    pub updated_at: DateTime<Utc>,
}

pub type JobResource = ResourceEnvelope<JobResourceSpec>;
pub type ActivityResource = ResourceEnvelope<ActivityResourceSpec>;
pub type PolicyResource = ResourceEnvelope<PolicyResourceSpec>;
pub type ExecutorResource = ResourceEnvelope<ExecutorResourceSpec>;

fn default_true() -> bool {
    true
}

fn is_true(value: &bool) -> bool {
    *value
}
