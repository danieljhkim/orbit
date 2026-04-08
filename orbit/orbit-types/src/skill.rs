use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::Role;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Skill {
    pub schema_version: u8,
    pub name: String,
    pub description: Option<String>,
    pub instructions: String,
    pub context_files: Vec<String>,
    pub allowed_tools: Vec<String>,
    pub role: Role,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}
