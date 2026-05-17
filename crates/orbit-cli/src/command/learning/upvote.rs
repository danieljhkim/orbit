use clap::Args;
use orbit_core::{LearningUpvoteParams, OrbitError, OrbitRuntime};
use serde_json::json;

use crate::command::Execute;

#[derive(Args)]
pub struct LearningUpvoteArgs {
    /// Learning ID (e.g. L20260511-1)
    #[arg(long = "id")]
    pub id: String,
    /// Voter model or canonical agent family
    #[arg(long)]
    pub model: String,
    /// Task ID anchoring this re-validation vote
    #[arg(long = "task")]
    pub task: Option<String>,
    /// Output as JSON
    #[arg(long)]
    pub json: bool,
}

impl Execute for LearningUpvoteArgs {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        let summary = runtime.upvote_learning(LearningUpvoteParams {
            learning_id: self.id.clone(),
            voter_model: self.model,
            task_id: self.task,
        })?;

        if self.json {
            crate::output::json::print_pretty(&json!({
                "vote_count": summary.vote_count,
                "last_voted_at": summary.last_voted_at.map(|ts| ts.to_rfc3339()),
            }))
        } else {
            println!("{}\tvote_count={}", self.id, summary.vote_count);
            Ok(())
        }
    }
}
