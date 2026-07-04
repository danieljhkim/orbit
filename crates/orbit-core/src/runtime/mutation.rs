use orbit_common::types::OrbitEvent;

use crate::{OrbitError, OrbitRuntime};

impl OrbitRuntime {
    /// `pub` for the direct v2 activity runner in `orbit-cmd` [ORB-10016],
    /// which records the standard activity-run lifecycle events.
    pub fn record_event(&self, event: OrbitEvent) -> Result<(), OrbitError> {
        self.event_log.append(event);
        Ok(())
    }

    pub fn with_mutation<F, T>(&self, f: F) -> Result<T, OrbitError>
    where
        F: FnOnce() -> Result<(T, OrbitEvent), OrbitError>,
    {
        let (result, event) = f()?;
        self.event_log.append(event);
        Ok(result)
    }
}
