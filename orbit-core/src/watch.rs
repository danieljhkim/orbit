use orbit_types::{OrbitError, OrbitEvent};

use crate::OrbitRuntime;

#[derive(Debug, Clone)]
pub struct DebounceQueueOne {
    window_ms: u64,
    last_emit_ms: Option<u64>,
    pending: Option<String>,
}

impl DebounceQueueOne {
    pub fn new(window_ms: u64) -> Self {
        Self {
            window_ms,
            last_emit_ms: None,
            pending: None,
        }
    }

    pub fn on_event(&mut self, path: String, now_ms: u64) -> Option<String> {
        match self.last_emit_ms {
            None => {
                self.last_emit_ms = Some(now_ms);
                Some(path)
            }
            Some(last) if now_ms.saturating_sub(last) >= self.window_ms => {
                self.last_emit_ms = Some(now_ms);
                Some(path)
            }
            Some(_) => {
                self.pending = Some(path);
                None
            }
        }
    }

    pub fn on_tick(&mut self, now_ms: u64) -> Option<String> {
        match (self.last_emit_ms, self.pending.take()) {
            (Some(last), Some(path)) if now_ms.saturating_sub(last) >= self.window_ms => {
                self.last_emit_ms = Some(now_ms);
                Some(path)
            }
            (last, pending) => {
                self.last_emit_ms = last;
                self.pending = pending;
                None
            }
        }
    }
}

impl OrbitRuntime {
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
