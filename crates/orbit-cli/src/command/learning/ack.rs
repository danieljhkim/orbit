use clap::Args;
use orbit_core::{LearningAckOutcome, OrbitError, OrbitRuntime};

use crate::command::Execute;

#[derive(Args)]
pub struct LearningAckArgs {
    /// Learning IDs to ack (as listed in the injected reminder block)
    #[arg(required = true)]
    pub ids: Vec<String>,
    /// Record an explicit dismissal instead of the default `used` outcome.
    /// Injections with no ack already count as ignored.
    #[arg(long)]
    pub ignored: bool,
    /// Session the ack belongs to. Defaults to ORBIT_SESSION_ID when exported.
    #[arg(long)]
    pub session: Option<String>,
    /// Output as JSON
    #[arg(long)]
    pub json: bool,
}

impl Execute for LearningAckArgs {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        let outcome = if self.ignored {
            LearningAckOutcome::Ignored
        } else {
            LearningAckOutcome::Used
        };
        let session_id = self.session.or_else(|| {
            std::env::var("ORBIT_SESSION_ID")
                .ok()
                .filter(|value| !value.trim().is_empty())
        });

        match runtime.ack_learnings(&self.ids, outcome, session_id.as_deref()) {
            Ok(()) => {
                if self.json {
                    crate::output::json::print_pretty(&serde_json::json!({
                        "acked": self.ids,
                        "outcome": outcome.as_str(),
                    }))?;
                } else {
                    println!("acked {} as {}", self.ids.join(", "), outcome.as_str());
                }
                Ok(())
            }
            // Caller mistakes (unknown ID, empty input) fail closed; an
            // unavailable audit backend fails open so a scripted ack (e.g.
            // in a session-end hook) never breaks the agent's main work.
            Err(error @ (OrbitError::InvalidInput(_) | OrbitError::NotFound { .. })) => Err(error),
            Err(error) => {
                eprintln!("warning: learning ack failed open: {error}");
                Ok(())
            }
        }
    }
}
