use std::fs;
use std::path::PathBuf;

use clap::{ArgAction, Args};
use orbit_core::{LearningUpdateParams, OrbitError, OrbitRuntime};

use crate::command::{CommandOut, CommandOutput, Execute, Payload};

use super::output::learning_to_json;

#[derive(Args)]
pub struct LearningUpdateArgs {
    /// Learning ID
    pub id: String,
    /// Replace `summary` (≤ 280 chars)
    #[arg(long)]
    pub summary: Option<String>,
    /// Replace `scope.paths`. Pass `--path` once per entry; pass `--path ""` to clear. Omit to preserve.
    #[arg(long = "path", action = ArgAction::Append)]
    pub paths: Vec<String>,
    /// Replace `scope.tags`. Pass `--tag` once per entry; pass `--tag ""` to clear. Omit to preserve.
    #[arg(long = "tag", action = ArgAction::Append)]
    pub tags: Vec<String>,
    /// Replace `body`
    #[arg(long)]
    pub body: Option<String>,
    /// Replace `body` from a file
    #[arg(long = "body-file")]
    pub body_file: Option<PathBuf>,
    /// Replace `priority` (0–255)
    #[arg(long)]
    pub priority: Option<u8>,
    /// Clear `priority`
    #[arg(long, conflicts_with = "priority")]
    pub clear_priority: bool,
    /// Output as JSON
    #[arg(long)]
    pub json: bool,
}

impl Execute for LearningUpdateArgs {
    fn execute(self, runtime: &OrbitRuntime) -> CommandOut {
        let body =
            match (self.body, self.body_file) {
                (Some(_), Some(_)) => {
                    return Err(OrbitError::InvalidInput(
                        "specify exactly one of `--body` and `--body-file`".to_string(),
                    ));
                }
                (Some(body), None) => Some(body),
                (None, Some(path)) => Some(fs::read_to_string(&path).map_err(|e| {
                    OrbitError::Io(format!("read body file {}: {e}", path.display()))
                })?),
                (None, None) => None,
            };

        let scope = if !self.paths.is_empty() || !self.tags.is_empty() {
            let mut scope = runtime.get_learning(&self.id)?.scope;
            if !self.paths.is_empty() {
                scope.paths = self.paths;
            }
            if !self.tags.is_empty() {
                scope.tags = self.tags;
            }
            Some(scope)
        } else {
            None
        };

        let priority = if self.clear_priority {
            Some(None)
        } else {
            self.priority.map(Some)
        };

        let learning = runtime.author_learning_update(
            &self.id,
            LearningUpdateParams {
                summary: self.summary,
                scope,
                body,
                evidence: None,
                priority,
            },
        )?;

        if self.json {
            Ok(Payload::document(learning_to_json(&learning)).into())
        } else {
            println!("{}", learning.id);
            Ok(CommandOutput::Silent)
        }
    }
}
