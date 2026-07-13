//! Staged retention for terminal job-run records and legacy bundles.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Duration, Utc};
use orbit_common::types::{JobRun, JobRunState, OrbitError};
use orbit_store::JobRunGcRecord;
use serde::{Deserialize, Serialize};

use crate::OrbitRuntime;
use crate::command::job::gc_owner_permits_reclaim;
use crate::runtime::run_claim_guard;

use super::{
    GcCandidate, GcCollector, GcContext, GcItemError, GcMutation, GcPlan, GcRevalidation, GcSkip,
    GcTarget,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RunGcPolicy {
    pub archive_after_days: u64,
    pub purge_after_days: u64,
    pub failure_archive_after_days: u64,
    pub failure_purge_after_days: u64,
}

impl RunGcPolicy {
    pub fn from_runtime(runtime: &OrbitRuntime) -> Self {
        let (archive, purge, failure_archive, failure_purge) = runtime.run_gc_retention_days();
        Self {
            archive_after_days: archive,
            purge_after_days: purge,
            failure_archive_after_days: failure_archive,
            failure_purge_after_days: failure_purge,
        }
    }

    fn validate(self) -> Result<Self, OrbitError> {
        if self.purge_after_days < self.archive_after_days
            || self.failure_purge_after_days < self.failure_archive_after_days
            || self.failure_archive_after_days < self.archive_after_days
            || self.failure_purge_after_days < self.purge_after_days
        {
            return Err(OrbitError::InvalidInput(
                "run purge ages must follow archive ages and failure ages must not be shorter than success ages"
                    .to_string(),
            ));
        }
        Ok(self)
    }

    fn ages_for(self, state: JobRunState) -> (u64, u64) {
        if matches!(
            state,
            JobRunState::Failed | JobRunState::Timeout | JobRunState::Interrupted
        ) {
            (
                self.failure_archive_after_days,
                self.failure_purge_after_days,
            )
        } else {
            (self.archive_after_days, self.purge_after_days)
        }
    }
}

pub struct RunGcCollector<'a> {
    runtime: &'a OrbitRuntime,
    policy: RunGcPolicy,
}

impl<'a> RunGcCollector<'a> {
    pub fn new(runtime: &'a OrbitRuntime, policy: RunGcPolicy) -> Self {
        Self { runtime, policy }
    }

    fn classify_record(
        &self,
        record: &JobRunGcRecord,
        context: &GcContext<'_>,
    ) -> Result<Classified, OrbitError> {
        let run = &record.run;
        if !run.state.is_terminal() {
            return Ok(Classified::skip(
                &run.run_id,
                "active_run",
                "pending or running runs are never eligible",
            ));
        }
        let Some(finished_at) = run.finished_at else {
            return Ok(Classified::skip(
                &run.run_id,
                "missing_terminal_timestamp",
                "terminal transition timestamp is absent",
            ));
        };
        if let Some(hold) = self.protection_hold(run, context)? {
            return Ok(Classified::Skip(hold));
        }

        let policy = self.policy.validate()?;
        let (archive_days, purge_days) = policy.ages_for(run.state);
        let (action, required_days) = if record.archived_at.is_some() {
            ("purge", purge_days)
        } else {
            ("archive", archive_days)
        };
        let eligible_at = retention_deadline(finished_at, required_days);
        if context.clock.now() < eligible_at {
            return Ok(Classified::skip(
                &run.run_id,
                "retained",
                format!(
                    "{action} requires {required_days} days from terminal transition {finished_at}"
                ),
            ));
        }
        let path = find_legacy_bundle(context.scope.root(), &run.job_id, &run.run_id);
        let bytes = path.as_deref().map(directory_bytes_no_follow).transpose()?;
        let expected = ExpectedState {
            run_id: run.run_id.clone(),
            job_id: run.job_id.clone(),
            state: run.state,
            finished_at,
            eligible_at,
            archived: record.archived_at.is_some(),
            persisted: true,
        };
        Ok(Classified::Candidate(GcCandidate {
            id: run.run_id.clone(),
            action: action.to_string(),
            path,
            bytes,
            ownership_evidence: "SQLite run row, steps/checkpoint, reservations, task links, scoreboards, and legacy bundle inventoried; audit evidence excluded"
                .to_string(),
            retention_evidence: format!(
                "persisted terminal transition {finished_at}; {action} age {required_days}d"
            ),
            expected_state: serde_json::to_string(&expected)
                .map_err(|error| OrbitError::Execution(error.to_string()))?,
            allow_owned_symlink: false,
        }))
    }

    /// Live ownership, liveness, and reference protections shared by persisted
    /// rows and rowless legacy bundles. Returns the first hold that trips so both
    /// candidate kinds fail closed on the *same* conditions — owner liveness/PID
    /// identity, resumable recovery checkpoints, retained task/retry links, active
    /// reservations, and aggregate scoreboard references (criteria 2 and 5). A
    /// legacy bundle must clear every one of these before it is eligible, exactly
    /// as an authoritative row must.
    fn protection_hold(
        &self,
        run: &JobRun,
        context: &GcContext<'_>,
    ) -> Result<Option<GcSkip>, OrbitError> {
        let hold = |code: &str, reason: &str| {
            Some(GcSkip {
                id: run.run_id.clone(),
                code: code.to_string(),
                reason: reason.to_string(),
            })
        };
        if std::env::var("ORBIT_RUN_ID").ok().as_deref() == Some(run.run_id.as_str()) {
            return Ok(hold("current_run", "the current process owns this run"));
        }
        if !gc_owner_permits_reclaim(run) {
            return Ok(hold(
                "live_or_inconclusive",
                "recorded owner is alive or its liveness is inconclusive",
            ));
        }
        if self.is_resumable(run)? {
            return Ok(hold("resumable", "run has persisted recovery checkpoints"));
        }
        if !self
            .runtime
            .list_tasks_filtered(None, None, None, Some(&run.run_id), None, None)?
            .is_empty()
        {
            return Ok(hold(
                "task_linked",
                "a retained task still references this run",
            ));
        }
        if self.has_active_reservation(&run.run_id, context)? {
            return Ok(hold(
                "reservation_held",
                "an active task reservation still belongs to this run",
            ));
        }
        if self
            .runtime
            .stores()
            .jobs()
            .list_runs_for_gc()?
            .iter()
            .any(|other| {
                other.run.run_id != run.run_id
                    && other.run.retry_source_run_id.as_deref() == Some(run.run_id.as_str())
            })
        {
            return Ok(hold(
                "retry_linked",
                "a retained retry run still references this source run",
            ));
        }
        if scoreboard_references(&self.runtime.paths().scoreboard_dir, &run.run_id)? {
            return Ok(hold(
                "scoreboard_linked",
                "a retained aggregate scoreboard still references this run",
            ));
        }
        Ok(None)
    }

    fn is_resumable(&self, run: &JobRun) -> Result<bool, OrbitError> {
        if !matches!(
            run.state,
            JobRunState::Failed | JobRunState::Timeout | JobRunState::Interrupted
        ) {
            return Ok(false);
        }
        let Some(state) = self.runtime.read_run_state(&run.run_id)? else {
            return Ok(false);
        };
        Ok(state.next_step_index > 0
            || !state.step_outputs.is_empty()
            || !state.pipeline_patches.is_empty()
            || !state.step_states.is_empty())
    }

    fn has_active_reservation(
        &self,
        run_id: &str,
        context: &GcContext<'_>,
    ) -> Result<bool, OrbitError> {
        let workspace_id = match context.scope {
            super::GcScope::Workspace { workspace_id, .. } => workspace_id.as_deref(),
            super::GcScope::Global { .. } => None,
        };
        let active = self.runtime.stores().task_reservations().list_active(
            &self.runtime.paths().orbit_dir.to_string_lossy(),
            workspace_id,
        )?;
        Ok(active
            .reservations
            .iter()
            .any(|reservation| reservation.owner_run_id.as_deref() == Some(run_id)))
    }

    fn reclassify(
        &self,
        expected: &ExpectedState,
        context: &GcContext<'_>,
    ) -> Result<GcRevalidation, OrbitError> {
        let record = self
            .runtime
            .stores()
            .jobs()
            .list_runs_for_gc()?
            .into_iter()
            .find(|record| record.run.run_id == expected.run_id);
        let Some(record) = record else {
            return Ok(GcRevalidation::Skip {
                code: "owner_changed".to_string(),
                reason: "run row disappeared after planning".to_string(),
            });
        };
        if record.run.job_id != expected.job_id
            || record.run.state != expected.state
            || record.run.finished_at != Some(expected.finished_at)
            || record.archived_at.is_some() != expected.archived
        {
            return Ok(GcRevalidation::Skip {
                code: "owner_changed".to_string(),
                reason: "run state, terminal timestamp, or archive stage changed".to_string(),
            });
        }
        match self.classify_record(&record, context)? {
            Classified::Candidate(candidate)
                if candidate.action
                    == if expected.archived {
                        "purge"
                    } else {
                        "archive"
                    } =>
            {
                Ok(GcRevalidation::Ready)
            }
            Classified::Skip(skip) => Ok(GcRevalidation::Skip {
                code: skip.code,
                reason: skip.reason,
            }),
            _ => Ok(GcRevalidation::Skip {
                code: "retention_changed".to_string(),
                reason: "run is no longer eligible".to_string(),
            }),
        }
    }
}

impl GcCollector for RunGcCollector<'_> {
    fn target(&self) -> GcTarget {
        GcTarget::Runs
    }

    fn plan(&self, context: &GcContext<'_>) -> Result<GcPlan, OrbitError> {
        if context.retention_override.is_some() {
            return Err(OrbitError::InvalidInput(
                "runs have separate archive, purge, and failure ages; use the qualified run retention options"
                    .to_string(),
            ));
        }
        let mut plan = GcPlan::empty(GcTarget::Runs);
        plan.config_source = "workspace".to_string();
        let records = self.runtime.stores().jobs().list_runs_for_gc()?;
        let persisted_ids = records
            .iter()
            .map(|record| record.run.run_id.clone())
            .collect::<BTreeSet<_>>();
        for record in records {
            plan.scanned = plan.scanned.saturating_add(1);
            match self.classify_record(&record, context) {
                Ok(Classified::Candidate(candidate)) => {
                    if let Some(bytes) = candidate.bytes {
                        plan.scanned_bytes =
                            plan.scanned_bytes.map(|sum| sum.saturating_add(bytes));
                    }
                    plan.candidates.push(candidate);
                }
                Ok(Classified::Skip(skip)) => plan.skipped.push(skip),
                Err(error) => plan.errors.push(GcItemError {
                    id: record.run.run_id,
                    phase: "scan".to_string(),
                    code: "inventory_failed".to_string(),
                    message: error.to_string(),
                }),
            }
        }
        self.inventory_stale_legacy(context, &persisted_ids, &mut plan)?;
        Ok(plan)
    }

    fn revalidate(
        &self,
        candidate: &GcCandidate,
        context: &GcContext<'_>,
    ) -> Result<GcRevalidation, OrbitError> {
        let expected: ExpectedState = serde_json::from_str(&candidate.expected_state)
            .map_err(|error| OrbitError::Execution(format!("invalid frozen run state: {error}")))?;
        if !expected.persisted {
            return self.revalidate_stale_bundle(candidate, &expected, context);
        }
        self.reclassify(&expected, context)
    }

    fn apply(
        &self,
        candidate: &GcCandidate,
        context: &GcContext<'_>,
    ) -> Result<GcMutation, OrbitError> {
        let expected: ExpectedState = serde_json::from_str(&candidate.expected_state)
            .map_err(|error| OrbitError::Execution(format!("invalid frozen run state: {error}")))?;
        if !expected.persisted {
            // Rowless legacy bundles take the *same* per-run claim guard as the
            // persisted path and hold it across the final no-row/protection
            // revalidation and the filesystem mutation, so a live or resumable
            // owner claiming the run — or an authoritative row materializing —
            // between plan and apply cannot race the archive/purge (criteria 2
            // and 5).
            let _guard =
                run_claim_guard::acquire(&self.runtime.paths().state_dir, &expected.run_id)?;
            if !matches!(
                self.revalidate_stale_bundle(candidate, &expected, context)?,
                GcRevalidation::Ready
            ) {
                return Err(OrbitError::Execution(
                    "legacy bundle eligibility changed while acquiring its claim guard".to_string(),
                ));
            }
            if let Some(path) = candidate.path.as_deref() {
                mutate_legacy_path(path, context.scope.root(), &expected, &candidate.action)?;
            }
            return Ok(GcMutation {
                reclaimed_bytes: candidate.bytes,
            });
        }
        let _guard = run_claim_guard::acquire(&self.runtime.paths().state_dir, &expected.run_id)?;
        if !matches!(self.reclassify(&expected, context)?, GcRevalidation::Ready) {
            return Err(OrbitError::Execution(
                "run eligibility changed while acquiring its claim guard".to_string(),
            ));
        }
        if let Some(path) = candidate.path.as_deref() {
            mutate_legacy_path(path, context.scope.root(), &expected, &candidate.action)?;
        }
        match candidate.action.as_str() {
            "archive" => self.runtime.archive_job_run(&expected.run_id)?,
            "purge" => self.runtime.delete_job_run(&expected.run_id)?,
            other => {
                return Err(OrbitError::Execution(format!(
                    "unsupported run GC action `{other}`"
                )));
            }
        }
        Ok(GcMutation {
            reclaimed_bytes: candidate.bytes,
        })
    }
}

impl RunGcCollector<'_> {
    /// Revalidate a rowless legacy bundle before mutation. Beyond confirming the
    /// bundle's identity/terminal state and age are unchanged, this fails closed
    /// when an authoritative row has since appeared (the persisted collector owns
    /// it then) and re-runs the shared ownership/liveness/reference protections on
    /// the freshly-read bundle, so a live/resumable owner or a retained reference
    /// blocks the archive/purge (criteria 2 and 5). The legacy `apply` path holds
    /// the per-run claim guard across this check and the mutation.
    fn revalidate_stale_bundle(
        &self,
        candidate: &GcCandidate,
        expected: &ExpectedState,
        context: &GcContext<'_>,
    ) -> Result<GcRevalidation, OrbitError> {
        let Some(path) = candidate.path.as_deref() else {
            return Ok(GcRevalidation::Skip {
                code: "missing_path".to_string(),
                reason: "legacy bundle path is absent".to_string(),
            });
        };
        let run = match read_legacy_run(path) {
            Ok(run) => run,
            Err(_) => {
                return Ok(GcRevalidation::Skip {
                    code: "bundle_changed".to_string(),
                    reason: "legacy bundle disappeared or became unreadable".to_string(),
                });
            }
        };
        if run.run_id != expected.run_id
            || run.job_id != expected.job_id
            || run.state != expected.state
            || run.finished_at != Some(expected.finished_at)
        {
            return Ok(GcRevalidation::Skip {
                code: "bundle_changed".to_string(),
                reason: "legacy bundle identity or terminal state changed".to_string(),
            });
        }
        if context.clock.now() < expected.eligible_at {
            return Ok(GcRevalidation::Skip {
                code: "retention_changed".to_string(),
                reason: "terminal timestamp is now in the future".to_string(),
            });
        }
        if self
            .runtime
            .stores()
            .jobs()
            .list_runs_for_gc()?
            .iter()
            .any(|record| record.run.run_id == expected.run_id)
        {
            return Ok(GcRevalidation::Skip {
                code: "row_appeared".to_string(),
                reason:
                    "an authoritative run row now exists; the persisted collector owns this run"
                        .to_string(),
            });
        }
        if let Some(hold) = self.protection_hold(&run, context)? {
            return Ok(GcRevalidation::Skip {
                code: hold.code,
                reason: hold.reason,
            });
        }
        Ok(GcRevalidation::Ready)
    }

    fn inventory_stale_legacy(
        &self,
        context: &GcContext<'_>,
        persisted_ids: &BTreeSet<String>,
        plan: &mut GcPlan,
    ) -> Result<(), OrbitError> {
        for (archived, root) in legacy_roots(context.scope.root()) {
            if !root.exists() {
                continue;
            }
            for job_entry in fs::read_dir(&root)? {
                let job_entry = job_entry?;
                if !job_entry.path().is_dir() {
                    continue;
                }
                if !archived
                    && job_entry.path().file_name().and_then(|name| name.to_str())
                        == Some("archived")
                {
                    continue;
                }
                for run_entry in fs::read_dir(job_entry.path())? {
                    let path = run_entry?.path();
                    if !path.is_dir() {
                        continue;
                    }
                    let run = match read_legacy_run(&path) {
                        Ok(run) => run,
                        Err(error) => {
                            plan.scanned = plan.scanned.saturating_add(1);
                            plan.errors.push(GcItemError {
                                id: path.display().to_string(),
                                phase: "scan".to_string(),
                                code: "legacy_bundle_invalid".to_string(),
                                message: error.to_string(),
                            });
                            continue;
                        }
                    };
                    if persisted_ids.contains(&run.run_id) {
                        continue;
                    }
                    plan.scanned = plan.scanned.saturating_add(1);
                    let Some(finished_at) = run.finished_at.filter(|_| run.state.is_terminal())
                    else {
                        plan.skipped.push(GcSkip {
                            id: run.run_id,
                            code: "orphan_nonterminal".to_string(),
                            reason:
                                "legacy bundle has no authoritative row and no terminal timestamp"
                                    .to_string(),
                        });
                        continue;
                    };
                    // Fail closed on the same ownership/liveness/reference holds
                    // the persisted path enforces, so a rowless legacy bundle is
                    // never eligible while a live/resumable owner or a retained
                    // task/retry/reservation/scoreboard reference still uses it.
                    if let Some(hold) = self.protection_hold(&run, context)? {
                        plan.skipped.push(hold);
                        continue;
                    }
                    let (archive_days, purge_days) = self.policy.validate()?.ages_for(run.state);
                    let (action, required_days) = if archived {
                        ("purge", purge_days)
                    } else {
                        ("archive", archive_days)
                    };
                    let eligible_at = retention_deadline(finished_at, required_days);
                    if context.clock.now() < eligible_at {
                        plan.skipped.push(GcSkip {
                            id: run.run_id,
                            code: "retained".to_string(),
                            reason: format!("stale legacy bundle requires {required_days} days"),
                        });
                        continue;
                    }
                    let bytes = directory_bytes_no_follow(&path).ok();
                    let expected = ExpectedState {
                        run_id: run.run_id.clone(),
                        job_id: run.job_id,
                        state: run.state,
                        finished_at,
                        eligible_at,
                        archived,
                        persisted: false,
                    };
                    plan.candidates.push(GcCandidate {
                        id: run.run_id,
                        action: action.to_string(),
                        path: Some(path),
                        bytes,
                        ownership_evidence: "legacy Orbit job-run bundle; no authoritative SQLite row; audit roots excluded"
                            .to_string(),
                        retention_evidence: format!(
                            "bundle persisted terminal transition {finished_at}; {action} age {required_days}d"
                        ),
                        expected_state: serde_json::to_string(&expected)
                            .map_err(|error| OrbitError::Execution(error.to_string()))?,
                        allow_owned_symlink: false,
                    });
                }
            }
        }
        Ok(())
    }
}

#[derive(Debug)]
enum Classified {
    Candidate(GcCandidate),
    Skip(GcSkip),
}

impl Classified {
    fn skip(id: &str, code: &str, reason: impl Into<String>) -> Self {
        Self::Skip(GcSkip {
            id: id.to_string(),
            code: code.to_string(),
            reason: reason.into(),
        })
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct ExpectedState {
    run_id: String,
    job_id: String,
    state: JobRunState,
    finished_at: DateTime<Utc>,
    eligible_at: DateTime<Utc>,
    archived: bool,
    persisted: bool,
}

#[derive(Deserialize)]
struct LegacyRunDocument {
    run: JobRun,
}

fn read_legacy_run(path: &Path) -> Result<JobRun, OrbitError> {
    let raw = fs::read_to_string(path.join("jrun.yaml"))?;
    serde_yaml::from_str::<LegacyRunDocument>(&raw)
        .map(|document| document.run)
        .map_err(|error| OrbitError::Store(format!("invalid legacy run bundle: {error}")))
}

fn legacy_roots(orbit_root: &Path) -> [(bool, PathBuf); 2] {
    let root = orbit_root.join("state").join("job-runs");
    [(false, root.clone()), (true, root.join("archived"))]
}

fn find_legacy_bundle(orbit_root: &Path, job_id: &str, run_id: &str) -> Option<PathBuf> {
    let root = orbit_root.join("state").join("job-runs");
    let active = root.join(job_id).join(run_id);
    if active.is_dir() {
        return Some(active);
    }
    let archived = root.join("archived").join(job_id).join(run_id);
    archived.is_dir().then_some(archived)
}

fn mutate_legacy_path(
    path: &Path,
    orbit_root: &Path,
    expected: &ExpectedState,
    action: &str,
) -> Result<(), OrbitError> {
    if !path.exists() {
        return Ok(());
    }
    match action {
        "archive" => {
            let destination = orbit_root
                .join("state/job-runs/archived")
                .join(&expected.job_id)
                .join(&expected.run_id);
            if path == destination {
                return Ok(());
            }
            let parent = destination.parent().ok_or_else(|| {
                OrbitError::Io("legacy archive destination has no parent".to_string())
            })?;
            fs::create_dir_all(parent)?;
            fs::rename(path, destination)?;
        }
        "purge" => fs::remove_dir_all(path)?,
        other => {
            return Err(OrbitError::Execution(format!(
                "unsupported legacy run action `{other}`"
            )));
        }
    }
    Ok(())
}

fn scoreboard_references(root: &Path, run_id: &str) -> Result<bool, OrbitError> {
    if !root.exists() {
        return Ok(false);
    }
    for entry in fs::read_dir(root)? {
        let path = entry?.path();
        if path.is_file() {
            let body = fs::read_to_string(path)?;
            if body.contains(run_id) {
                return Ok(true);
            }
        }
    }
    Ok(false)
}

fn directory_bytes_no_follow(path: &Path) -> Result<u64, OrbitError> {
    let metadata = fs::symlink_metadata(path)?;
    if metadata.file_type().is_symlink() || metadata.is_file() {
        return Ok(metadata.len());
    }
    let mut total = 0_u64;
    for entry in fs::read_dir(path)? {
        total = total.saturating_add(directory_bytes_no_follow(&entry?.path())?);
    }
    Ok(total)
}

fn retention_deadline(finished_at: DateTime<Utc>, days: u64) -> DateTime<Utc> {
    let Ok(days) = i64::try_from(days) else {
        return DateTime::<Utc>::MAX_UTC;
    };
    let Some(duration) = Duration::try_days(days) else {
        return DateTime::<Utc>::MAX_UTC;
    };
    finished_at
        .checked_add_signed(duration)
        .unwrap_or(DateTime::<Utc>::MAX_UTC)
}
