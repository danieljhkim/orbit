use chrono::{Duration, Utc};
use orbit_common::types::{AdrStatus, OrbitError};
use orbit_store::JobRunQuery;
use orbit_store::scoreboard_summary::ScoreboardInputs;

use crate::OrbitRuntime;

const RECENT_WINDOW_DAYS: i64 = 7;
/// Cap on the "most-called tools" leaderboard. 50 is comfortably above the
/// distinct (role, tool) pair count we observe in current workspaces and
/// keeps `summary.json` size bounded.
const TOP_TOOLS_LIMIT: usize = 50;

impl OrbitRuntime {
    pub fn generate_scoreboard_summary(
        &self,
    ) -> Result<orbit_store::scoreboard_summary::ScoreboardSummary, OrbitError> {
        let tasks = self.list_tasks()?;

        let now = Utc::now();
        let since_recent = now - Duration::days(RECENT_WINDOW_DAYS);

        let audit_tool_calls = self.audit_tool_call_counts_by_role(None)?;
        let audit_tool_calls_by_surface = self.audit_tool_call_counts_by_surface_and_role(None)?;
        let audit_tool_calls_by_surface_recent =
            self.audit_tool_call_counts_by_surface_and_role(Some(&since_recent))?;
        let top_tool_calls = self.audit_top_tool_calls(None, TOP_TOOLS_LIMIT)?;
        let job_runs = self
            .stores()
            .jobs()
            .list_runs_filtered(&JobRunQuery::default())?;
        let learnings = self.list_learnings(None)?;
        let mut learning_vote_counts = Vec::with_capacity(learnings.len());
        for learning in &learnings {
            let vote_count = self.learning_vote_summary(&learning.id)?.vote_count as u64;
            learning_vote_counts.push((learning.id.clone(), vote_count));
        }
        let adrs = self.stores().adrs().list_filtered(
            None::<AdrStatus>,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
        )?;
        let frictions = orbit_store::friction_store::list_frictions(
            &self.data_root().join("frictions"),
            &orbit_store::friction_store::FrictionListFilter::default(),
        )?;

        let summary = orbit_store::scoreboard_summary::generate_summary_with_inputs(
            &self.paths().scoreboard_dir,
            &tasks,
            &ScoreboardInputs {
                audit_tool_calls: &audit_tool_calls,
                audit_tool_calls_by_surface: &audit_tool_calls_by_surface,
                audit_tool_calls_by_surface_recent: &audit_tool_calls_by_surface_recent,
                job_runs: &job_runs,
                top_tool_calls: &top_tool_calls,
                learnings: &learnings,
                learning_vote_counts: &learning_vote_counts,
                adrs: &adrs,
                frictions: &frictions,
                now: Some(now),
            },
        )?;
        let _ =
            orbit_store::scoreboard_summary::write_summary(&self.paths().scoreboard_dir, &summary)?;
        Ok(summary)
    }

    pub fn scoreboard_summary_path(&self) -> std::path::PathBuf {
        orbit_store::scoreboard_summary::summary_path(&self.paths().scoreboard_dir)
    }

    /// Read the append-only duel scoreboard log. The CLI's
    /// `orbit duel scoreboard` command aggregates the returned runs in
    /// memory via `orbit_store::duel_scoreboard::aggregate`. Returns an
    /// empty vector when the file does not yet exist — an unrun
    /// scoreboard is not an error condition.
    pub fn load_duel_runs(&self) -> Result<Vec<orbit_common::types::DuelRun>, OrbitError> {
        orbit_store::duel_scoreboard::load_runs(&self.paths().scoreboard_dir)
    }
}
