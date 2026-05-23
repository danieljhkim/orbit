//! Query, list, show, and history methods for job runs, with reconciliation.

use orbit_common::types::{JobRun, NotFoundKind, OrbitError};
use orbit_store::JobRunQuery;

use crate::OrbitRuntime;

use super::types::JobRunListParams;

impl OrbitRuntime {
    pub fn job_history(&self, job_id: &str) -> Result<Vec<JobRun>, OrbitError> {
        self.reconcile_stale_job_runs(Some(job_id))?;
        match self.load_v2_job_asset_by_name(job_id) {
            Ok(_) => self.list_reconciled_job_history_backend(job_id),
            Err(error) => {
                let runs = self.list_reconciled_job_history_backend(job_id)?;
                if runs.is_empty() {
                    Err(error)
                } else {
                    Ok(runs)
                }
            }
        }
    }

    pub fn list_job_runs(&self, params: JobRunListParams) -> Result<Vec<JobRun>, OrbitError> {
        self.reconcile_stale_job_runs(params.job_id.as_deref())?;
        if let Some(job_id) = params.job_id.as_deref()
            && let Err(error) = self.load_v2_job_asset_by_name(job_id)
        {
            let runs = self.list_job_history_backend(job_id)?;
            if runs.is_empty() {
                return Err(error);
            }
        }

        let query = JobRunQuery {
            job_id: params.job_id,
            state: params.state,
            created_since: params.since,
            limit: params.limit,
        };
        let runs = self.list_job_runs_filtered_backend(&query)?;
        if self.reconcile_job_run_records(&runs)? > 0 {
            self.list_job_runs_filtered_backend(&query)
        } else {
            Ok(runs)
        }
    }

    pub fn show_job_run(&self, run_id: &str) -> Result<JobRun, OrbitError> {
        let run = self
            .get_job_run_backend(run_id)?
            .ok_or_else(|| OrbitError::not_found(NotFoundKind::JobRun, run_id.to_string()))?;
        self.reconcile_stale_job_run(&run)?;
        self.get_job_run_backend(run_id)?
            .ok_or_else(|| OrbitError::not_found(NotFoundKind::JobRun, run_id.to_string()))
    }

    // Note: list_reconciled..., reconcile_job_run_records, list_job_history_backend,
    // list_job_runs_filtered_backend, get_job_run_backend live in reconcile + here
    // but to avoid dup, some private backends are here; reconcile has list_reconciled etc.

    pub(super) fn list_job_history_backend(&self, job_id: &str) -> Result<Vec<JobRun>, OrbitError> {
        self.stores().jobs().list_runs(job_id)
    }

    pub(super) fn list_job_runs_filtered_backend(
        &self,
        query: &JobRunQuery,
    ) -> Result<Vec<JobRun>, OrbitError> {
        self.stores().jobs().list_runs_filtered(query)
    }

    pub(crate) fn get_job_run_backend(&self, run_id: &str) -> Result<Option<JobRun>, OrbitError> {
        self.stores().jobs().get_run(run_id)
    }
}
