use std::str::FromStr;

use chrono::{DateTime, Utc};
use orbit_common::types::{
    Crew, JobRun, JobRunState, JobRunStep, JobTargetType, KnowledgeRunMetrics, NotFoundKind,
    OrbitError, PipelineState, RunEvent,
};
use orbit_common::utility::process_identity::process_start_identity_token;
use rusqlite::TransactionBehavior;

use crate::backend::{JobRunQuery, JobRunStepParams, JobRunStoreBackend};
use crate::file::layout::validate_path_stem;
use crate::{Store, parse_timestamp};

#[derive(Clone)]
pub struct SqliteJobRunStore {
    store: Store,
    workspace_id: String,
}

impl SqliteJobRunStore {
    pub fn new(store: Store, workspace_id: impl Into<String>) -> Self {
        Self {
            store,
            workspace_id: workspace_id.into(),
        }
    }

    fn read_run(&self, run_id: &str) -> Result<Option<JobRun>, OrbitError> {
        self.store
            .get_job_run_for_workspace(&self.workspace_id, run_id)
    }

    fn update_run(
        &self,
        run_id: &str,
        update: impl FnOnce(&mut JobRun) -> Result<(), OrbitError>,
    ) -> Result<bool, OrbitError> {
        self.store
            .with_transaction_behavior(TransactionBehavior::Immediate, |tx| {
                let Some(mut run) =
                    get_job_run_for_workspace_conn(&tx.tx, &self.workspace_id, run_id)?
                else {
                    return Ok(false);
                };
                update(&mut run)?;
                upsert_job_run_for_workspace_conn(&tx.tx, &self.workspace_id, &run, None)?;
                Ok(true)
            })
    }

    fn next_run_id(&self, job_id: &str) -> Result<String, OrbitError> {
        let base = format!("jrun-{}", Utc::now().format("%Y%m%d-%H%M"));
        for suffix in 1..1024_u32 {
            let candidate = if suffix == 1 {
                base.clone()
            } else {
                format!("{base}-{suffix}")
            };
            if self
                .store
                .get_job_run_for_workspace(&self.workspace_id, &candidate)?
                .is_none()
            {
                return Ok(candidate);
            }
        }
        Ok(format!("{base}-{job_id}"))
    }
}

impl JobRunStoreBackend for SqliteJobRunStore {
    fn list_job_runs(&self, job_id: &str) -> Result<Vec<JobRun>, OrbitError> {
        validate_path_stem(job_id, "job")?;
        self.list_job_runs_filtered(&JobRunQuery {
            job_id: Some(job_id.to_string()),
            ..Default::default()
        })
    }

    fn list_job_runs_filtered(&self, query: &JobRunQuery) -> Result<Vec<JobRun>, OrbitError> {
        self.store
            .list_job_runs_for_workspace(&self.workspace_id, query)
    }

    fn get_job_run(&self, run_id: &str) -> Result<Option<JobRun>, OrbitError> {
        self.read_run(run_id)
    }

    fn list_pending_or_running_job_runs(&self, job_id: &str) -> Result<Vec<JobRun>, OrbitError> {
        validate_path_stem(job_id, "job")?;
        let mut runs = self.store.list_job_runs_for_workspace(
            &self.workspace_id,
            &JobRunQuery {
                job_id: Some(job_id.to_string()),
                ..Default::default()
            },
        )?;
        runs.retain(|run| matches!(run.state, JobRunState::Pending | JobRunState::Running));
        runs.sort_by_key(|run| std::cmp::Reverse(run.created_at));
        Ok(runs)
    }

    fn insert_job_run(
        &self,
        job_id: &str,
        attempt: u32,
        scheduled_at: DateTime<Utc>,
        input: Option<serde_json::Value>,
        retry_source_run_id: Option<String>,
    ) -> Result<JobRun, OrbitError> {
        validate_path_stem(job_id, "job")?;
        let run = JobRun {
            run_id: self.next_run_id(job_id)?,
            job_id: job_id.to_string(),
            attempt,
            state: JobRunState::Pending,
            scheduled_at,
            started_at: None,
            finished_at: None,
            duration_ms: None,
            created_at: Utc::now(),
            pid: None,
            pid_start_time: None,
            input,
            retry_source_run_id,
            knowledge_metrics: None,
            resolved_crew: None,
            planner_model: None,
            implementer_model: None,
            reviewer_model: None,
            steps: Vec::new(),
        };
        self.store
            .upsert_job_run_for_workspace(&self.workspace_id, &run, None)?;
        Ok(run)
    }

    fn mark_job_run_running(
        &self,
        run_id: &str,
        started_at: DateTime<Utc>,
        pid: u32,
    ) -> Result<bool, OrbitError> {
        self.update_run(run_id, |run| {
            run.state = run
                .state
                .try_transition(RunEvent::Start)
                .map_err(OrbitError::JobRunStateTransition)?;
            run.started_at = Some(started_at);
            run.pid = Some(pid);
            run.pid_start_time = process_start_identity_token(pid);
            Ok(())
        })
    }

    fn take_over_running_job_run(
        &self,
        run_id: &str,
        expected_pid: Option<u32>,
        expected_pid_start_time: Option<String>,
        started_at: DateTime<Utc>,
        pid: u32,
    ) -> Result<bool, OrbitError> {
        self.update_run(run_id, |run| {
            if run.state != JobRunState::Running
                || run.pid != expected_pid
                || run.pid_start_time != expected_pid_start_time
            {
                return Err(OrbitError::InvalidInput(
                    "job run takeover mismatch".to_string(),
                ));
            }
            run.started_at = run.started_at.or(Some(started_at));
            run.pid = Some(pid);
            run.pid_start_time = process_start_identity_token(pid);
            Ok(())
        })
        .or_else(|err| match err {
            OrbitError::InvalidInput(message) if message == "job run takeover mismatch" => {
                Ok(false)
            }
            other => Err(other),
        })
    }

    fn abandon_job_run(
        &self,
        run_id: &str,
        finished_at: DateTime<Utc>,
    ) -> Result<bool, OrbitError> {
        self.update_run(run_id, |run| {
            if run.state.is_terminal() {
                return Ok(());
            }
            run.state = run
                .state
                .try_transition(RunEvent::Abandon)
                .map_err(OrbitError::JobRunStateTransition)?;
            run.finished_at = Some(finished_at);
            Ok(())
        })
    }

    fn complete_job_run_step(
        &self,
        run_id: &str,
        params: &JobRunStepParams,
    ) -> Result<bool, OrbitError> {
        if self.read_run(run_id)?.is_none() {
            return Ok(false);
        }
        params
            .state
            .validate_step_state()
            .map_err(OrbitError::JobRunStateTransition)?;
        let step = JobRunStep {
            step_index: params.step_index as u32,
            target_type: params.target_type,
            target_id: params.target_id.clone(),
            started_at: Some(params.started_at),
            finished_at: Some(params.finished_at),
            duration_ms: params.duration_ms,
            exit_code: params.exit_code,
            agent_response_json: params.agent_response_json.clone(),
            state: params.state,
            error_code: params.error_code.clone(),
            error_message: params.error_message.clone(),
        };
        self.store
            .upsert_job_run_step_for_workspace(&self.workspace_id, run_id, &step)?;
        Ok(true)
    }

    fn record_job_run_knowledge_metrics(
        &self,
        run_id: &str,
        metrics: KnowledgeRunMetrics,
    ) -> Result<bool, OrbitError> {
        self.update_run(run_id, |run| {
            run.knowledge_metrics = Some(metrics);
            Ok(())
        })
    }

    fn record_job_run_crew(&self, run_id: &str, crew: &Crew) -> Result<bool, OrbitError> {
        self.update_run(run_id, |run| {
            run.resolved_crew = Some(crew.name.clone());
            run.planner_model = Some(crew.planner.model.clone());
            run.implementer_model = Some(crew.implementer.model.clone());
            run.reviewer_model = Some(crew.reviewer.model.clone());
            Ok(())
        })
    }

    fn finalize_job_run(
        &self,
        run_id: &str,
        state: JobRunState,
        finished_at: DateTime<Utc>,
        duration_ms: Option<u64>,
    ) -> Result<bool, OrbitError> {
        self.update_run(run_id, |run| {
            if run.state.is_terminal() {
                return Ok(());
            }
            let event = match state {
                JobRunState::Success => RunEvent::Complete,
                JobRunState::Failed => RunEvent::Fail,
                JobRunState::Timeout => RunEvent::Timeout,
                JobRunState::Cancelled => RunEvent::Cancel,
                other => {
                    return Err(OrbitError::JobRunStateTransition(format!(
                        "cannot finalize to non-terminal state: {other}"
                    )));
                }
            };
            run.state = run
                .state
                .try_transition(event)
                .map_err(OrbitError::JobRunStateTransition)?;
            run.finished_at = Some(finished_at);
            run.duration_ms = duration_ms;
            Ok(())
        })
    }

    fn repair_terminal_job_run_timing(
        &self,
        run_id: &str,
        finished_at: DateTime<Utc>,
        duration_ms: Option<u64>,
    ) -> Result<bool, OrbitError> {
        let mut changed = false;
        let found = self.update_run(run_id, |run| {
            if !run.state.is_terminal() {
                return Ok(());
            }
            if run.finished_at.is_none() {
                run.finished_at = Some(finished_at);
                changed = true;
            }
            if run.duration_ms.is_none() {
                run.duration_ms = duration_ms;
                changed = true;
            }
            Ok(())
        })?;
        Ok(found && changed)
    }

    fn list_all_pending_or_running_runs(&self) -> Result<Vec<JobRun>, OrbitError> {
        let mut runs = self.store.list_job_runs_for_workspace(
            &self.workspace_id,
            &JobRunQuery {
                ..Default::default()
            },
        )?;
        runs.retain(|run| matches!(run.state, JobRunState::Pending | JobRunState::Running));
        runs.sort_by_key(|run| std::cmp::Reverse(run.created_at));
        Ok(runs)
    }

    fn archive_job_run(&self, run_id: &str) -> Result<String, OrbitError> {
        let run = self
            .read_run(run_id)?
            .ok_or_else(|| OrbitError::not_found(NotFoundKind::JobRun, run_id.to_string()))?;
        self.store
            .delete_job_run_for_workspace(&self.workspace_id, run_id)?;
        Ok(run.job_id)
    }

    fn delete_job_run(&self, run_id: &str) -> Result<String, OrbitError> {
        let run = self
            .read_run(run_id)?
            .ok_or_else(|| OrbitError::not_found(NotFoundKind::JobRun, run_id.to_string()))?;
        self.store
            .delete_job_run_for_workspace(&self.workspace_id, run_id)?;
        Ok(run.job_id)
    }

    fn read_run_state(&self, run_id: &str) -> Result<Option<PipelineState>, OrbitError> {
        self.store
            .read_job_run_state_for_workspace(&self.workspace_id, run_id)
    }

    fn write_run_state(&self, run_id: &str, state: &PipelineState) -> Result<(), OrbitError> {
        self.store
            .write_job_run_state_for_workspace(&self.workspace_id, run_id, state)
    }
}

impl Store {
    pub fn upsert_job_run_for_workspace(
        &self,
        workspace_id: &str,
        run: &JobRun,
        pipeline_state: Option<&PipelineState>,
    ) -> Result<(), OrbitError> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| OrbitError::Store(format!("mutex poisoned: {e}")))?;
        upsert_job_run_for_workspace_conn(&conn, workspace_id, run, pipeline_state)
    }

    pub fn upsert_job_run_step_for_workspace(
        &self,
        workspace_id: &str,
        run_id: &str,
        step: &JobRunStep,
    ) -> Result<(), OrbitError> {
        let agent_response_json = optional_json(&step.agent_response_json, "agent response")?;
        let conn = self
            .conn
            .lock()
            .map_err(|e| OrbitError::Store(format!("mutex poisoned: {e}")))?;
        conn.execute(
            r#"INSERT INTO job_run_steps(
                workspace_id, run_id, step_index, target_type, target_id, state,
                started_at, finished_at, duration_ms, exit_code, error_code,
                error_message, agent_response_json
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)
            ON CONFLICT(workspace_id, run_id, step_index) DO UPDATE SET
                target_type = excluded.target_type,
                target_id = excluded.target_id,
                state = excluded.state,
                started_at = excluded.started_at,
                finished_at = excluded.finished_at,
                duration_ms = excluded.duration_ms,
                exit_code = excluded.exit_code,
                error_code = excluded.error_code,
                error_message = excluded.error_message,
                agent_response_json = excluded.agent_response_json"#,
            rusqlite::params![
                workspace_id,
                run_id,
                i64::from(step.step_index),
                step.target_type.to_string(),
                step.target_id,
                step.state.to_string(),
                step.started_at.map(|ts| ts.to_rfc3339()),
                step.finished_at.map(|ts| ts.to_rfc3339()),
                step.duration_ms.map(|value| value as i64),
                step.exit_code,
                step.error_code,
                step.error_message,
                agent_response_json,
            ],
        )
        .map_err(|e| OrbitError::Store(e.to_string()))?;
        Ok(())
    }

    pub fn get_job_run_for_workspace(
        &self,
        workspace_id: &str,
        run_id: &str,
    ) -> Result<Option<JobRun>, OrbitError> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| OrbitError::Store(format!("mutex poisoned: {e}")))?;
        get_job_run_for_workspace_conn(&conn, workspace_id, run_id)
    }

    pub fn list_job_runs_for_workspace(
        &self,
        workspace_id: &str,
        query: &JobRunQuery,
    ) -> Result<Vec<JobRun>, OrbitError> {
        let mut conditions = vec!["workspace_id = ?1".to_string()];
        let mut params: Vec<Box<dyn rusqlite::types::ToSql>> =
            vec![Box::new(workspace_id.to_string())];
        if let Some(job_id) = &query.job_id {
            conditions.push(format!("job_id = ?{}", params.len() + 1));
            params.push(Box::new(job_id.clone()));
        }
        if let Some(state) = query.state {
            conditions.push(format!("state = ?{}", params.len() + 1));
            params.push(Box::new(state.to_string()));
        }
        if let Some(created_since) = query.created_since {
            conditions.push(format!("created_at >= ?{}", params.len() + 1));
            params.push(Box::new(created_since.to_rfc3339()));
        }
        let mut sql = format!(
            "SELECT run_id, job_id, attempt, state, scheduled_at, started_at, finished_at, \
             duration_ms, created_at, pid, pid_start_time, input_json, retry_source_run_id, \
             knowledge_metrics_json, resolved_crew, planner_model, implementer_model, reviewer_model \
             FROM job_runs WHERE {} ORDER BY created_at DESC, run_id ASC",
            conditions.join(" AND ")
        );
        if let Some(limit) = query.limit {
            sql.push_str(&format!(" LIMIT ?{}", params.len() + 1));
            params.push(Box::new(limit as i64));
        }
        let param_refs: Vec<&dyn rusqlite::types::ToSql> =
            params.iter().map(|b| b.as_ref()).collect();
        let conn = self
            .conn
            .lock()
            .map_err(|e| OrbitError::Store(format!("mutex poisoned: {e}")))?;
        let mut stmt = conn
            .prepare(&sql)
            .map_err(|e| OrbitError::Store(e.to_string()))?;
        let rows = stmt
            .query_map(param_refs.as_slice(), row_to_job_run)
            .map_err(|e| OrbitError::Store(e.to_string()))?;
        let mut runs = rows
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| OrbitError::Store(e.to_string()))?;
        for run in &mut runs {
            run.steps = read_steps(&conn, workspace_id, &run.run_id)?;
        }
        Ok(runs)
    }

    pub fn read_job_run_state_for_workspace(
        &self,
        workspace_id: &str,
        run_id: &str,
    ) -> Result<Option<PipelineState>, OrbitError> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| OrbitError::Store(format!("mutex poisoned: {e}")))?;
        let raw = match conn.query_row(
            "SELECT pipeline_state_json FROM job_runs WHERE workspace_id = ?1 AND run_id = ?2",
            rusqlite::params![workspace_id, run_id],
            |row| row.get::<_, Option<String>>(0),
        ) {
            Ok(raw) => raw,
            Err(rusqlite::Error::QueryReturnedNoRows) => return Ok(None),
            Err(err) => return Err(OrbitError::Store(err.to_string())),
        };
        raw.map(|raw| {
            serde_json::from_str(&raw)
                .map_err(|e| OrbitError::Store(format!("invalid pipeline_state_json: {e}")))
        })
        .transpose()
    }

    pub fn write_job_run_state_for_workspace(
        &self,
        workspace_id: &str,
        run_id: &str,
        state: &PipelineState,
    ) -> Result<(), OrbitError> {
        let state_json = serde_json::to_string_pretty(state)
            .map_err(|e| OrbitError::Store(format!("serialize pipeline state: {e}")))?;
        let conn = self
            .conn
            .lock()
            .map_err(|e| OrbitError::Store(format!("mutex poisoned: {e}")))?;
        let updated = conn
            .execute(
                "UPDATE job_runs SET pipeline_state_json = ?3 WHERE workspace_id = ?1 AND run_id = ?2",
                rusqlite::params![workspace_id, run_id, state_json],
            )
            .map_err(|e| OrbitError::Store(e.to_string()))?;
        if updated == 0 {
            return Err(OrbitError::not_found(
                NotFoundKind::JobRun,
                run_id.to_string(),
            ));
        }
        Ok(())
    }

    pub fn delete_job_run_for_workspace(
        &self,
        workspace_id: &str,
        run_id: &str,
    ) -> Result<bool, OrbitError> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| OrbitError::Store(format!("mutex poisoned: {e}")))?;
        conn.execute(
            "DELETE FROM job_runs WHERE workspace_id = ?1 AND run_id = ?2",
            rusqlite::params![workspace_id, run_id],
        )
        .map(|count| count > 0)
        .map_err(|e| OrbitError::Store(e.to_string()))
    }
}

fn upsert_job_run_for_workspace_conn(
    conn: &rusqlite::Connection,
    workspace_id: &str,
    run: &JobRun,
    pipeline_state: Option<&PipelineState>,
) -> Result<(), OrbitError> {
    let input_json = optional_json(&run.input, "job run input")?;
    let knowledge_metrics_json =
        optional_json(&run.knowledge_metrics, "job run knowledge metrics")?;
    let pipeline_state_json = optional_json(&pipeline_state, "job run pipeline state")?;
    conn.execute(
        r#"INSERT INTO job_runs(
            run_id, workspace_id, job_id, attempt, state, scheduled_at,
            started_at, finished_at, duration_ms, created_at, pid, pid_start_time,
            input_json, retry_source_run_id, knowledge_metrics_json, resolved_crew,
            planner_model, implementer_model, reviewer_model, pipeline_state_json
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20)
        ON CONFLICT(workspace_id, run_id) DO UPDATE SET
            job_id = excluded.job_id,
            attempt = excluded.attempt,
            state = excluded.state,
            scheduled_at = excluded.scheduled_at,
            started_at = excluded.started_at,
            finished_at = excluded.finished_at,
            duration_ms = excluded.duration_ms,
            created_at = excluded.created_at,
            pid = excluded.pid,
            pid_start_time = excluded.pid_start_time,
            input_json = excluded.input_json,
            retry_source_run_id = excluded.retry_source_run_id,
            knowledge_metrics_json = excluded.knowledge_metrics_json,
            resolved_crew = excluded.resolved_crew,
            planner_model = excluded.planner_model,
            implementer_model = excluded.implementer_model,
            reviewer_model = excluded.reviewer_model,
            pipeline_state_json = COALESCE(excluded.pipeline_state_json, job_runs.pipeline_state_json)"#,
        rusqlite::params![
            run.run_id,
            workspace_id,
            run.job_id,
            i64::from(run.attempt),
            run.state.to_string(),
            run.scheduled_at.to_rfc3339(),
            run.started_at.map(|ts| ts.to_rfc3339()),
            run.finished_at.map(|ts| ts.to_rfc3339()),
            run.duration_ms.map(|value| value as i64),
            run.created_at.to_rfc3339(),
            run.pid.map(i64::from),
            run.pid_start_time,
            input_json,
            run.retry_source_run_id,
            knowledge_metrics_json,
            run.resolved_crew,
            run.planner_model,
            run.implementer_model,
            run.reviewer_model,
            pipeline_state_json,
        ],
    )
    .map_err(|e| OrbitError::Store(e.to_string()))?;
    Ok(())
}

fn get_job_run_for_workspace_conn(
    conn: &rusqlite::Connection,
    workspace_id: &str,
    run_id: &str,
) -> Result<Option<JobRun>, OrbitError> {
    let mut stmt = conn
        .prepare(
            "SELECT run_id, job_id, attempt, state, scheduled_at, started_at, finished_at, \
             duration_ms, created_at, pid, pid_start_time, input_json, retry_source_run_id, \
             knowledge_metrics_json, resolved_crew, planner_model, implementer_model, reviewer_model \
             FROM job_runs WHERE workspace_id = ?1 AND run_id = ?2",
        )
        .map_err(|e| OrbitError::Store(e.to_string()))?;
    let mut run = match stmt.query_row(rusqlite::params![workspace_id, run_id], row_to_job_run) {
        Ok(run) => run,
        Err(rusqlite::Error::QueryReturnedNoRows) => return Ok(None),
        Err(err) => return Err(OrbitError::Store(err.to_string())),
    };
    run.steps = read_steps(conn, workspace_id, run_id)?;
    Ok(Some(run))
}

fn row_to_job_run(row: &rusqlite::Row<'_>) -> rusqlite::Result<JobRun> {
    let attempt: i64 = row.get(2)?;
    let state_raw: String = row.get(3)?;
    let scheduled_raw: String = row.get(4)?;
    let started_raw: Option<String> = row.get(5)?;
    let finished_raw: Option<String> = row.get(6)?;
    let duration_ms: Option<i64> = row.get(7)?;
    let created_raw: String = row.get(8)?;
    let pid: Option<i64> = row.get(9)?;
    let input_json: Option<String> = row.get(11)?;
    let knowledge_metrics_json: Option<String> = row.get(13)?;
    Ok(JobRun {
        run_id: row.get(0)?,
        job_id: row.get(1)?,
        attempt: attempt as u32,
        state: parse_job_run_state(&state_raw)?,
        scheduled_at: parse_timestamp(&scheduled_raw)?,
        started_at: parse_optional_timestamp(started_raw)?,
        finished_at: parse_optional_timestamp(finished_raw)?,
        duration_ms: duration_ms.map(|value| value as u64),
        created_at: parse_timestamp(&created_raw)?,
        pid: pid.map(|value| value as u32),
        pid_start_time: row.get(10)?,
        input: parse_optional_json(input_json, "input_json")?,
        retry_source_run_id: row.get(12)?,
        knowledge_metrics: parse_optional_json(knowledge_metrics_json, "knowledge_metrics_json")?,
        resolved_crew: row.get(14)?,
        planner_model: row.get(15)?,
        implementer_model: row.get(16)?,
        reviewer_model: row.get(17)?,
        steps: Vec::new(),
    })
}

fn read_steps(
    conn: &rusqlite::Connection,
    workspace_id: &str,
    run_id: &str,
) -> Result<Vec<JobRunStep>, OrbitError> {
    let mut stmt = conn
        .prepare(
            "SELECT step_index, target_type, target_id, state, started_at, finished_at, \
             duration_ms, exit_code, error_code, error_message, agent_response_json \
             FROM job_run_steps WHERE workspace_id = ?1 AND run_id = ?2 ORDER BY step_index ASC",
        )
        .map_err(|e| OrbitError::Store(e.to_string()))?;
    let rows = stmt
        .query_map(rusqlite::params![workspace_id, run_id], row_to_job_run_step)
        .map_err(|e| OrbitError::Store(e.to_string()))?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|e| OrbitError::Store(e.to_string()))
}

fn row_to_job_run_step(row: &rusqlite::Row<'_>) -> rusqlite::Result<JobRunStep> {
    let step_index: i64 = row.get(0)?;
    let target_type_raw: String = row.get(1)?;
    let state_raw: String = row.get(3)?;
    let started_raw: Option<String> = row.get(4)?;
    let finished_raw: Option<String> = row.get(5)?;
    let duration_ms: Option<i64> = row.get(6)?;
    let agent_response_json: Option<String> = row.get(10)?;
    Ok(JobRunStep {
        step_index: step_index as u32,
        target_type: parse_job_target_type(&target_type_raw)?,
        target_id: row.get(2)?,
        state: parse_job_run_state(&state_raw)?,
        started_at: parse_optional_timestamp(started_raw)?,
        finished_at: parse_optional_timestamp(finished_raw)?,
        duration_ms: duration_ms.map(|value| value as u64),
        exit_code: row.get(7)?,
        error_code: row.get(8)?,
        error_message: row.get(9)?,
        agent_response_json: parse_optional_json(agent_response_json, "agent_response_json")?,
    })
}

fn optional_json<T: serde::Serialize>(
    value: &Option<T>,
    label: &str,
) -> Result<Option<String>, OrbitError> {
    value
        .as_ref()
        .map(|value| {
            serde_json::to_string(value)
                .map_err(|e| OrbitError::Store(format!("serialize {label}: {e}")))
        })
        .transpose()
}

fn parse_optional_json<T: serde::de::DeserializeOwned>(
    raw: Option<String>,
    label: &str,
) -> rusqlite::Result<Option<T>> {
    raw.map(|raw| {
        serde_json::from_str(&raw).map_err(|e| {
            rusqlite::Error::FromSqlConversionFailure(
                raw.len(),
                rusqlite::types::Type::Text,
                Box::new(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    format!("invalid {label}: {e}"),
                )),
            )
        })
    })
    .transpose()
}

fn parse_optional_timestamp(raw: Option<String>) -> rusqlite::Result<Option<DateTime<Utc>>> {
    raw.map(|raw| parse_timestamp(&raw)).transpose()
}

fn parse_job_run_state(raw: &str) -> rusqlite::Result<JobRunState> {
    JobRunState::from_str(raw).map_err(|e| {
        rusqlite::Error::FromSqlConversionFailure(
            raw.len(),
            rusqlite::types::Type::Text,
            Box::new(std::io::Error::new(std::io::ErrorKind::InvalidData, e)),
        )
    })
}

fn parse_job_target_type(raw: &str) -> rusqlite::Result<JobTargetType> {
    JobTargetType::from_str(raw).map_err(|e| {
        rusqlite::Error::FromSqlConversionFailure(
            raw.len(),
            rusqlite::types::Type::Text,
            Box::new(std::io::Error::new(std::io::ErrorKind::InvalidData, e)),
        )
    })
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Barrier};
    use std::thread;
    use std::time::Duration;

    use chrono::Utc;
    use orbit_common::types::JobTargetType;
    use tempfile::TempDir;

    use super::*;

    #[test]
    fn job_run_lifecycle_round_trips() {
        let backend = SqliteJobRunStore::new(Store::open_in_memory().expect("store"), "ws_a");
        let scheduled_at = Utc::now();
        let run = backend
            .insert_job_run("job-a", 1, scheduled_at, None, None)
            .expect("insert");
        assert_eq!(run.state, JobRunState::Pending);

        assert!(
            backend
                .mark_job_run_running(&run.run_id, scheduled_at, 42)
                .expect("running")
        );
        let step_params = JobRunStepParams {
            step_index: 0,
            target_type: JobTargetType::Activity,
            target_id: "activity-a".to_string(),
            started_at: scheduled_at,
            finished_at: scheduled_at,
            duration_ms: Some(7),
            exit_code: Some(0),
            agent_response_json: Some(serde_json::json!({"ok": true})),
            state: JobRunState::Success,
            error_code: None,
            error_message: None,
        };
        assert!(
            backend
                .complete_job_run_step(&run.run_id, &step_params)
                .expect("step")
        );
        assert!(
            backend
                .finalize_job_run(&run.run_id, JobRunState::Success, scheduled_at, Some(7))
                .expect("finalize")
        );
        let loaded = backend
            .get_job_run(&run.run_id)
            .expect("get")
            .expect("some");
        assert_eq!(loaded.state, JobRunState::Success);
        assert_eq!(loaded.steps.len(), 1);
    }

    #[test]
    fn update_run_serializes_concurrent_mutations_without_torn_write() {
        let temp = TempDir::new().expect("tempdir");
        let db_path = temp.path().join("orbit.db");
        let backend_a = SqliteJobRunStore::new(Store::open(&db_path).expect("store a"), "ws_a");
        let backend_b = SqliteJobRunStore::new(Store::open(&db_path).expect("store b"), "ws_a");
        let scheduled_at = Utc::now();
        let run = backend_a
            .insert_job_run("job-a", 1, scheduled_at, None, None)
            .expect("insert");
        let run_id = run.run_id.clone();
        let barrier = Arc::new(Barrier::new(2));

        let run_id_a = run_id.clone();
        let barrier_a = Arc::clone(&barrier);
        let writer_a = thread::spawn(move || {
            backend_a.update_run(&run_id_a, |run| {
                run.resolved_crew = Some("crew-a".to_string());
                barrier_a.wait();
                thread::sleep(Duration::from_millis(100));
                Ok(())
            })
        });

        barrier.wait();
        let run_id_b = run_id.clone();
        let writer_b = thread::spawn(move || {
            backend_b.update_run(&run_id_b, |run| {
                run.knowledge_metrics = Some(KnowledgeRunMetrics {
                    raw_read_token_baseline: 100,
                    knowledge_pack_tokens: Some(50),
                    compression_ratio: Some(2.0),
                    actual_fs_read_tokens_during_run: 25,
                    double_read_rate: Some(0.0),
                    knowledge_pack_used: true,
                    knowledge_pack_unresolved_count: 0,
                    total_llm_input_tokens: 75,
                });
                Ok(())
            })
        });

        assert!(writer_a.join().expect("writer a").expect("update a"));
        assert!(writer_b.join().expect("writer b").expect("update b"));

        let loaded = SqliteJobRunStore::new(Store::open(&db_path).expect("store c"), "ws_a")
            .get_job_run(&run_id)
            .expect("read")
            .expect("run");
        assert_eq!(loaded.resolved_crew.as_deref(), Some("crew-a"));
        assert!(loaded.knowledge_metrics.is_some());
    }
}
