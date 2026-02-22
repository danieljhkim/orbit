use chrono::{DateTime, Utc};

use orbit_types::{OrbitError, OrbitEvent};

use crate::OrbitRuntime;

impl OrbitRuntime {
    pub fn run_due_jobs(&self, now: DateTime<Utc>) -> Result<usize, OrbitError> {
        let lock_name = orbit_store::Store::global_job_lock_name();
        if !self.context.store.try_lock(lock_name)? {
            return Ok(0);
        }

        let result = (|| {
            let claim = self
                .context
                .store
                .with_transaction(|tx| tx.claim_due_jobs(now))?;

            for skipped_job_id in &claim.skipped {
                self.with_mutation(|_| {
                    Ok((
                        (),
                        OrbitEvent::JobSkipped {
                            job_id: skipped_job_id.clone(),
                            reason: "running session already exists".to_string(),
                        },
                    ))
                })?;
                let _ = self.append_job_system_entry(
                    skipped_job_id,
                    "scheduler skipped run: running session already exists".to_string(),
                );
            }

            for run in &claim.claimed {
                self.event_bus.publish(OrbitEvent::JobSessionStarted {
                    job_id: run.job.job_id.clone(),
                    session_id: run.session.session_id.clone(),
                    trigger: run.session.trigger.to_string(),
                });
            }

            let mut ran = 0usize;
            for run in claim.claimed {
                self.execute_claimed_job(&run)?;
                ran += 1;
            }
            Ok(ran)
        })();

        let _ = self.context.store.unlock(lock_name);
        result
    }
}
