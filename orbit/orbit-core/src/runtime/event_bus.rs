use std::sync::{Arc, Mutex};

use orbit_types::OrbitEvent;

/// In-process, session-scoped event log.
///
/// Appended to during the lifetime of a single [`crate::OrbitRuntime`] instance and discarded
/// when the process exits. It is **not persisted** to any store. Agents and callers reading
/// historical audit data should query the SQLite-backed audit event store via
/// [`crate::OrbitRuntime::list_audit_events`] instead.
#[derive(Clone, Default)]
pub struct EventLog {
    events: Arc<Mutex<Vec<OrbitEvent>>>,
}

impl EventLog {
    pub fn append(&self, event: OrbitEvent) {
        if let Ok(mut events) = self.events.lock() {
            events.push(event);
        }
    }

    pub fn snapshot(&self) -> Vec<OrbitEvent> {
        self.events
            .lock()
            .map(|events| events.clone())
            .unwrap_or_default()
    }
}
