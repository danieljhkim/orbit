use crate::{OrbitError, OrbitRuntime};

impl OrbitRuntime {
    pub fn execute_watch_run_command(&self) -> Result<(), OrbitError> {
        self.run_watch_forever()
    }
}
