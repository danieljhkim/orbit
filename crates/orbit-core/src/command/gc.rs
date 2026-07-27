use chrono::{Duration, Utc};
use orbit_engine::{WorktreeGcOptions, WorktreeGcResult, collect_worktrees};

use crate::{OrbitError, OrbitRuntime};

impl OrbitRuntime {
    pub fn gc_worktrees(
        &self,
        delete: bool,
        run_id: Option<String>,
        older_than_hours: Option<u64>,
    ) -> Result<WorktreeGcResult, OrbitError> {
        let runs = self.list_job_runs(super::job::JobRunListParams::default())?;
        let older_than = older_than_hours
            .map(|hours| {
                let hours = i64::try_from(hours).map_err(|_| {
                    OrbitError::InvalidInput("--older-than-hours is too large".to_string())
                })?;
                Utc::now()
                    .checked_sub_signed(Duration::hours(hours))
                    .ok_or_else(|| {
                        OrbitError::InvalidInput("--older-than-hours is too large".to_string())
                    })
            })
            .transpose()?;
        collect_worktrees(
            &self.paths().repo_root,
            &runs,
            self,
            &WorktreeGcOptions {
                delete,
                run_id,
                older_than,
            },
        )
    }
}
