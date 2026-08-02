use clap::Args;
use orbit_core::OrbitRuntime;

use crate::command::{CommandOut, Execute, Payload};

use super::output::learning_show_to_json;

#[derive(Args)]
pub struct LearningShowArgs {
    /// Learning ID (e.g. L-0001)
    pub id: String,
    /// Output as JSON
    #[arg(long)]
    pub json: bool,
}

impl Execute for LearningShowArgs {
    fn execute(self, runtime: &OrbitRuntime) -> CommandOut {
        let learning = runtime.get_learning(&self.id)?;
        // Opening the full body is the passive usage signal for this learning.
        runtime.record_learning_shown(&learning.id)?;
        let doc = learning_show_to_json(&learning);

        use std::fmt::Write as _;
        let mut out = String::new();
        let _ = writeln!(out, "ID: {}", learning.id);
        let _ = writeln!(out, "Status: {}", learning.status.as_str());
        let _ = writeln!(out, "Summary: {}", learning.summary);
        if !learning.scope.paths.is_empty() {
            let _ = writeln!(out, "Paths: {}", learning.scope.paths.join(", "));
        }
        if !learning.scope.tags.is_empty() {
            let _ = writeln!(out, "Tags: {}", learning.scope.tags.join(", "));
        }
        if !learning.body.is_empty() {
            let _ = writeln!(out, "Body:\n{}", learning.body);
        }
        if !learning.evidence.is_empty() {
            let _ = writeln!(out, "Evidence:");
            for evidence in &learning.evidence {
                let _ = writeln!(out, "  {}: {}", evidence.kind, evidence.reference);
            }
        }
        if let Some(priority) = learning.priority {
            let _ = writeln!(out, "Priority: {priority}");
        }
        if let Some(ref supersedes) = learning.supersedes {
            let _ = writeln!(out, "Supersedes: {supersedes}");
        }
        if let Some(ref superseded_by) = learning.superseded_by {
            let _ = writeln!(out, "Superseded By: {superseded_by}");
        }
        let _ = writeln!(out, "Created: {}", learning.created_at.to_rfc3339());
        let _ = writeln!(out, "Updated: {}", learning.updated_at.to_rfc3339());
        Ok(Payload::detail(doc, out).into())
    }
}
