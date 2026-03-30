use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::OrbitId;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Activity {
    /// Unique identifier for this activity definition.
    pub id: OrbitId,
    /// Activity type discriminator such as `agent_invoke`, `shell`, or `job`.
    pub spec_type: String,
    /// Human-readable summary of what this activity does.
    pub description: String,
    /// JSON Schema used to validate the activity input payload.
    #[serde(default)]
    pub input_schema_json: Value,
    /// JSON Schema used to validate the activity result payload.
    #[serde(default)]
    pub output_schema_json: Value,
    /// Type-specific configuration payload for the selected activity kind.
    #[serde(default)]
    pub spec_config: Value,
    /// Tool allowlist for agent_invoke activities. Empty means unrestricted.
    #[serde(default)]
    pub tools: Vec<String>,
    /// Program allowlist for `proc.spawn`. Empty means unrestricted.
    #[serde(default)]
    pub proc_allowed_programs: Vec<String>,
    /// Optional workspace path override used when resolving execution context.
    #[serde(default)]
    pub workspace_path: Option<String>,
    /// Actor identity that created this activity definition, when recorded.
    #[serde(default)]
    pub created_by: Option<String>,
    /// Whether this activity is enabled for execution.
    pub is_active: bool,
    /// Timestamp when this activity definition was first created.
    pub created_at: DateTime<Utc>,
    /// Timestamp of the most recent activity definition update.
    pub updated_at: DateTime<Utc>,
}
