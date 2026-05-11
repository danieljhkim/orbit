use std::collections::HashMap;
use std::fmt;
use std::path::PathBuf;
use std::str::FromStr;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkspaceStatus {
    Active,
    Invalid,
}

impl fmt::Display for WorkspaceStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            WorkspaceStatus::Active => write!(f, "active"),
            WorkspaceStatus::Invalid => write!(f, "invalid"),
        }
    }
}

impl FromStr for WorkspaceStatus {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "active" => Ok(WorkspaceStatus::Active),
            "invalid" => Ok(WorkspaceStatus::Invalid),
            other => Err(format!("unknown workspace status: {other}")),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Workspace {
    pub id: String,
    pub name: String,
    pub root: PathBuf,
    pub orbit_dir: PathBuf,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub git_remote: Option<String>,
    #[serde(default = "default_base_branch")]
    pub base_branch: String,
    #[serde(default = "default_status")]
    pub status: WorkspaceStatus,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

fn default_base_branch() -> String {
    "main".to_string()
}

fn default_status() -> WorkspaceStatus {
    WorkspaceStatus::Active
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct WorkspaceRegistry {
    #[serde(default)]
    pub workspaces: Vec<Workspace>,
    #[serde(default)]
    pub path_overrides: HashMap<PathBuf, String>,
}

/// Derived directory layout for a workspace.
///
/// All sub-paths are derived from `orbit_dir` in the constructor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspacePaths {
    pub repo_root: PathBuf,
    pub orbit_dir: PathBuf,
    pub global_dir: PathBuf,
    pub resources_dir: PathBuf,
    pub state_dir: PathBuf,
    pub tasks_dir: PathBuf,
    pub adrs_dir: PathBuf,
    pub knowledge_dir: PathBuf,
    pub activities_dir: PathBuf,
    pub jobs_dir: PathBuf,
    pub skills_dir: PathBuf,
    pub executors_dir: PathBuf,
    pub policies_dir: PathBuf,
    pub audit_dir: PathBuf,
    pub job_runs_dir: PathBuf,
    pub logs_dir: PathBuf,
    pub scoreboard_dir: PathBuf,
    pub diagnostics_dir: PathBuf,
    pub worktrees_dir: PathBuf,
}

impl WorkspacePaths {
    pub fn new(repo_root: PathBuf, orbit_dir: PathBuf, global_dir: PathBuf) -> Self {
        let resources_dir = orbit_dir.join("resources");
        let state_dir = orbit_dir.join("state");
        Self {
            resources_dir: resources_dir.clone(),
            state_dir: state_dir.clone(),
            tasks_dir: orbit_dir.join("tasks"),
            adrs_dir: orbit_dir.join("adrs"),
            knowledge_dir: orbit_dir.join("knowledge"),
            activities_dir: resources_dir.join("activities"),
            jobs_dir: resources_dir.join("jobs"),
            skills_dir: resources_dir.join("skills"),
            executors_dir: resources_dir.join("executors"),
            policies_dir: resources_dir.join("policies"),
            audit_dir: state_dir.join("audit"),
            job_runs_dir: state_dir.join("job-runs"),
            logs_dir: state_dir.join("logs"),
            scoreboard_dir: state_dir.join("scoreboard"),
            diagnostics_dir: state_dir.join("diagnostics"),
            worktrees_dir: state_dir.join("worktrees"),
            repo_root,
            orbit_dir,
            global_dir,
        }
    }
}
