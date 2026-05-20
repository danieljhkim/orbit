use std::str::FromStr;

use clap::Args;
use orbit_common::utility::glob::{compile_glob_regex, normalize_glob_path};
use orbit_core::{LearningStatus, OrbitError, OrbitRuntime};
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

        let learnings = runtime.list_learnings(status)?;
        let filtered: Vec<_> = learnings
            .into_iter()
            .filter(|l| {
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
            let array = Value::Array(filtered.iter().map(learning_to_json).collect());
            crate::output::json::print_pretty(&array)
        } else {
            for learning in &filtered {
                println!(
                    "{}\t{}\t{}",
                    learning.id,
                    learning.status.as_str(),
                    learning.summary
                );
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
