use chrono::{DateTime, Duration, Utc};

use orbit_types::{OrbitError, OrbitEvent};

use crate::OrbitRuntime;

impl OrbitRuntime {
    pub fn run_due_jobs(&self, now: DateTime<Utc>) -> Result<usize, OrbitError> {
        let lock_name = orbit_store::Store::global_job_lock_name();
        if !self.context.store.try_lock(lock_name)? {
            return Ok(0);
        }

        let result = (|| {
            let claimed_jobs = self
                .context
                .store
                .with_transaction(|tx| tx.claim_due_jobs(now))?;

            for job in &claimed_jobs {
                self.event_bus
                    .publish(OrbitEvent::JobStarted { id: job.id.clone() });
            }

            let mut ran = 0usize;
            for job in claimed_jobs {
                let success = match self.execute_shell_command("job", &job.command) {
                    Ok(result) => result.success,
                    Err(_) => false,
                };

                let next_run_at = now + Duration::minutes(1);
                let completed = self.with_mutation(|tx| {
                    let _final_status = crate::job::state_machine::next_after_run(success);
                    let completed = tx.complete_job(&job.id, next_run_at, success)?;
                    Ok((
                        completed,
                        OrbitEvent::JobCompleted {
                            id: job.id.clone(),
                            success,
                        },
                    ))
                })?;

                if completed {
                    ran += 1;
                }
            }
            Ok(ran)
        })();

        let _ = self.context.store.unlock(lock_name);
        result
    }
}
