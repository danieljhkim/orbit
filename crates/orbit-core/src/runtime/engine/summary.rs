use chrono::{Duration, Utc};
use orbit_common::types::OrbitError;
use orbit_store::scoreboard_summary::{ScoreboardInputs, ScoreboardWindow};
use orbit_store::{AdrListFilter, JobRunQuery};

use crate::OrbitRuntime;

const RECENT_WINDOW_DAYS: i64 = 7;
/// Cap on the "most-called tools" leaderboard. 50 is comfortably above the
/// distinct (role, tool) pair count we observe in current workspaces and
/// keeps `summary.json` size bounded.
const TOP_TOOLS_LIMIT: usize = 50;

impl OrbitRuntime {
    /// Build a scoreboard summary for the workspace.
    ///
    /// `window`: `None` (or `Some(ScoreboardWindow::All)`) preserves the
    /// legacy lifetime view. A finite window scopes audit-sourced fields
    /// to the matching SQL cutoff and zeroes snapshot-sourced fields
    /// (see [`ScoreboardWindow`] for per-source semantics). `recent_7d`
    /// stays fixed at 7d regardless of `window`.
    pub fn generate_scoreboard_summary(
        &self,
        window: Option<ScoreboardWindow>,
    ) -> Result<orbit_store::scoreboard_summary::ScoreboardSummary, OrbitError> {
        let window = window.unwrap_or_default();
        let tasks = self.list_tasks()?;

        let now = Utc::now();
        let since_recent = now - Duration::days(RECENT_WINDOW_DAYS);
        let since_window = window.duration().map(|d| now - d);

        let audit_tool_calls = self.audit_tool_call_counts_by_role(since_window.as_ref())?;
        let audit_tool_calls_by_surface =
            self.audit_tool_call_counts_by_surface_and_role(since_window.as_ref())?;
        let audit_tool_calls_by_surface_recent =
            self.audit_tool_call_counts_by_surface_and_role(Some(&since_recent))?;
        let top_tool_calls = self.audit_top_tool_calls(since_window.as_ref(), TOP_TOOLS_LIMIT)?;
        let job_runs = self
            .stores()
            .jobs()
            .list_job_runs_filtered(&JobRunQuery::default())?;
        let learnings = self.list_learnings(None)?;
        let adrs = self
            .stores()
            .adrs()
            .list_adrs_filtered(AdrListFilter::default())?;
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
                adrs: &adrs,
                frictions: &frictions,
                now: Some(now),
                window,
            },
        )?;
        let _ =
            orbit_store::scoreboard_summary::write_summary(&self.paths().scoreboard_dir, &summary)?;
        Ok(summary)
    }
}
