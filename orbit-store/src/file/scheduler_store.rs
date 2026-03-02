use std::fs;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use orbit_types::{
    OrbitError, Scheduler, SchedulerRetryBackoffStrategy, SchedulerRun, SchedulerRunState,
    SchedulerScheduleState, SchedulerTargetType,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Clone)]
pub(crate) struct SchedulerFileStore {
    root: PathBuf,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SchedulerFileDocument {
    schema_version: u8,
    scheduler: Scheduler,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SchedulerRunFileDocument {
    schema_version: u8,
    run: SchedulerRun,
}

impl SchedulerFileStore {
    pub(crate) fn new(root: PathBuf) -> Self {
        Self { root }
    }

    pub(crate) fn ensure_layout(&self) -> Result<(), OrbitError> {
        fs::create_dir_all(self.jobs_dir()).map_err(|e| OrbitError::Io(e.to_string()))?;
        fs::create_dir_all(self.runs_dir()).map_err(|e| OrbitError::Io(e.to_string()))?;
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn insert_job_v2(
        &self,
        target_type: SchedulerTargetType,
        target_id: &str,
        schedule: &str,
        agent_cli: &str,
        timeout_seconds: u64,
        retry_max_attempts: u32,
        retry_backoff_strategy: SchedulerRetryBackoffStrategy,
        retry_initial_delay_seconds: u64,
        next_run_at: DateTime<Utc>,
    ) -> Result<Scheduler, OrbitError> {
        self.ensure_layout()?;
        let now = Utc::now();
        let scheduler = Scheduler {
            scheduler_id: self.next_id("scheduler"),
            target_type,
            target_id: target_id.to_string(),
            schedule: schedule.to_string(),
            agent_cli: agent_cli.to_string(),
            timeout_seconds,
            retry_max_attempts,
            retry_backoff_strategy,
            retry_initial_delay_seconds,
            state: SchedulerScheduleState::Enabled,
            next_run_at,
            created_at: now,
            updated_at: now,
        };
        self.write_job(&scheduler)?;
        Ok(scheduler)
    }

    pub(crate) fn list_schedulers(
        &self,
        include_disabled: bool,
    ) -> Result<Vec<Scheduler>, OrbitError> {
        let mut schedulers = self.read_all_jobs()?;
        if !include_disabled {
            schedulers.retain(|scheduler| scheduler.state != SchedulerScheduleState::Disabled);
        }
        schedulers.sort_by(|a, b| {
            b.created_at
                .cmp(&a.created_at)
                .then_with(|| a.scheduler_id.cmp(&b.scheduler_id))
        });
        Ok(schedulers)
    }

    pub(crate) fn get_scheduler(
        &self,
        scheduler_id: &str,
    ) -> Result<Option<Scheduler>, OrbitError> {
        let path = self.scheduler_path(scheduler_id);
        if !path.exists() {
            return Ok(None);
        }
        Ok(Some(self.read_job_at(&path)?))
    }

    pub(crate) fn due_schedulers(&self, now: DateTime<Utc>) -> Result<Vec<Scheduler>, OrbitError> {
        let mut schedulers = self
            .read_all_jobs()?
            .into_iter()
            .filter(|scheduler| {
                scheduler.state == SchedulerScheduleState::Enabled && scheduler.next_run_at <= now
            })
            .collect::<Vec<_>>();
        schedulers.sort_by(|a, b| a.next_run_at.cmp(&b.next_run_at));
        Ok(schedulers)
    }

    pub(crate) fn list_scheduler_runs(
        &self,
        scheduler_id: &str,
    ) -> Result<Vec<SchedulerRun>, OrbitError> {
        let mut runs = self.read_runs_for_job(scheduler_id)?;
        runs.sort_by(|a, b| {
            b.created_at
                .cmp(&a.created_at)
                .then_with(|| a.run_id.cmp(&b.run_id))
        });
        Ok(runs)
    }

    pub(crate) fn get_pending_or_running_scheduler_run(
        &self,
        scheduler_id: &str,
    ) -> Result<Option<SchedulerRun>, OrbitError> {
        let mut runs = self
            .read_runs_for_job(scheduler_id)?
            .into_iter()
            .filter(|run| {
                run.state == SchedulerRunState::Pending || run.state == SchedulerRunState::Running
            })
            .collect::<Vec<_>>();
        runs.sort_by(|a, b| b.created_at.cmp(&a.created_at));
        Ok(runs.into_iter().next())
    }

    pub(crate) fn set_scheduler_state(
        &self,
        scheduler_id: &str,
        state: SchedulerScheduleState,
    ) -> Result<bool, OrbitError> {
        let Some(mut scheduler) = self.get_scheduler(scheduler_id)? else {
            return Ok(false);
        };
        scheduler.state = state;
        scheduler.updated_at = Utc::now();
        self.write_job(&scheduler)?;
        Ok(true)
    }

    pub(crate) fn mark_scheduler_disabled(&self, scheduler_id: &str) -> Result<bool, OrbitError> {
        self.set_scheduler_state(scheduler_id, SchedulerScheduleState::Disabled)
    }

    pub(crate) fn update_scheduler_next_run(
        &self,
        scheduler_id: &str,
        next_run_at: DateTime<Utc>,
    ) -> Result<bool, OrbitError> {
        let Some(mut scheduler) = self.get_scheduler(scheduler_id)? else {
            return Ok(false);
        };
        scheduler.next_run_at = next_run_at;
        scheduler.updated_at = Utc::now();
        self.write_job(&scheduler)?;
        Ok(true)
    }

    pub(crate) fn insert_scheduler_run(
        &self,
        scheduler_id: &str,
        attempt: u32,
        scheduled_at: DateTime<Utc>,
    ) -> Result<SchedulerRun, OrbitError> {
        let run = SchedulerRun {
            run_id: self.next_id("jrun"),
            scheduler_id: scheduler_id.to_string(),
            attempt,
            state: SchedulerRunState::Pending,
            scheduled_at,
            started_at: None,
            finished_at: None,
            duration_ms: None,
            exit_code: None,
            agent_response_json: None,
            error_code: None,
            error_message: None,
            created_at: Utc::now(),
        };
        self.write_run(scheduler_id, &run)?;
        Ok(run)
    }

    pub(crate) fn mark_scheduler_run_running(
        &self,
        run_id: &str,
        started_at: DateTime<Utc>,
    ) -> Result<bool, OrbitError> {
        let Some((scheduler_id, path)) = self.find_run_path(run_id)? else {
            return Ok(false);
        };
        let mut run = self.read_run_at(&path)?;
        run.state = SchedulerRunState::Running;
        run.started_at = Some(started_at);
        self.write_run(&scheduler_id, &run)?;
        Ok(true)
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn complete_scheduler_run(
        &self,
        run_id: &str,
        state: SchedulerRunState,
        finished_at: DateTime<Utc>,
        duration_ms: Option<u64>,
        exit_code: Option<i32>,
        agent_response_json: Option<&Value>,
        error_code: Option<&str>,
        error_message: Option<&str>,
    ) -> Result<bool, OrbitError> {
        let Some((scheduler_id, path)) = self.find_run_path(run_id)? else {
            return Ok(false);
        };
        let mut run = self.read_run_at(&path)?;
        run.state = state;
        run.finished_at = Some(finished_at);
        run.duration_ms = duration_ms;
        run.exit_code = exit_code;
        run.agent_response_json = agent_response_json.cloned();
        run.error_code = error_code.map(ToString::to_string);
        run.error_message = error_message.map(ToString::to_string);
        self.write_run(&scheduler_id, &run)?;
        Ok(true)
    }

    pub(crate) fn claim_due_schedulers(
        &self,
        now: DateTime<Utc>,
    ) -> Result<DueJobsClaim, OrbitError> {
        let due_schedulers = self.due_schedulers(now)?;
        let mut result = DueJobsClaim::default();

        for scheduler in due_schedulers {
            if self
                .get_pending_or_running_scheduler_run(&scheduler.scheduler_id)?
                .is_some()
            {
                result.skipped.push(scheduler.scheduler_id.clone());
                continue;
            }
            let run = self.insert_scheduler_run(&scheduler.scheduler_id, 1, now)?;
            result.claimed.push(ClaimedJobRun { scheduler, run });
        }
        Ok(result)
    }

    fn read_all_jobs(&self) -> Result<Vec<Scheduler>, OrbitError> {
        self.ensure_layout()?;
        let mut paths = fs::read_dir(self.jobs_dir())
            .map_err(|e| OrbitError::Io(e.to_string()))?
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| is_yaml(path))
            .collect::<Vec<_>>();
        paths.sort();
        let mut schedulers = Vec::new();
        for path in paths {
            schedulers.push(self.read_job_at(&path)?);
        }
        Ok(schedulers)
    }

    fn read_runs_for_job(&self, scheduler_id: &str) -> Result<Vec<SchedulerRun>, OrbitError> {
        let dir = self.run_dir(scheduler_id);
        if !dir.exists() {
            return Ok(Vec::new());
        }
        let mut paths = fs::read_dir(dir)
            .map_err(|e| OrbitError::Io(e.to_string()))?
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| is_yaml(path))
            .collect::<Vec<_>>();
        paths.sort();
        let mut runs = Vec::new();
        for path in paths {
            runs.push(self.read_run_at(&path)?);
        }
        Ok(runs)
    }

    fn find_run_path(&self, run_id: &str) -> Result<Option<(String, PathBuf)>, OrbitError> {
        let runs_root = self.runs_dir();
        if !runs_root.exists() {
            return Ok(None);
        }
        for entry in fs::read_dir(runs_root).map_err(|e| OrbitError::Io(e.to_string()))? {
            let entry = entry.map_err(|e| OrbitError::Io(e.to_string()))?;
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            let Some(scheduler_id) = path.file_name().and_then(|v| v.to_str()) else {
                continue;
            };
            let run_path = path.join(format!("{run_id}.yaml"));
            if run_path.exists() {
                return Ok(Some((scheduler_id.to_string(), run_path)));
            }
        }
        Ok(None)
    }

    fn read_job_at(&self, path: &Path) -> Result<Scheduler, OrbitError> {
        let raw = fs::read_to_string(path).map_err(|e| OrbitError::Io(e.to_string()))?;
        let doc = serde_yaml::from_str::<SchedulerFileDocument>(&raw).map_err(|e| {
            OrbitError::Store(format!("invalid scheduler file '{}': {e}", path.display()))
        })?;
        Ok(doc.scheduler)
    }

    fn read_run_at(&self, path: &Path) -> Result<SchedulerRun, OrbitError> {
        let raw = fs::read_to_string(path).map_err(|e| OrbitError::Io(e.to_string()))?;
        let doc = serde_yaml::from_str::<SchedulerRunFileDocument>(&raw).map_err(|e| {
            OrbitError::Store(format!(
                "invalid scheduler run file '{}': {e}",
                path.display()
            ))
        })?;
        Ok(doc.run)
    }

    fn write_job(&self, scheduler: &Scheduler) -> Result<(), OrbitError> {
        self.ensure_layout()?;
        let doc = SchedulerFileDocument {
            schema_version: 1,
            scheduler: scheduler.clone(),
        };
        let content = serde_yaml::to_string(&doc).map_err(|e| OrbitError::Store(e.to_string()))?;
        write_atomic(&self.scheduler_path(&scheduler.scheduler_id), &content)
    }

    fn write_run(&self, scheduler_id: &str, run: &SchedulerRun) -> Result<(), OrbitError> {
        self.ensure_layout()?;
        let doc = SchedulerRunFileDocument {
            schema_version: 1,
            run: run.clone(),
        };
        let content = serde_yaml::to_string(&doc).map_err(|e| OrbitError::Store(e.to_string()))?;
        write_atomic(&self.run_path(scheduler_id, &run.run_id), &content)
    }

    fn next_id(&self, prefix: &str) -> String {
        let nanos = Utc::now().timestamp_nanos_opt().unwrap_or_default();
        format!("{prefix}-{nanos}")
    }

    fn jobs_dir(&self) -> PathBuf {
        self.root.join("schedulers")
    }

    fn runs_dir(&self) -> PathBuf {
        self.root.join("runs")
    }

    fn scheduler_path(&self, scheduler_id: &str) -> PathBuf {
        self.jobs_dir().join(format!("{scheduler_id}.yaml"))
    }

    fn run_dir(&self, scheduler_id: &str) -> PathBuf {
        self.runs_dir().join(scheduler_id)
    }

    fn run_path(&self, scheduler_id: &str, run_id: &str) -> PathBuf {
        self.run_dir(scheduler_id).join(format!("{run_id}.yaml"))
    }
}

fn write_atomic(path: &Path, content: &str) -> Result<(), OrbitError> {
    let parent = path.parent().ok_or_else(|| {
        OrbitError::Io(format!("cannot determine parent for '{}'", path.display()))
    })?;
    fs::create_dir_all(parent).map_err(|e| OrbitError::Io(e.to_string()))?;

    let mut tmp = path.to_path_buf();
    let nanos = Utc::now().timestamp_nanos_opt().unwrap_or_default();
    tmp.set_extension(format!("yaml.tmp.{nanos}"));
    fs::write(&tmp, content).map_err(|e| OrbitError::Io(e.to_string()))?;
    if let Err(err) = fs::rename(&tmp, path) {
        let _ = fs::remove_file(&tmp);
        return Err(OrbitError::Io(err.to_string()));
    }
    Ok(())
}

fn is_yaml(path: &Path) -> bool {
    path.extension()
        .and_then(|value| value.to_str())
        .is_some_and(|ext| ext.eq_ignore_ascii_case("yaml") || ext.eq_ignore_ascii_case("yml"))
}
use crate::{ClaimedJobRun, DueJobsClaim};
