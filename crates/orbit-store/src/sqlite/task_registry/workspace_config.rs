use std::fs;
use std::path::{Path, PathBuf};

use orbit_common::types::OrbitError;
use orbit_common::utility::fs::atomic_write_text;
use serde::{Deserialize, Serialize};

use super::CONFIG_SCHEMA_VERSION;
use super::types::{LearningDeliveryConfig, WorkspaceConfig};
use super::workspace_id::{sanitize_slug, validate_workspace_id, workspace_id_candidate};

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct WorkspaceConfigDoc {
    schema_version: u32,
    workspace_id: String,
    #[serde(default)]
    learnings: LearningDeliveryConfigDoc,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct LearningDeliveryConfigDoc {
    #[serde(default = "default_learning_tag_vocabulary")]
    tag_vocabulary: Vec<String>,
    #[serde(default = "default_upfront_learning_injection_cap")]
    upfront_injection_cap: usize,
}

impl Default for LearningDeliveryConfigDoc {
    fn default() -> Self {
        Self {
            tag_vocabulary: default_learning_tag_vocabulary(),
            upfront_injection_cap: default_upfront_learning_injection_cap(),
        }
    }
}

pub const DEFAULT_UPFRONT_LEARNING_INJECTION_CAP: usize = 5;

/// Canonical starter vocabulary. Workspaces commit their effective copy in
/// `.orbit/config.yaml`; this list makes pre-vocabulary configs upgrade
/// compatibly and seeds new workspaces with Orbit's curated terms.
pub const DEFAULT_LEARNING_TAG_VOCABULARY: &[&str] = &[
    "activity-job",
    "adr",
    "agents",
    "architecture",
    "attribution",
    "audit",
    "auto-task:qa-sweep",
    "boundaries",
    "branching",
    "bridge-contract",
    "ci",
    "cli",
    "codex",
    "config",
    "config-parsing",
    "connector",
    "cowork",
    "crate-layering",
    "croner",
    "dashboard",
    "dashboard-js-split",
    "dashboard-static-assets",
    "default-assets",
    "dependencies",
    "distribution",
    "dk-server-1",
    "docs",
    "dotfile",
    "embeddings",
    "environment",
    "error-handling",
    "extractors",
    "fallback",
    "fixtures",
    "fts5",
    "github",
    "git-conventions",
    "gitignore",
    "graph-tools",
    "hooks",
    "host-registry",
    "identity",
    "independent-review",
    "injection",
    "integrity",
    "job-runs",
    "learning",
    "learning-reminders",
    "learnings",
    "lifecycle",
    "locking",
    "macos",
    "mcp",
    "mcp-bridge",
    "mcp-policy",
    "metrics",
    "migration",
    "migrations",
    "multi-workspace",
    "no-diff-expected",
    "observability",
    "operations",
    "orbit-cli",
    "orbit-config",
    "orbit-core",
    "orbit-graph",
    "orbit-mcp",
    "orbit-root",
    "orbit-store",
    "orbit-worktree",
    "panic-handling",
    "path-extension",
    "performance",
    "pipeline",
    "platform",
    "plugin",
    "pr",
    "procedure",
    "process-identity",
    "process_identity",
    "ps",
    "publishing",
    "pull-request",
    "release",
    "resume",
    "review-threads",
    "rollback",
    "routines",
    "rust",
    "rust-std",
    "sandbox",
    "sbpl",
    "scheduler",
    "search",
    "security",
    "semantic-companion",
    "semantic-search",
    "ship",
    "skills",
    "sqlite",
    "symlinks",
    "systemd",
    "task-context",
    "task-lifecycle",
    "templating",
    "test",
    "testing",
    "tests",
    "tooling",
    "unix",
    "validation",
    "workers",
    "workflow",
    "workflow-admission",
    "workspace-registry",
    "worktree",
    "worktrees",
];

fn default_learning_tag_vocabulary() -> Vec<String> {
    DEFAULT_LEARNING_TAG_VOCABULARY
        .iter()
        .map(|tag| (*tag).to_string())
        .collect()
}

const fn default_upfront_learning_injection_cap() -> usize {
    DEFAULT_UPFRONT_LEARNING_INJECTION_CAP
}

pub fn task_registry_path(global_root: &Path) -> PathBuf {
    global_root.join("tasks").join("index.sqlite")
}

pub fn home_task_workspace_dir(global_root: &Path, workspace_id: &str) -> PathBuf {
    global_root
        .join("tasks")
        .join("workspaces")
        .join(workspace_id)
}

pub fn workspace_config_path(orbit_dir: &Path) -> PathBuf {
    orbit_dir.join("config.yaml")
}

pub fn read_workspace_config(orbit_dir: &Path) -> Result<WorkspaceConfig, OrbitError> {
    read_workspace_config_optional(orbit_dir)?.ok_or_else(|| {
        OrbitError::InvalidInput(format!(
            "workspace config is missing: {}",
            workspace_config_path(orbit_dir).display()
        ))
    })
}

pub fn workspace_id_for_orbit_dir(orbit_dir: &Path) -> Result<String, OrbitError> {
    let config = read_workspace_config(orbit_dir).map_err(|err| match err {
        OrbitError::InvalidInput(message) => {
            OrbitError::InvalidInput(format!("{message} (expected key `workspace_id`)"))
        }
        other => other,
    })?;
    Ok(config.workspace_id)
}

pub fn read_workspace_config_optional(
    orbit_dir: &Path,
) -> Result<Option<WorkspaceConfig>, OrbitError> {
    let path = workspace_config_path(orbit_dir);
    let raw = match fs::read_to_string(&path) {
        Ok(raw) => raw,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(err) => return Err(OrbitError::Io(err.to_string())),
    };
    let doc: WorkspaceConfigDoc = serde_yaml::from_str(&raw).map_err(|e| {
        OrbitError::InvalidInput(format!(
            "invalid workspace config '{}': {e}",
            path.display()
        ))
    })?;
    validate_workspace_config_doc(doc).map(Some)
}

pub fn write_workspace_config(
    orbit_dir: &Path,
    config: &WorkspaceConfig,
) -> Result<(), OrbitError> {
    let workspace_id = validate_workspace_id(&config.workspace_id)?;
    if config.schema_version != CONFIG_SCHEMA_VERSION {
        return Err(OrbitError::InvalidInput(format!(
            "unsupported workspace config schema_version {}",
            config.schema_version
        )));
    }

    let learnings = validate_learning_delivery_config(config.learnings.clone())?;
    let doc = WorkspaceConfigDoc {
        schema_version: CONFIG_SCHEMA_VERSION,
        workspace_id,
        learnings: LearningDeliveryConfigDoc {
            tag_vocabulary: learnings.tag_vocabulary,
            upfront_injection_cap: learnings.upfront_injection_cap,
        },
    };
    let content = serde_yaml::to_string(&doc).map_err(|e| OrbitError::Store(e.to_string()))?;
    atomic_write_text(&workspace_config_path(orbit_dir), &content)
        .map_err(|e| OrbitError::Io(e.to_string()))
}

pub fn assign_workspace_id(slug_source: &str, path: &Path) -> String {
    workspace_id_candidate(&sanitize_slug(slug_source), path, 0)
}

fn validate_workspace_config_doc(doc: WorkspaceConfigDoc) -> Result<WorkspaceConfig, OrbitError> {
    if doc.schema_version != CONFIG_SCHEMA_VERSION {
        return Err(OrbitError::InvalidInput(format!(
            "unsupported workspace config schema_version {}",
            doc.schema_version
        )));
    }
    Ok(WorkspaceConfig {
        schema_version: doc.schema_version,
        workspace_id: validate_workspace_id(&doc.workspace_id)?,
        learnings: validate_learning_delivery_config(LearningDeliveryConfig {
            tag_vocabulary: doc.learnings.tag_vocabulary,
            upfront_injection_cap: doc.learnings.upfront_injection_cap,
        })?,
    })
}

pub fn validate_learning_delivery_config(
    mut config: LearningDeliveryConfig,
) -> Result<LearningDeliveryConfig, OrbitError> {
    let mut seen = std::collections::BTreeSet::new();
    for tag in &mut config.tag_vocabulary {
        *tag = tag.trim().to_lowercase();
        if tag.is_empty() {
            return Err(OrbitError::InvalidInput(
                "learning tag vocabulary entries must not be empty".to_string(),
            ));
        }
        if !seen.insert(tag.clone()) {
            return Err(OrbitError::InvalidInput(format!(
                "learning tag vocabulary contains duplicate tag '{tag}'"
            )));
        }
    }
    Ok(config)
}

impl Default for LearningDeliveryConfig {
    fn default() -> Self {
        Self {
            tag_vocabulary: default_learning_tag_vocabulary(),
            upfront_injection_cap: DEFAULT_UPFRONT_LEARNING_INJECTION_CAP,
        }
    }
}
