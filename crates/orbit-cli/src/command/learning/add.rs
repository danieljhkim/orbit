use std::fs;
use std::path::PathBuf;

use clap::{ArgAction, Args};
use orbit_core::{
    EvidenceKind, LearningCreateParams, LearningEvidence, LearningScope, OrbitError, OrbitRuntime,
};

use crate::command::Execute;

use super::output::learning_to_json;

#[derive(Args)]
pub struct LearningAddArgs {
    /// One-line summary (≤ 280 chars)
    #[arg(long)]
    pub summary: String,
    /// Path-glob scope entry. Repeat for multiple.
    #[arg(long = "path", action = ArgAction::Append)]
    pub paths: Vec<String>,
    /// Tag scope entry. Repeat for multiple.
    #[arg(long = "tag", action = ArgAction::Append)]
    pub tags: Vec<String>,
    /// Long-form body (inline)
    #[arg(long)]
    pub body: Option<String>,
    /// Read the body from a file
    #[arg(long = "body-file")]
    pub body_file: Option<PathBuf>,
    /// Evidence in `<kind>:<ref>` form (kinds: task, commit, external). Repeat for multiple.
    #[arg(long = "evidence", action = ArgAction::Append)]
    pub evidence: Vec<String>,
    /// Optional priority (0-255) used as a secondary search-ranking key
    #[arg(long)]
    pub priority: Option<u8>,
    /// Output as JSON
    #[arg(long)]
    pub json: bool,
}

impl Execute for LearningAddArgs {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        let body = match (self.body, self.body_file) {
            (Some(_), Some(_)) => {
                return Err(OrbitError::InvalidInput(
                    "specify exactly one of `--body` and `--body-file`".to_string(),
                ));
            }
            (Some(body), None) => body,
            (None, Some(path)) => fs::read_to_string(&path)
                .map_err(|e| OrbitError::Io(format!("read body file {}: {e}", path.display())))?,
            (None, None) => String::new(),
        };
        let evidence = self
            .evidence
            .into_iter()
            .map(parse_evidence_spec)
            .collect::<Result<Vec<_>, _>>()?;

        let learning = runtime.author_learning(LearningCreateParams {
            summary: self.summary,
            scope: LearningScope {
                paths: self.paths,
                tags: self.tags,
                ..Default::default()
            },
            body,
            evidence,
            created_by: None,
            priority: self.priority,
        })?;

        if self.json {
            crate::output::json::print_pretty(&learning_to_json(&learning))
        } else {
            println!("{}", learning.id);
            Ok(())
        }
    }
}

fn parse_evidence_spec(raw: String) -> Result<LearningEvidence, OrbitError> {
    let (kind, reference) = raw.split_once(':').ok_or_else(|| {
        OrbitError::InvalidInput(format!(
            "evidence must be `<kind>:<ref>`; got `{raw}` (kinds: task, commit, external)"
        ))
    })?;
    let kind = match kind.trim() {
        "task" => EvidenceKind::Task,
        "commit" => EvidenceKind::Commit,
        "external" => EvidenceKind::External,
        other => {
            return Err(OrbitError::InvalidInput(format!(
                "unknown evidence kind `{other}`; expected one of task, commit, external"
            )));
        }
    };
    Ok(LearningEvidence {
        kind,
        reference: reference.trim().to_string(),
    })
}
