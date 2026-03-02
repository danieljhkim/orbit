use chrono::{DateTime, Utc};
use orbit_types::{
    OrbitError, Scheduler, SchedulerRetryBackoffStrategy, SchedulerRun, SchedulerRunState,
    SchedulerScheduleState, SchedulerTargetType,
};
use rusqlite::{OptionalExtension, params};
use serde_json::Value;

use crate::{Store, StoreTx, new_id, now_string, parse_timestamp};

#[derive(Debug, Clone)]
pub struct ClaimedJobRun {
    pub scheduler: Scheduler,
    pub run: SchedulerRun,
}

#[derive(Debug, Clone, Default)]
pub struct DueJobsClaim {
    pub claimed: Vec<ClaimedJobRun>,
    pub skipped: Vec<String>,
}

const JOB_COLS: &str = "id, target_type, target_id, schedule, agent_cli, timeout_seconds, retry_max_attempts, retry_backoff_strategy, retry_initial_delay_seconds, state, next_run_at, created_at, updated_at";
const JOB_RUN_COLS: &str = "id, scheduler_id, attempt, state, scheduled_at, started_at, finished_at, duration_ms, exit_code, agent_response_json, error_code, error_message, created_at";

impl Store {
    pub fn list_schedulers(&self, include_disabled: bool) -> Result<Vec<Scheduler>, OrbitError> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| OrbitError::Store(format!("mutex poisoned: {e}")))?;

        let sql = if include_disabled {
            format!("SELECT {JOB_COLS} FROM schedulers ORDER BY created_at DESC")
        } else {
            format!(
                "SELECT {JOB_COLS} FROM schedulers WHERE state != 'disabled' ORDER BY created_at DESC"
            )
        };

        let mut stmt = conn
            .prepare(&sql)
            .map_err(|e| OrbitError::Store(e.to_string()))?;
        let rows = stmt
            .query_map([], row_to_job)
            .map_err(|e| OrbitError::Store(e.to_string()))?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|e| OrbitError::Store(e.to_string()))
    }

    pub fn get_scheduler(&self, scheduler_id: &str) -> Result<Option<Scheduler>, OrbitError> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| OrbitError::Store(format!("mutex poisoned: {e}")))?;

        conn.query_row(
            &format!("SELECT {JOB_COLS} FROM schedulers WHERE id = ?1"),
            [scheduler_id],
            row_to_job,
        )
        .optional()
        .map_err(|e| OrbitError::Store(e.to_string()))
    }

    pub fn due_schedulers(&self, now: DateTime<Utc>) -> Result<Vec<Scheduler>, OrbitError> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| OrbitError::Store(format!("mutex poisoned: {e}")))?;

        let mut stmt = conn
            .prepare(&format!(
                "SELECT {JOB_COLS}
                 FROM schedulers
                 WHERE state = 'enabled'
                   AND next_run_at <= ?1
                 ORDER BY next_run_at ASC"
            ))
            .map_err(|e| OrbitError::Store(e.to_string()))?;

        let rows = stmt
            .query_map([now.to_rfc3339()], row_to_job)
            .map_err(|e| OrbitError::Store(e.to_string()))?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|e| OrbitError::Store(e.to_string()))
    }

    pub fn list_scheduler_runs(&self, scheduler_id: &str) -> Result<Vec<SchedulerRun>, OrbitError> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| OrbitError::Store(format!("mutex poisoned: {e}")))?;

        let mut stmt = conn
            .prepare(&format!(
                "SELECT {JOB_RUN_COLS}
                 FROM scheduler_runs
                 WHERE scheduler_id = ?1
                 ORDER BY created_at DESC"
            ))
            .map_err(|e| OrbitError::Store(e.to_string()))?;
        let rows = stmt
            .query_map([scheduler_id], row_to_job_run)
            .map_err(|e| OrbitError::Store(e.to_string()))?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|e| OrbitError::Store(e.to_string()))
    }

    pub fn get_job_run(&self, run_id: &str) -> Result<Option<SchedulerRun>, OrbitError> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| OrbitError::Store(format!("mutex poisoned: {e}")))?;

        conn.query_row(
            &format!("SELECT {JOB_RUN_COLS} FROM scheduler_runs WHERE id = ?1"),
            [run_id],
            row_to_job_run,
        )
        .optional()
        .map_err(|e| OrbitError::Store(e.to_string()))
    }

    pub fn get_running_job_run(
        &self,
        scheduler_id: &str,
    ) -> Result<Option<SchedulerRun>, OrbitError> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| OrbitError::Store(format!("mutex poisoned: {e}")))?;

        conn.query_row(
            &format!(
                "SELECT {JOB_RUN_COLS}
                 FROM scheduler_runs
                 WHERE scheduler_id = ?1 AND state = 'running'
                 ORDER BY created_at DESC
                 LIMIT 1"
            ),
            [scheduler_id],
            row_to_job_run,
        )
        .optional()
        .map_err(|e| OrbitError::Store(e.to_string()))
    }

    pub fn get_pending_or_running_scheduler_run(
        &self,
        scheduler_id: &str,
    ) -> Result<Option<SchedulerRun>, OrbitError> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| OrbitError::Store(format!("mutex poisoned: {e}")))?;

        conn.query_row(
            &format!(
                "SELECT {JOB_RUN_COLS}
                 FROM scheduler_runs
                 WHERE scheduler_id = ?1 AND (state = 'pending' OR state = 'running')
                 ORDER BY created_at DESC
                 LIMIT 1"
            ),
            [scheduler_id],
            row_to_job_run,
        )
        .optional()
        .map_err(|e| OrbitError::Store(e.to_string()))
    }
}

impl<'a> StoreTx<'a> {
    #[allow(clippy::too_many_arguments)]
    pub fn insert_job_v2(
        &mut self,
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
        let now = Utc::now();
        let scheduler = Scheduler {
            scheduler_id: new_id("scheduler"),
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

        self.tx
            .execute(
                "INSERT INTO schedulers(
                    id, target_type, target_id, schedule, agent_cli,
                    timeout_seconds, retry_max_attempts, retry_backoff_strategy,
                    retry_initial_delay_seconds, state, next_run_at, created_at, updated_at
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
                params![
                    scheduler.scheduler_id,
                    scheduler.target_type.to_string(),
                    scheduler.target_id,
                    scheduler.schedule,
                    scheduler.agent_cli,
                    scheduler.timeout_seconds as i64,
                    scheduler.retry_max_attempts as i64,
                    scheduler.retry_backoff_strategy.to_string(),
                    scheduler.retry_initial_delay_seconds as i64,
                    scheduler.state.to_string(),
                    scheduler.next_run_at.to_rfc3339(),
                    scheduler.created_at.to_rfc3339(),
                    scheduler.updated_at.to_rfc3339(),
                ],
            )
            .map_err(|e| OrbitError::Store(e.to_string()))?;
        Ok(scheduler)
    }

    pub fn set_scheduler_state(
        &mut self,
        scheduler_id: &str,
        state: SchedulerScheduleState,
    ) -> Result<bool, OrbitError> {
        let changed = self
            .tx
            .execute(
                "UPDATE schedulers
                 SET state = ?1, updated_at = ?2
                 WHERE id = ?3",
                params![state.to_string(), now_string(), scheduler_id],
            )
            .map_err(|e| OrbitError::Store(e.to_string()))?;
        Ok(changed == 1)
    }

    pub fn mark_scheduler_disabled(&mut self, scheduler_id: &str) -> Result<bool, OrbitError> {
        self.set_scheduler_state(scheduler_id, SchedulerScheduleState::Disabled)
    }

    pub fn update_scheduler_next_run(
        &mut self,
        scheduler_id: &str,
        next_run_at: DateTime<Utc>,
    ) -> Result<bool, OrbitError> {
        let changed = self
            .tx
            .execute(
                "UPDATE schedulers
                 SET next_run_at = ?1, updated_at = ?2
                 WHERE id = ?3",
                params![next_run_at.to_rfc3339(), now_string(), scheduler_id],
            )
            .map_err(|e| OrbitError::Store(e.to_string()))?;
        Ok(changed == 1)
    }

    pub fn next_job_run_attempt(
        &mut self,
        scheduler_id: &str,
        scheduled_at: DateTime<Utc>,
    ) -> Result<u32, OrbitError> {
        let max_attempt: Option<i64> = self
            .tx
            .query_row(
                "SELECT MAX(attempt) FROM scheduler_runs WHERE scheduler_id = ?1 AND scheduled_at = ?2",
                params![scheduler_id, scheduled_at.to_rfc3339()],
                |row| row.get::<_, Option<i64>>(0),
            )
            .map_err(|e| OrbitError::Store(e.to_string()))?;

        Ok((max_attempt.unwrap_or(0) as u32) + 1)
    }

    pub fn insert_scheduler_run(
        &mut self,
        scheduler_id: &str,
        attempt: u32,
        scheduled_at: DateTime<Utc>,
    ) -> Result<SchedulerRun, OrbitError> {
        let now = Utc::now();
        let run = SchedulerRun {
            run_id: new_id("jrun"),
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
            created_at: now,
        };

        self.tx
            .execute(
                "INSERT INTO scheduler_runs(
                    id, scheduler_id, attempt, state, scheduled_at, started_at,
                    finished_at, duration_ms, exit_code, agent_response_json,
                    error_code, error_message, created_at
                ) VALUES (?1, ?2, ?3, ?4, ?5, NULL, NULL, NULL, NULL, NULL, NULL, NULL, ?6)",
                params![
                    run.run_id,
                    run.scheduler_id,
                    run.attempt as i64,
                    run.state.to_string(),
                    run.scheduled_at.to_rfc3339(),
                    run.created_at.to_rfc3339(),
                ],
            )
            .map_err(|e| OrbitError::Store(e.to_string()))?;

        Ok(run)
    }

    pub fn mark_scheduler_run_running(
        &mut self,
        run_id: &str,
        started_at: DateTime<Utc>,
    ) -> Result<bool, OrbitError> {
        let changed = self
            .tx
            .execute(
                "UPDATE scheduler_runs
                 SET state = 'running', started_at = ?1
                 WHERE id = ?2",
                params![started_at.to_rfc3339(), run_id],
            )
            .map_err(|e| OrbitError::Store(e.to_string()))?;
        Ok(changed == 1)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn complete_scheduler_run(
        &mut self,
        run_id: &str,
        state: SchedulerRunState,
        finished_at: DateTime<Utc>,
        duration_ms: Option<u64>,
        exit_code: Option<i32>,
        agent_response_json: Option<&Value>,
        error_code: Option<&str>,
        error_message: Option<&str>,
    ) -> Result<bool, OrbitError> {
        let response_raw = agent_response_json
            .map(serde_json::to_string)
            .transpose()
            .map_err(|e| OrbitError::Store(format!("serialize agent response: {e}")))?;

        let changed = self
            .tx
            .execute(
                "UPDATE scheduler_runs
                 SET state = ?1,
                     finished_at = ?2,
                     duration_ms = ?3,
                     exit_code = ?4,
                     agent_response_json = ?5,
                     error_code = ?6,
                     error_message = ?7
                 WHERE id = ?8",
                params![
                    state.to_string(),
                    finished_at.to_rfc3339(),
                    duration_ms.map(|v| v as i64),
                    exit_code,
                    response_raw,
                    error_code,
                    error_message,
                    run_id,
                ],
            )
            .map_err(|e| OrbitError::Store(e.to_string()))?;
        Ok(changed == 1)
    }

    pub fn claim_due_schedulers(&mut self, now: DateTime<Utc>) -> Result<DueJobsClaim, OrbitError> {
        let due_schedulers = {
            let mut stmt = self
                .tx
                .prepare(&format!(
                    "SELECT {JOB_COLS}
                     FROM schedulers
                     WHERE state = 'enabled'
                       AND next_run_at <= ?1
                     ORDER BY next_run_at ASC"
                ))
                .map_err(|e| OrbitError::Store(e.to_string()))?;

            let rows = stmt
                .query_map([now.to_rfc3339()], row_to_job)
                .map_err(|e| OrbitError::Store(e.to_string()))?;
            rows.collect::<Result<Vec<_>, _>>()
                .map_err(|e| OrbitError::Store(e.to_string()))?
        };

        let mut result = DueJobsClaim::default();
        for scheduler in due_schedulers {
            let running_exists: Option<String> = self
                .tx
                .query_row(
                    "SELECT id FROM scheduler_runs WHERE scheduler_id = ?1 AND (state = 'pending' OR state = 'running') LIMIT 1",
                    [scheduler.scheduler_id.clone()],
                    |row| row.get(0),
                )
                .optional()
                .map_err(|e| OrbitError::Store(e.to_string()))?;

            if running_exists.is_some() {
                result.skipped.push(scheduler.scheduler_id.clone());
                continue;
            }

            let run = self.insert_scheduler_run(&scheduler.scheduler_id, 1, now)?;
            result.claimed.push(ClaimedJobRun { scheduler, run });
        }

        Ok(result)
    }
}

fn row_to_job(row: &rusqlite::Row<'_>) -> rusqlite::Result<Scheduler> {
    let target_type_raw: String = row.get(1)?;
    let state_raw: String = row.get(9)?;
    let next_run_at_raw: String = row.get(10)?;
    let created_at_raw: String = row.get(11)?;
    let updated_at_raw: String = row.get(12)?;
    let timeout_seconds: i64 = row.get(5)?;
    let retry_max_attempts: i64 = row.get(6)?;
    let retry_initial_delay_seconds: i64 = row.get(8)?;
    let backoff_raw: String = row.get(7)?;

    Ok(Scheduler {
        scheduler_id: row.get(0)?,
        target_type: parse_target_type(&target_type_raw)?,
        target_id: row.get(2)?,
        schedule: row.get(3)?,
        agent_cli: row.get(4)?,
        timeout_seconds: timeout_seconds as u64,
        retry_max_attempts: retry_max_attempts as u32,
        retry_backoff_strategy: parse_backoff_strategy(&backoff_raw)?,
        retry_initial_delay_seconds: retry_initial_delay_seconds as u64,
        state: parse_job_state(&state_raw)?,
        next_run_at: parse_timestamp(&next_run_at_raw)?,
        created_at: parse_timestamp(&created_at_raw)?,
        updated_at: parse_timestamp(&updated_at_raw)?,
    })
}

fn row_to_job_run(row: &rusqlite::Row<'_>) -> rusqlite::Result<SchedulerRun> {
    let state_raw: String = row.get(3)?;
    let scheduled_at_raw: String = row.get(4)?;
    let started_at_raw: Option<String> = row.get(5)?;
    let finished_at_raw: Option<String> = row.get(6)?;
    let agent_response_raw: Option<String> = row.get(9)?;
    let created_at_raw: String = row.get(12)?;

    let response_json = agent_response_raw
        .map(|raw| {
            serde_json::from_str::<Value>(&raw).map_err(|e| {
                rusqlite::Error::FromSqlConversionFailure(
                    raw.len(),
                    rusqlite::types::Type::Text,
                    Box::new(e),
                )
            })
        })
        .transpose()?;

    let attempt: i64 = row.get(2)?;
    let duration_ms: Option<i64> = row.get(7)?;

    Ok(SchedulerRun {
        run_id: row.get(0)?,
        scheduler_id: row.get(1)?,
        attempt: attempt as u32,
        state: parse_job_run_state(&state_raw)?,
        scheduled_at: parse_timestamp(&scheduled_at_raw)?,
        started_at: parse_optional_timestamp(started_at_raw)?,
        finished_at: parse_optional_timestamp(finished_at_raw)?,
        duration_ms: duration_ms.map(|v| v as u64),
        exit_code: row.get(8)?,
        agent_response_json: response_json,
        error_code: row.get(10)?,
        error_message: row.get(11)?,
        created_at: parse_timestamp(&created_at_raw)?,
    })
}

fn parse_optional_timestamp(raw: Option<String>) -> rusqlite::Result<Option<DateTime<Utc>>> {
    raw.map(|value| parse_timestamp(&value)).transpose()
}

fn parse_target_type(raw: &str) -> rusqlite::Result<SchedulerTargetType> {
    raw.parse::<SchedulerTargetType>()
        .map_err(|e| parse_enum_error(raw, e))
}

fn parse_job_state(raw: &str) -> rusqlite::Result<SchedulerScheduleState> {
    raw.parse::<SchedulerScheduleState>()
        .map_err(|e| parse_enum_error(raw, e))
}

fn parse_backoff_strategy(raw: &str) -> rusqlite::Result<SchedulerRetryBackoffStrategy> {
    raw.parse::<SchedulerRetryBackoffStrategy>()
        .map_err(|e| parse_enum_error(raw, e))
}

fn parse_job_run_state(raw: &str) -> rusqlite::Result<SchedulerRunState> {
    raw.parse::<SchedulerRunState>()
        .map_err(|e| parse_enum_error(raw, e))
}

fn parse_enum_error(raw: &str, message: String) -> rusqlite::Error {
    rusqlite::Error::FromSqlConversionFailure(
        raw.len(),
        rusqlite::types::Type::Text,
        Box::new(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            message,
        )),
    )
}
