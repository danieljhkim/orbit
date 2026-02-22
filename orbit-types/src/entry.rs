use std::fmt::{Display, Formatter};
use std::str::FromStr;

use chrono::{DateTime, Utc};
use clap::ValueEnum;
use serde::{Deserialize, Serialize};

use crate::OrbitId;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash, ValueEnum)]
#[serde(rename_all = "snake_case")]
pub enum EntityType {
    Task,
    Job,
    Watch,
    Session,
    Workflow,
}

impl Display for EntityType {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            EntityType::Task => write!(f, "task"),
            EntityType::Job => write!(f, "job"),
            EntityType::Watch => write!(f, "watch"),
            EntityType::Session => write!(f, "session"),
            EntityType::Workflow => write!(f, "workflow"),
        }
    }
}

impl FromStr for EntityType {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "task" => Ok(EntityType::Task),
            "job" => Ok(EntityType::Job),
            "watch" => Ok(EntityType::Watch),
            "session" => Ok(EntityType::Session),
            "workflow" => Ok(EntityType::Workflow),
            other => Err(format!("unknown entity type: {other}")),
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash, ValueEnum)]
#[serde(rename_all = "snake_case")]
pub enum EntryType {
    Comment,
    Reasoning,
    Decision,
    Artifact,
    System,
}

impl Display for EntryType {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            EntryType::Comment => write!(f, "comment"),
            EntryType::Reasoning => write!(f, "reasoning"),
            EntryType::Decision => write!(f, "decision"),
            EntryType::Artifact => write!(f, "artifact"),
            EntryType::System => write!(f, "system"),
        }
    }
}

impl FromStr for EntryType {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "comment" => Ok(EntryType::Comment),
            "reasoning" => Ok(EntryType::Reasoning),
            "decision" => Ok(EntryType::Decision),
            "artifact" => Ok(EntryType::Artifact),
            "system" => Ok(EntryType::System),
            other => Err(format!("unknown entry type: {other}")),
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash, ValueEnum)]
#[serde(rename_all = "snake_case")]
pub enum AuthorType {
    Human,
    Agent,
    System,
}

impl Display for AuthorType {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            AuthorType::Human => write!(f, "human"),
            AuthorType::Agent => write!(f, "agent"),
            AuthorType::System => write!(f, "system"),
        }
    }
}

impl FromStr for AuthorType {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "human" => Ok(AuthorType::Human),
            "agent" => Ok(AuthorType::Agent),
            "system" => Ok(AuthorType::System),
            other => Err(format!("unknown author type: {other}")),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Entry {
    pub id: OrbitId,
    pub entity_type: EntityType,
    pub entity_id: OrbitId,
    pub session_id: Option<OrbitId>,
    pub sequence_number: i64,
    pub entry_type: EntryType,
    pub author_type: AuthorType,
    pub author_id: String,
    pub author_model: Option<String>,
    pub body: String,
    pub created_at: DateTime<Utc>,
}

impl Display for Entry {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}\t{}\t{}\t{}",
            self.sequence_number, self.entry_type, self.author_type, self.body
        )
    }
}
