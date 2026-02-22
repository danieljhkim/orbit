use chrono::Utc;
use orbit_store::ClaimedJobRun;
use orbit_types::{
    AuthorType, EntityType, EntryType, Job, JobScheduleState, JobSession, JobSessionStatus,
    JobTrigger, OrbitError, OrbitEvent, Role,
};

use crate::OrbitRuntime;
use crate::agent::context::{compose_agent_context, parse_planned_tool_calls};
use crate::command::entry::EntryAddParams;

const SYSTEM_ENTRY_AUTHOR_ID: &str = "runtime";

#[derive(Debug, Clone)]
pub struct JobAddParams {
    pub name: String,
    pub task_id: String,
    pub schedule_spec: String,
    pub timezone: Option<String>,
}

#[derive(Debug, Clone)]
pub struct JobRunResult {
    pub job_id: String,
    pub session_id: String,
    pub status: JobSessionStatus,
}

#[derive(Debug, Clone)]
pub struct JobExecutionOutcome {
    pub status: JobSessionStatus,
    pub exit_code: Option<i32>,
    pub error: Option<String>,
}

impl OrbitRuntime {
    pub fn add_job(&self, params: JobAddParams) -> Result<Job, OrbitError> {
        if params.name.trim().is_empty() {
            return Err(OrbitError::JobValidation(
                "job name must not be empty".to_string(),
            ));
        }
        if params.schedule_spec.trim().is_empty() {
            return Err(OrbitError::JobValidation(
                "schedule must not be empty".to_string(),
            ));
        }

        let task = self.get_task(&params.task_id)?;
        let timezone = resolve_timezone(params.timezone);
        let next_run_at = crate::job::state_machine::compute_next_run_at(
            &params.schedule_spec,
            &timezone,
            Utc::now(),
        )?;

        self.with_mutation(|tx| {
            let job = tx.insert_job(
                &params.name,
                &task.id,
                &params.schedule_spec,
                &timezone,
                Some(next_run_at),
            )?;
            Ok((
                job.clone(),
                OrbitEvent::JobAdded {
                    job_id: job.job_id.clone(),
                },
            ))
        })
    }

    pub fn list_jobs(&self, include_deleted: bool) -> Result<Vec<Job>, OrbitError> {
        self.context.store.list_jobs(include_deleted)
    }

    pub fn show_job(&self, job_id: &str) -> Result<Job, OrbitError> {
        self.context
            .store
            .get_job(job_id)?
            .ok_or_else(|| OrbitError::JobNotFound(job_id.to_string()))
    }

    pub fn pause_job(&self, job_id: &str) -> Result<(), OrbitError> {
        let job = self.show_job(job_id)?;
        if job.state == JobScheduleState::Deleted {
            return Err(OrbitError::JobValidation(
                "cannot pause deleted job".to_string(),
            ));
        }
        self.with_mutation(|tx| {
            let changed = tx.set_job_state(job_id, JobScheduleState::Paused)?;
            if !changed {
                return Err(OrbitError::JobNotFound(job_id.to_string()));
            }
            Ok((
                (),
                OrbitEvent::JobPaused {
                    job_id: job_id.to_string(),
                },
            ))
        })
    }

    pub fn resume_job(&self, job_id: &str) -> Result<(), OrbitError> {
        let job = self.show_job(job_id)?;
        if job.state == JobScheduleState::Deleted {
            return Err(OrbitError::JobValidation(
                "cannot resume deleted job".to_string(),
            ));
        }

        let next_run_at = crate::job::state_machine::compute_next_run_at(
            &job.schedule_spec,
            &job.timezone,
            Utc::now(),
        )?;

        self.with_mutation(|tx| {
            let changed = tx.set_job_state(job_id, JobScheduleState::Active)?;
            if !changed {
                return Err(OrbitError::JobNotFound(job_id.to_string()));
            }
            let _ = tx.update_job_next_run(job_id, Some(next_run_at), None)?;
            Ok((
                (),
                OrbitEvent::JobResumed {
                    job_id: job_id.to_string(),
                },
            ))
        })
    }

    pub fn delete_job(&self, job_id: &str) -> Result<(), OrbitError> {
        self.with_mutation(|tx| {
            let changed = tx.mark_job_deleted(job_id)?;
            if !changed {
                return Err(OrbitError::JobNotFound(job_id.to_string()));
            }
            Ok((
                (),
                OrbitEvent::JobDeleted {
                    job_id: job_id.to_string(),
                },
            ))
        })
    }

    pub fn cancel_job(&self, job_id: &str) -> Result<String, OrbitError> {
        let session_id = self.with_mutation(|tx| {
            let session_id = tx.request_cancel_running_session(job_id)?.ok_or_else(|| {
                OrbitError::JobSessionNotFound(format!("no running session for job {job_id}"))
            })?;
            Ok((
                session_id.clone(),
                OrbitEvent::JobSessionCancelled {
                    job_id: job_id.to_string(),
                    session_id,
                },
            ))
        })?;

        let _ = self.append_job_system_entry(
            job_id,
            format!("job cancellation requested: session={session_id}"),
        );

        Ok(session_id)
    }

    pub fn job_history(&self, job_id: &str) -> Result<Vec<JobSession>, OrbitError> {
        let _ = self.show_job(job_id)?;
        self.context.store.list_job_sessions(job_id)
    }

    pub fn run_job_now(&self, job_id: &str) -> Result<JobRunResult, OrbitError> {
        let job = self.show_job(job_id)?;
        if job.state == JobScheduleState::Deleted {
            return Err(OrbitError::JobValidation(
                "cannot run deleted job".to_string(),
            ));
        }

        let now = Utc::now();
        let session = self.with_mutation(|tx| {
            let session = tx.insert_job_session(
                &job.job_id,
                &job.task_id,
                JobTrigger::Manual,
                Role::Admin,
                now,
                None,
                None,
            )?;
            Ok((
                session.clone(),
                OrbitEvent::JobSessionStarted {
                    job_id: job.job_id.clone(),
                    session_id: session.session_id.clone(),
                    trigger: session.trigger.to_string(),
                },
            ))
        })?;

        let outcome = self.execute_job_session(&job, &session)?;
        let next_run_at =
            crate::job::state_machine::compute_next_run_at(&job.schedule_spec, &job.timezone, now)
                .ok();

        self.with_mutation(|tx| {
            let changed = tx.finish_job_session(
                &session.session_id,
                outcome.status,
                outcome.exit_code,
                outcome.error.as_deref(),
            )?;
            if !changed {
                return Err(OrbitError::JobSessionNotFound(session.session_id.clone()));
            }
            let _ = tx.update_job_next_run(&job.job_id, next_run_at, outcome.error.as_deref())?;
            Ok((
                (),
                OrbitEvent::JobSessionCompleted {
                    job_id: job.job_id.clone(),
                    session_id: session.session_id.clone(),
                    status: outcome.status.to_string(),
                },
            ))
        })?;

        let _ = self.append_job_system_entry(
            &job.job_id,
            format!(
                "job session completed: {} status={}",
                session.session_id, outcome.status
            ),
        );

        Ok(JobRunResult {
            job_id: job.job_id,
            session_id: session.session_id,
            status: outcome.status,
        })
    }

    pub(crate) fn execute_claimed_job(&self, run: &ClaimedJobRun) -> Result<(), OrbitError> {
        let now = Utc::now();
        let outcome = self.execute_job_session(&run.job, &run.session)?;
        let next_run_at = crate::job::state_machine::compute_next_run_at(
            &run.job.schedule_spec,
            &run.job.timezone,
            now,
        )
        .ok();

        self.with_mutation(|tx| {
            let _ = tx.finish_job_session(
                &run.session.session_id,
                outcome.status,
                outcome.exit_code,
                outcome.error.as_deref(),
            )?;
            let _ =
                tx.update_job_next_run(&run.job.job_id, next_run_at, outcome.error.as_deref())?;
            Ok((
                (),
                OrbitEvent::JobSessionCompleted {
                    job_id: run.job.job_id.clone(),
                    session_id: run.session.session_id.clone(),
                    status: outcome.status.to_string(),
                },
            ))
        })?;

        let _ = self.append_job_system_entry(
            &run.job.job_id,
            format!(
                "job session completed: {} status={}",
                run.session.session_id, outcome.status
            ),
        );
        Ok(())
    }

    fn execute_job_session(
        &self,
        job: &Job,
        session: &JobSession,
    ) -> Result<JobExecutionOutcome, OrbitError> {
        let task = self.get_task(&job.task_id)?;
        let skills = self.list_task_skills(&task.id)?;
        let composed = compose_agent_context(self, &task, &skills, Role::Admin)?;
        let planned_calls = parse_planned_tool_calls(&task.instructions)?;

        for call in planned_calls {
            if self
                .context
                .store
                .is_job_session_cancel_requested(&session.session_id)?
            {
                return Ok(JobExecutionOutcome {
                    status: JobSessionStatus::Cancelled,
                    exit_code: Some(130),
                    error: Some("cancel requested".to_string()),
                });
            }

            if !composed.effective_allowed_tools.contains(&call.name) {
                let _ = self.with_mutation(|_| {
                    Ok((
                        (),
                        OrbitEvent::PolicyDenied {
                            tool: call.name.clone(),
                        },
                    ))
                });
                return Ok(JobExecutionOutcome {
                    status: JobSessionStatus::Failed,
                    exit_code: Some(1),
                    error: Some(format!(
                        "tool '{}' not permitted by effective allowlist",
                        call.name
                    )),
                });
            }

            if let Err(err) = self.run_tool_with_role(&call.name, call.input, composed.role) {
                return Ok(JobExecutionOutcome {
                    status: JobSessionStatus::Failed,
                    exit_code: Some(1),
                    error: Some(err.to_string()),
                });
            }
        }

        Ok(JobExecutionOutcome {
            status: JobSessionStatus::Succeeded,
            exit_code: Some(0),
            error: None,
        })
    }

    pub(crate) fn append_job_system_entry(
        &self,
        job_id: &str,
        body: String,
    ) -> Result<(), OrbitError> {
        let _entry = self.add_entry(EntryAddParams {
            entity_type: EntityType::Job,
            entity_id: job_id.to_string(),
            session_id: None,
            entry_type: EntryType::System,
            author_type: AuthorType::System,
            author_id: SYSTEM_ENTRY_AUTHOR_ID.to_string(),
            author_model: None,
            body,
        })?;
        Ok(())
    }
}

fn resolve_timezone(timezone: Option<String>) -> String {
    match timezone {
        Some(tz) if !tz.trim().is_empty() => tz,
        _ => std::env::var("ORBIT_TIMEZONE")
            .ok()
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| "UTC".to_string()),
    }
}
