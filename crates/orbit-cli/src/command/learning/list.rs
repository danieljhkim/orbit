use std::str::FromStr;

use clap::Args;
use orbit_common::utility::glob::{compile_glob_regex, normalize_glob_path};
use orbit_core::{LearningListEntry, LearningStatus, OrbitError, OrbitRuntime};
use serde_json::Value;

use crate::command::Execute;

use super::output::learning_to_json;

#[derive(Args)]
pub struct LearningListArgs {
    /// Filter by status (active | superseded). Defaults to all.
    #[arg(long)]
    pub status: Option<String>,
    /// Filter to learnings whose scope tags contain this tag
    #[arg(long)]
    pub tag: Option<String>,
    /// Filter to learnings whose `scope.paths` glob-contain this path. A
    /// learning matches when any of its scope globs resolves true against
    /// the given path.
    #[arg(long)]
    pub path: Option<String>,
    /// Output as JSON
    #[arg(long)]
    pub json: bool,
    /// Include learnings whose body files are recorded in another worktree but not locally readable
    #[arg(long)]
    pub include_remote: bool,
}

impl Execute for LearningListArgs {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        let status = self
            .status
            .as_deref()
            .map(|raw| LearningStatus::from_str(raw).map_err(OrbitError::InvalidInput))
            .transpose()?;
        let tag = self.tag.as_deref().map(|t| t.trim().to_lowercase());
        let path_normalized = self.path.as_deref().map(normalize_glob_path).transpose()?;

        let learnings = runtime.list_learning_entries(status, self.include_remote)?;
        let filtered: Vec<_> = learnings
            .into_iter()
            .filter(|entry| {
                let LearningListEntry::Local(l) = entry else {
                    return tag.is_none() && self.path.is_none();
                };
                if let Some(ref tag) = tag
                    && !l.scope.tags.iter().any(|t| t == tag)
                {
                    return false;
                }
                if let Some(ref path) = path_normalized
                    && !learning_scope_contains_path(l, path)
                {
                    return false;
                }
                true
            })
            .collect();

        if self.json {
            let array = Value::Array(filtered.iter().map(learning_entry_to_json).collect());
            crate::output::json::print_pretty(&array)
        } else {
            for entry in &filtered {
                match entry {
                    LearningListEntry::Local(learning) => {
                        println!(
                            "{}\t{}\t{}",
                            learning.id,
                            learning.status.as_str(),
                            learning.summary
                        );
                    }
                    LearningListEntry::Remote(stub) => {
                        println!(
                            "{}\t{}\t[remote: {}]",
                            stub.id,
                            stub.status,
                            stub.worktree_root.display()
                        );
                    }
                }
            }
            Ok(())
        }
    }
}

fn learning_scope_contains_path(learning: &orbit_core::Learning, path: &str) -> bool {
    learning.scope.paths.iter().any(|rule| {
        compile_glob_regex(rule)
            .map(|regex| regex.is_match(path))
            .unwrap_or(false)
    })
}

fn learning_entry_to_json(entry: &LearningListEntry) -> Value {
    match entry {
        LearningListEntry::Local(learning) => {
            let mut value = learning_to_json(learning);
            if let Some(object) = value.as_object_mut() {
                object.insert("remote".to_string(), Value::Bool(false));
            }
            value
        }
        LearningListEntry::Remote(stub) => serde_json::json!({
            "id": stub.id,
            "kind": stub.kind,
            "status": stub.status,
            "remote": true,
            "remote_marker": format!("[remote: {}]", stub.worktree_root.display()),
            "worktree_root": stub.worktree_root.to_string_lossy(),
            "branch": stub.branch,
            "body_path": stub.body_path.as_ref().map(|path| path.to_string_lossy().to_string()),
            "summary": Value::Null,
            "body": Value::Null,
            "scope": Value::Null,
            "evidence": Value::Null,
        }),
    }
}
