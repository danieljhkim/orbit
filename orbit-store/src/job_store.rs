use chrono::{DateTime, Utc};
use orbit_types::{Job, JobStatus, OrbitError, OrbitEvent};
use rusqlite::{OptionalExtension, params};

use crate::{Store, StoreTx, new_id, now_string, parse_timestamp, status_to_str, str_to_status};

fn row_to_job(row: &rusqlite::Row<'_>) -> rusqlite::Result<Job> {
    let next_run_raw: String = row.get(3)?;
    let last_run_raw: Option<String> = row.get(4)?;
    let status_raw: String = row.get(5)?;
    Ok(Job {
        id: row.get(0)?,
        name: row.get(1)?,
        command: row.get(2)?,
        next_run_at: parse_timestamp(&next_run_raw)?,
        last_run_at: match last_run_raw {
            Some(v) => Some(parse_timestamp(&v)?),
            None => None,
        },
        status: str_to_status(&status_raw),
    })
}

impl Store {
    pub fn due_jobs(&self, now: DateTime<Utc>) -> Result<Vec<Job>, OrbitError> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| OrbitError::Store(format!("mutex poisoned: {e}")))?;

        let mut stmt = conn
            .prepare(
                "SELECT id, name, command, next_run_at, last_run_at, status FROM jobs WHERE next_run_at <= ?1 AND status = 'scheduled' ORDER BY next_run_at ASC",
            )
            .map_err(|e| OrbitError::Store(e.to_string()))?;

        let rows = stmt
            .query_map([now.to_rfc3339()], row_to_job)
            .map_err(|e| OrbitError::Store(e.to_string()))?;

        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|e| OrbitError::Store(e.to_string()))
    }

    pub fn get_job(&self, id: &str) -> Result<Option<Job>, OrbitError> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| OrbitError::Store(format!("mutex poisoned: {e}")))?;

        conn.query_row(
            "SELECT id, name, command, next_run_at, last_run_at, status FROM jobs WHERE id = ?1",
            [id],
            row_to_job,
        )
        .optional()
        .map_err(|e| OrbitError::Store(e.to_string()))
    }

    pub fn get_job_status(&self, id: &str) -> Result<Option<JobStatus>, OrbitError> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| OrbitError::Store(format!("mutex poisoned: {e}")))?;

        let status = conn
            .query_row("SELECT status FROM jobs WHERE id = ?1", [id], |row| {
                let raw: String = row.get(0)?;
                Ok(str_to_status(&raw))
            })
            .optional()
            .map_err(|e| OrbitError::Store(e.to_string()))?;

        Ok(status)
    }
}

impl<'a> StoreTx<'a> {
    pub fn claim_due_jobs(&mut self, now: DateTime<Utc>) -> Result<Vec<Job>, OrbitError> {
        let due_jobs = {
            let mut stmt = self
                .tx
                .prepare(
                    "SELECT id, name, command, next_run_at, last_run_at, status FROM jobs WHERE next_run_at <= ?1 AND status = 'scheduled' ORDER BY next_run_at ASC",
                )
                .map_err(|e| OrbitError::Store(e.to_string()))?;

            let rows = stmt
                .query_map([now.to_rfc3339()], row_to_job)
                .map_err(|e| OrbitError::Store(e.to_string()))?;

            rows.collect::<Result<Vec<_>, _>>()
                .map_err(|e| OrbitError::Store(e.to_string()))?
        };

        let mut claimed = Vec::new();
        for mut job in due_jobs {
            let changed = self
                .tx
                .execute(
                    "UPDATE jobs SET status = 'running' WHERE id = ?1 AND status = 'scheduled'",
                    params![job.id],
                )
                .map_err(|e| OrbitError::Store(e.to_string()))?;

            if changed == 1 {
                job.status = JobStatus::Running;
                self.insert_audit_event(&OrbitEvent::JobStarted { id: job.id.clone() })?;
                claimed.push(job);
            }
        }

        Ok(claimed)
    }

    pub fn insert_job(
        &mut self,
        name: &str,
        command: &str,
        next_run_at: DateTime<Utc>,
    ) -> Result<Job, OrbitError> {
        let job = Job {
            id: new_id("job"),
            name: name.to_string(),
            command: command.to_string(),
            next_run_at,
            last_run_at: None,
            status: JobStatus::Scheduled,
        };

        self.tx
            .execute(
                "INSERT INTO jobs(id, name, command, next_run_at, last_run_at, status) VALUES (?1, ?2, ?3, ?4, NULL, ?5)",
                params![job.id, job.name, job.command, job.next_run_at.to_rfc3339(), status_to_str(&job.status)],
            )
            .map_err(|e| OrbitError::Store(e.to_string()))?;

        Ok(job)
    }

    pub fn transition_job_status(
        &mut self,
        id: &str,
        from: JobStatus,
        to: JobStatus,
    ) -> Result<bool, OrbitError> {
        let changed = self
            .tx
            .execute(
                "UPDATE jobs SET status = ?1 WHERE id = ?2 AND status = ?3",
                params![status_to_str(&to), id, status_to_str(&from)],
            )
            .map_err(|e| OrbitError::Store(e.to_string()))?;
        Ok(changed == 1)
    }

    pub fn complete_job(
        &mut self,
        id: &str,
        next_run_at: DateTime<Utc>,
        success: bool,
    ) -> Result<bool, OrbitError> {
        let final_state = if success {
            JobStatus::Complete
        } else {
            JobStatus::Failed
        };

        let changed = self
            .tx
            .execute(
                "UPDATE jobs SET status = ?1, last_run_at = ?2, next_run_at = ?3 WHERE id = ?4",
                params![
                    status_to_str(&final_state),
                    now_string(),
                    next_run_at.to_rfc3339(),
                    id
                ],
            )
            .map_err(|e| OrbitError::Store(e.to_string()))?;

        Ok(changed == 1)
    }
}
