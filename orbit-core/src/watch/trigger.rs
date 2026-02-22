use orbit_types::{OrbitError, OrbitEvent, Watch};

use crate::OrbitRuntime;

impl OrbitRuntime {
    pub(crate) fn execute_watch_action(
        &self,
        watch: &Watch,
        path: &str,
    ) -> Result<bool, OrbitError> {
        let lock_name = format!("watch/{}", watch.id);
        if !self.context.store.try_lock(&lock_name)? {
            return Ok(false);
        }

        let result = (|| {
            self.with_mutation(|_| {
                Ok((
                    (),
                    OrbitEvent::WatchTriggered {
                        path: path.to_string(),
                    },
                ))
            })?;

            let execution = self.execute_shell_command("watch", &watch.command);
            Ok(matches!(execution, Ok(result) if result.success))
        })();

        let _ = self.context.store.unlock(&lock_name);
        result
    }

    pub fn trigger_watch_path(&self, path: &str) -> Result<(), OrbitError> {
        self.with_mutation(|_| {
            Ok((
                (),
                OrbitEvent::WatchTriggered {
                    path: path.to_string(),
                },
            ))
        })?;
        Ok(())
    }
}
