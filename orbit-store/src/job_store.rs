use chrono::{DateTime, Utc};
use orbit_types::{
    Job, JobScheduleState, JobSession, JobSessionStatus, JobTrigger, OrbitError, Role,
};
use rusqlite::{OptionalExtension, params};

use crate::{Store, StoreTx, new_id, now_string, parse_timestamp};

#[derive(Debug, Clone)]
pub struct ClaimedJobRun {
    pub job: Job,
    pub session: JobSession,
}

#[derive(Debug, Clone, Default)]
pub struct DueJobsClaim {
    pub claimed: Vec<ClaimedJobRun>,
    pub skipped: Vec<String>,
}

const JOB_COLS: &str = "job_id, name, task_id, schedule_spec, timezone, state, created_at, updated_at, paused_at, deleted_at, last_run_session_id, last_run_at, next_run_at, last_error";
const JOB_SESSION_COLS: &str = "session_id, job_id, task_id, trigger, trigger_time, started_at, finished_at, status, exit_code, error, composed_context_hash, effective_allowlist_hash, created_by_role, created_at, cancel_requested_at";

impl Store {
    pub fn list_jobs(&self, include_deleted: bool) -> Result<Vec<Job>, OrbitError> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| OrbitError::Store(format!("mutex poisoned: {e}")))?;

        let sql = if include_deleted {
            format!("SELECT {JOB_COLS} FROM jobs ORDER BY created_at DESC")
        } else {
            format!("SELECT {JOB_COLS} FROM jobs WHERE state != 'deleted' ORDER BY created_at DESC")
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

    pub fn get_job(&self, job_id: &str) -> Result<Option<Job>, OrbitError> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| OrbitError::Store(format!("mutex poisoned: {e}")))?;

        conn.query_row(
            &format!("SELECT {JOB_COLS} FROM jobs WHERE job_id = ?1"),
            [job_id],
            row_to_job,
        )
        .optional()
        .map_err(|e| OrbitError::Store(e.to_string()))
    }

    pub fn due_jobs(&self, now: DateTime<Utc>) -> Result<Vec<Job>, OrbitError> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| OrbitError::Store(format!("mutex poisoned: {e}")))?;

        let mut stmt = conn
            .prepare(&format!(
                "SELECT {JOB_COLS}
                 FROM jobs
                 WHERE state = 'active'
                   AND next_run_at IS NOT NULL
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

    pub fn list_job_sessions(&self, job_id: &str) -> Result<Vec<JobSession>, OrbitError> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| OrbitError::Store(format!("mutex poisoned: {e}")))?;

        let mut stmt = conn
            .prepare(&format!(
                "SELECT {JOB_SESSION_COLS}
                 FROM job_sessions
                 WHERE job_id = ?1
                 ORDER BY created_at DESC"
            ))
            .map_err(|e| OrbitError::Store(e.to_string()))?;
        let rows = stmt
            .query_map([job_id], row_to_job_session)
            .map_err(|e| OrbitError::Store(e.to_string()))?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|e| OrbitError::Store(e.to_string()))
    }

    pub fn get_job_session(&self, session_id: &str) -> Result<Option<JobSession>, OrbitError> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| OrbitError::Store(format!("mutex poisoned: {e}")))?;

        conn.query_row(
            &format!("SELECT {JOB_SESSION_COLS} FROM job_sessions WHERE session_id = ?1"),
            [session_id],
            row_to_job_session,
        )
        .optional()
        .map_err(|e| OrbitError::Store(e.to_string()))
    }

    pub fn get_running_job_session(&self, job_id: &str) -> Result<Option<JobSession>, OrbitError> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| OrbitError::Store(format!("mutex poisoned: {e}")))?;

        conn.query_row(
            &format!(
                "SELECT {JOB_SESSION_COLS}
                 FROM job_sessions
                 WHERE job_id = ?1 AND status = 'running'
                 ORDER BY created_at DESC
                 LIMIT 1"
            ),
            [job_id],
            row_to_job_session,
        )
        .optional()
        .map_err(|e| OrbitError::Store(e.to_string()))
    }

    pub fn is_job_session_cancel_requested(&self, session_id: &str) -> Result<bool, OrbitError> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| OrbitError::Store(format!("mutex poisoned: {e}")))?;

        let is_requested: Option<i64> = conn
            .query_row(
                "SELECT CASE WHEN cancel_requested_at IS NULL THEN 0 ELSE 1 END FROM job_sessions WHERE session_id = ?1",
                [session_id],
                |row| row.get(0),
            )
            .optional()
            .map_err(|e| OrbitError::Store(e.to_string()))?;

        Ok(is_requested.unwrap_or(0) == 1)
    }
}

impl<'a> StoreTx<'a> {
    pub fn insert_job(
        &mut self,
        name: &str,
        task_id: &str,
        schedule_spec: &str,
        timezone: &str,
        next_run_at: Option<DateTime<Utc>>,
    ) -> Result<Job, OrbitError> {
        let now = Utc::now();
        let job = Job {
            job_id: new_id("job"),
            name: name.to_string(),
            task_id: task_id.to_string(),
            schedule_spec: schedule_spec.to_string(),
            timezone: timezone.to_string(),
            state: JobScheduleState::Active,
            created_at: now,
            updated_at: now,
            paused_at: None,
            deleted_at: None,
            last_run_session_id: None,
            last_run_at: None,
            next_run_at,
            last_error: None,
        };

        self.tx
            .execute(
                "INSERT INTO jobs(
                    job_id, name, task_id, schedule_spec, timezone, state, created_at, updated_at, paused_at, deleted_at, last_run_session_id, last_run_at, next_run_at, last_error
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, NULL, NULL, NULL, NULL, ?9, NULL)",
                params![
                    job.job_id,
                    job.name,
                    job.task_id,
                    job.schedule_spec,
                    job.timezone,
                    job.state.to_string(),
                    job.created_at.to_rfc3339(),
                    job.updated_at.to_rfc3339(),
                    job.next_run_at.as_ref().map(DateTime::<Utc>::to_rfc3339),
                ],
            )
            .map_err(|e| OrbitError::Store(e.to_string()))?;
        Ok(job)
    }

    pub fn update_job_next_run(
        &mut self,
        job_id: &str,
        next_run_at: Option<DateTime<Utc>>,
        last_error: Option<&str>,
    ) -> Result<bool, OrbitError> {
        let changed = self
            .tx
            .execute(
                "UPDATE jobs
                 SET next_run_at = ?1, last_error = ?2, updated_at = ?3
                 WHERE job_id = ?4",
                params![
                    next_run_at.as_ref().map(DateTime::<Utc>::to_rfc3339),
                    last_error,
                    now_string(),
                    job_id
                ],
            )
            .map_err(|e| OrbitError::Store(e.to_string()))?;
        Ok(changed == 1)
    }

    pub fn set_job_state(
        &mut self,
        job_id: &str,
        state: JobScheduleState,
    ) -> Result<bool, OrbitError> {
        let now = now_string();
        let changed = self
            .tx
            .execute(
                "UPDATE jobs
                 SET state = ?1,
                     paused_at = CASE WHEN ?1 = 'paused' THEN ?2 ELSE NULL END,
                     updated_at = ?2
                 WHERE job_id = ?3",
                params![state.to_string(), now, job_id],
            )
            .map_err(|e| OrbitError::Store(e.to_string()))?;
        Ok(changed == 1)
    }

    pub fn mark_job_deleted(&mut self, job_id: &str) -> Result<bool, OrbitError> {
        let now = now_string();
        let changed = self
            .tx
            .execute(
                "UPDATE jobs
                 SET state = 'deleted', deleted_at = ?1, updated_at = ?1
                 WHERE job_id = ?2",
                params![now, job_id],
            )
            .map_err(|e| OrbitError::Store(e.to_string()))?;
        Ok(changed == 1)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn insert_job_session(
        &mut self,
        job_id: &str,
        task_id: &str,
        trigger: JobTrigger,
        created_by_role: Role,
        trigger_time: DateTime<Utc>,
        composed_context_hash: Option<&str>,
        effective_allowlist_hash: Option<&str>,
    ) -> Result<JobSession, OrbitError> {
        let now = Utc::now();
        let session = JobSession {
            session_id: new_id("jsess"),
            job_id: job_id.to_string(),
            task_id: task_id.to_string(),
            trigger,
            trigger_time,
            started_at: Some(now),
            finished_at: None,
            status: JobSessionStatus::Running,
            exit_code: None,
            error: None,
            composed_context_hash: composed_context_hash.map(ToString::to_string),
            effective_allowlist_hash: effective_allowlist_hash.map(ToString::to_string),
            created_by_role,
            created_at: now,
            cancel_requested_at: None,
        };

        self.tx
            .execute(
                "INSERT INTO job_sessions(
                    session_id, job_id, task_id, trigger, trigger_time, started_at, finished_at,
                    status, exit_code, error, composed_context_hash,
                    effective_allowlist_hash, created_by_role, created_at, cancel_requested_at
                 ) VALUES (
                    ?1, ?2, ?3, ?4, ?5, ?6, NULL,
                    ?7, NULL, NULL, ?8,
                    ?9, ?10, ?11, NULL
                 )",
                params![
                    session.session_id,
                    session.job_id,
                    session.task_id,
                    session.trigger.to_string(),
                    session.trigger_time.to_rfc3339(),
                    session.started_at.as_ref().map(DateTime::<Utc>::to_rfc3339),
                    session.status.to_string(),
                    session.composed_context_hash,
                    session.effective_allowlist_hash,
                    session.created_by_role.to_string(),
                    session.created_at.to_rfc3339(),
                ],
            )
            .map_err(|e| OrbitError::Store(e.to_string()))?;

        self.tx
            .execute(
                "UPDATE jobs SET last_run_session_id = ?1, last_run_at = ?2, updated_at = ?2 WHERE job_id = ?3",
                params![session.session_id, session.started_at.as_ref().map(DateTime::<Utc>::to_rfc3339), job_id],
            )
            .map_err(|e| OrbitError::Store(e.to_string()))?;

        Ok(session)
    }

    pub fn finish_job_session(
        &mut self,
        session_id: &str,
        status: JobSessionStatus,
        exit_code: Option<i32>,
        error: Option<&str>,
    ) -> Result<bool, OrbitError> {
        let finished_at = now_string();
        let changed = self
            .tx
            .execute(
                "UPDATE job_sessions
                 SET status = ?1, exit_code = ?2, error = ?3, finished_at = ?4
                 WHERE session_id = ?5",
                params![
                    status.to_string(),
                    exit_code,
                    error,
                    finished_at,
                    session_id
                ],
            )
            .map_err(|e| OrbitError::Store(e.to_string()))?;
        Ok(changed == 1)
    }

    pub fn request_cancel_running_session(
        &mut self,
        job_id: &str,
    ) -> Result<Option<String>, OrbitError> {
        let session_id: Option<String> = self
            .tx
            .query_row(
                "SELECT session_id FROM job_sessions WHERE job_id = ?1 AND status = 'running' ORDER BY created_at DESC LIMIT 1",
                [job_id],
                |row| row.get(0),
            )
            .optional()
            .map_err(|e| OrbitError::Store(e.to_string()))?;

        let Some(session_id) = session_id else {
            return Ok(None);
        };

        self.tx
            .execute(
                "UPDATE job_sessions SET cancel_requested_at = ?1 WHERE session_id = ?2",
                params![now_string(), session_id],
            )
            .map_err(|e| OrbitError::Store(e.to_string()))?;

        Ok(Some(session_id))
    }

    pub fn claim_due_jobs(&mut self, now: DateTime<Utc>) -> Result<DueJobsClaim, OrbitError> {
        let due_jobs = {
            let mut stmt = self
                .tx
                .prepare(&format!(
                    "SELECT {JOB_COLS}
                     FROM jobs
                     WHERE state = 'active'
                       AND next_run_at IS NOT NULL
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
        for job in due_jobs {
            let running_exists: Option<String> = self
                .tx
                .query_row(
                    "SELECT session_id FROM job_sessions WHERE job_id = ?1 AND status = 'running' LIMIT 1",
                    [job.job_id.clone()],
                    |row| row.get(0),
                )
                .optional()
                .map_err(|e| OrbitError::Store(e.to_string()))?;

            if running_exists.is_some() {
                result.skipped.push(job.job_id.clone());
                continue;
            }

            let session = self.insert_job_session(
                &job.job_id,
                &job.task_id,
                JobTrigger::Schedule,
                Role::Admin,
                now,
                None,
                None,
            )?;
            result.claimed.push(ClaimedJobRun { job, session });
        }

        Ok(result)
    }
}

fn row_to_job(row: &rusqlite::Row<'_>) -> rusqlite::Result<Job> {
    let state_raw: String = row.get(5)?;
    let created_at_raw: String = row.get(6)?;
    let updated_at_raw: String = row.get(7)?;
    let paused_at_raw: Option<String> = row.get(8)?;
    let deleted_at_raw: Option<String> = row.get(9)?;
    let last_run_at_raw: Option<String> = row.get(11)?;
    let next_run_at_raw: Option<String> = row.get(12)?;

    Ok(Job {
        job_id: row.get(0)?,
        name: row.get(1)?,
        task_id: row.get(2)?,
        schedule_spec: row.get(3)?,
        timezone: row.get(4)?,
        state: parse_job_state(&state_raw)?,
        created_at: parse_timestamp(&created_at_raw)?,
        updated_at: parse_timestamp(&updated_at_raw)?,
        paused_at: parse_optional_timestamp(paused_at_raw)?,
        deleted_at: parse_optional_timestamp(deleted_at_raw)?,
        last_run_session_id: row.get(10)?,
        last_run_at: parse_optional_timestamp(last_run_at_raw)?,
        next_run_at: parse_optional_timestamp(next_run_at_raw)?,
        last_error: row.get(13)?,
    })
}

fn row_to_job_session(row: &rusqlite::Row<'_>) -> rusqlite::Result<JobSession> {
    let trigger_raw: String = row.get(3)?;
    let trigger_time_raw: String = row.get(4)?;
    let started_at_raw: Option<String> = row.get(5)?;
    let finished_at_raw: Option<String> = row.get(6)?;
    let status_raw: String = row.get(7)?;
    let created_by_role_raw: String = row.get(12)?;
    let created_at_raw: String = row.get(13)?;
    let cancel_requested_at_raw: Option<String> = row.get(14)?;

    Ok(JobSession {
        session_id: row.get(0)?,
        job_id: row.get(1)?,
        task_id: row.get(2)?,
        trigger: parse_job_trigger(&trigger_raw)?,
        trigger_time: parse_timestamp(&trigger_time_raw)?,
        started_at: parse_optional_timestamp(started_at_raw)?,
        finished_at: parse_optional_timestamp(finished_at_raw)?,
        status: parse_job_session_status(&status_raw)?,
        exit_code: row.get(8)?,
        error: row.get(9)?,
        composed_context_hash: row.get(10)?,
        effective_allowlist_hash: row.get(11)?,
        created_by_role: parse_role(&created_by_role_raw)?,
        created_at: parse_timestamp(&created_at_raw)?,
        cancel_requested_at: parse_optional_timestamp(cancel_requested_at_raw)?,
    })
}

fn parse_optional_timestamp(raw: Option<String>) -> rusqlite::Result<Option<DateTime<Utc>>> {
    raw.map(|value| parse_timestamp(&value)).transpose()
}

fn parse_job_state(raw: &str) -> rusqlite::Result<JobScheduleState> {
    raw.parse::<JobScheduleState>()
        .map_err(|e| parse_enum_error(raw, e))
}

fn parse_job_session_status(raw: &str) -> rusqlite::Result<JobSessionStatus> {
    raw.parse::<JobSessionStatus>()
        .map_err(|e| parse_enum_error(raw, e))
}

fn parse_job_trigger(raw: &str) -> rusqlite::Result<JobTrigger> {
    raw.parse::<JobTrigger>()
        .map_err(|e| parse_enum_error(raw, e))
}

fn parse_role(raw: &str) -> rusqlite::Result<Role> {
    raw.parse::<Role>().map_err(|e| parse_enum_error(raw, e))
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
