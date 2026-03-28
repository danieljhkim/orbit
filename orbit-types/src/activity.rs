use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::OrbitId;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Activity {
    pub id: OrbitId,
    pub spec_type: String,
    pub description: String,
    #[serde(default)]
    pub input_schema_json: Value,
    pub output_schema_json: Value,
    #[serde(default)]
    pub spec_config: Value,
    /// Tool allowlist for agent_invoke activities. Empty means unrestricted.
    #[serde(default)]
    pub tools: Vec<String>,
    /// Program allowlist for `proc.spawn`. Empty means unrestricted.
    #[serde(default)]
    pub proc_allowed_programs: Vec<String>,
    #[serde(default)]
    pub workspace_path: Option<String>,
    #[serde(default)]
    pub created_by: Option<String>,
    pub is_active: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}
