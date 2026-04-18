use serde::{Deserialize, Deserializer, Serialize};
use serde_json::Value;

use crate::{JobScheduleState, JobTargetType, OrbitId};

/// v2 Job definition. See §5.2 for block-form pipeline refs.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct JobV2 {
    pub state: JobScheduleState,
    #[serde(default)]
    pub default_input: Option<Value>,
    #[serde(default = "default_max_active_runs")]
    pub max_active_runs: u32,
    pub steps: Vec<JobV2Step>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub policy: Option<String>,
}

/// A step in a v2 job. Sequential by default (Phase 3 adds parallel/DAG).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct JobV2Step {
    pub id: String,
    pub target_type: JobTargetType,
    pub target_id: OrbitId,
    /// Optional structured or literal input for this step.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_input: Option<Value>,
    #[serde(default)]
    pub timeout_seconds: u64,
}

/// Structured pipeline-context reference (§5.2, §12 Q8).
///
/// Accepts either a block form
/// ```yaml
/// default_input:
///   from: steps.dispatch.output.task_id
/// ```
/// or a literal string (treated as a Handlebars template scalar, compatible
/// with v1 `{{ steps.<id>.output.<field> }}` syntax).
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(untagged)]
pub enum PipelineRef {
    /// Structured reference into the pipeline context.
    Block { from: String },
    /// Literal / template string (v1-compatible).
    Literal(String),
}

impl<'de> Deserialize<'de> for PipelineRef {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        // Peek as a generic Value; then decide the shape.
        let v = Value::deserialize(deserializer)?;
        if let Some(s) = v.as_str() {
            return Ok(PipelineRef::Literal(s.to_string()));
        }
        if let Some(obj) = v.as_object() {
            if let Some(from) = obj.get("from").and_then(|x| x.as_str()) {
                return Ok(PipelineRef::Block {
                    from: from.to_string(),
                });
            }
        }
        Err(serde::de::Error::custom(
            "PipelineRef must be either a string or a block with `from:`",
        ))
    }
}

const fn default_max_active_runs() -> u32 {
    1
}
