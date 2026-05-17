use clap::Args;
use orbit_core::{OrbitError, OrbitRuntime};

use crate::command::Execute;

use super::output::learning_show_to_json;

#[derive(Args)]
pub struct LearningShowArgs {
    /// Learning ID (e.g. L20260511-1)
    pub id: String,
    /// Output as JSON
    #[arg(long)]
    pub json: bool,
}

impl Execute for LearningShowArgs {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        let learning = runtime.get_learning(&self.id)?;
        let vote_summary = runtime.learning_vote_summary(&self.id)?;
        if self.json {
            crate::output::json::print_pretty(&learning_show_to_json(&learning, &vote_summary))
        } else {
            println!("ID: {}", learning.id);
            println!("Status: {}", learning.status.as_str());
            println!("Votes: {}", vote_summary.vote_count);
            if let Some(last_voted_at) = vote_summary.last_voted_at {
                println!("Last Voted: {}", last_voted_at.to_rfc3339());
            }
            println!("Summary: {}", learning.summary);
            if !learning.scope.paths.is_empty() {
                println!("Paths: {}", learning.scope.paths.join(", "));
            }
            if !learning.scope.tags.is_empty() {
                println!("Tags: {}", learning.scope.tags.join(", "));
            }
            if !learning.body.is_empty() {
                println!("Body:\n{}", learning.body);
            }
            if !learning.evidence.is_empty() {
                println!("Evidence:");
                for evidence in &learning.evidence {
                    println!("  {}: {}", evidence.kind, evidence.reference);
                }
            }
            if let Some(priority) = learning.priority {
                println!("Priority: {priority}");
            }
            if let Some(ref supersedes) = learning.supersedes {
                println!("Supersedes: {supersedes}");
            }
            if let Some(ref superseded_by) = learning.superseded_by {
                println!("Superseded By: {superseded_by}");
            }
            println!("Created: {}", learning.created_at.to_rfc3339());
            println!("Updated: {}", learning.updated_at.to_rfc3339());
            Ok(())
        }
    }
}
