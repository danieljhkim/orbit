use std::collections::{HashMap, HashSet, VecDeque};
use std::path::Path;
use std::sync::mpsc::{self, Receiver};
use std::time::Instant;

use notify::{Event, RecommendedWatcher, RecursiveMode, Watcher};
use orbit_types::{OrbitError, Watch};

use crate::OrbitRuntime;

use super::debounce::{DEFAULT_WATCH_DEBOUNCE_MS, DebounceQueueOne};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WatchEvent {
    pub path: String,
    pub timestamp_ms: u64,
}

impl WatchEvent {
    pub fn new(path: String, timestamp_ms: u64) -> Self {
        Self { path, timestamp_ms }
    }
}

pub trait WatchEventSource {
    fn next_event(&mut self) -> Result<Option<WatchEvent>, OrbitError>;
}

pub struct VecWatchEventSource {
    events: VecDeque<WatchEvent>,
}

impl VecWatchEventSource {
    pub fn new(events: Vec<WatchEvent>) -> Self {
        Self {
            events: events.into(),
        }
    }
}

impl WatchEventSource for VecWatchEventSource {
    fn next_event(&mut self) -> Result<Option<WatchEvent>, OrbitError> {
        Ok(self.events.pop_front())
    }
}

pub struct NotifyEventSource {
    _watcher: RecommendedWatcher,
    rx: Receiver<notify::Result<Event>>,
    started_at: Instant,
}

impl NotifyEventSource {
    pub fn new(paths: impl IntoIterator<Item = String>) -> Result<Self, OrbitError> {
        let (tx, rx) = mpsc::channel();

        let mut watcher = notify::recommended_watcher(move |res| {
            let _ = tx.send(res);
        })
        .map_err(|e| OrbitError::Execution(format!("failed to initialize watcher: {e}")))?;

        for path in paths {
            let p = Path::new(&path);
            let mode = if p.is_dir() {
                RecursiveMode::Recursive
            } else {
                RecursiveMode::NonRecursive
            };
            watcher.watch(p, mode).map_err(|e| {
                OrbitError::Execution(format!("failed to watch path `{}`: {e}", p.display()))
            })?;
        }

        Ok(Self {
            _watcher: watcher,
            rx,
            started_at: Instant::now(),
        })
    }
}

impl WatchEventSource for NotifyEventSource {
    fn next_event(&mut self) -> Result<Option<WatchEvent>, OrbitError> {
        let recv = self
            .rx
            .recv()
            .map_err(|e| OrbitError::Execution(format!("watch receive error: {e}")))?;
        let event = recv.map_err(|e| OrbitError::Execution(format!("watch event error: {e}")))?;

        let Some(path) = event.paths.first() else {
            return Ok(None);
        };

        Ok(Some(WatchEvent {
            path: path.to_string_lossy().to_string(),
            timestamp_ms: self.started_at.elapsed().as_millis() as u64,
        }))
    }
}

impl OrbitRuntime {
    pub fn run_watch_forever(&self) -> Result<(), OrbitError> {
        let watches = self.context.store.list_watches()?;
        if watches.is_empty() {
            return Err(OrbitError::Execution(
                "no watches configured; add watches before running watch mode".to_string(),
            ));
        }

        let mut unique_paths = HashSet::new();
        for watch in &watches {
            unique_paths.insert(watch.path.clone());
        }

        let mut source = NotifyEventSource::new(unique_paths)?;
        let _ = self.run_watch_with_source_for_watches(&watches, &mut source, None)?;
        Ok(())
    }

    pub fn run_watch_with_source<S: WatchEventSource>(
        &self,
        source: &mut S,
        max_events: Option<usize>,
    ) -> Result<usize, OrbitError> {
        let watches = self.context.store.list_watches()?;
        self.run_watch_with_source_for_watches(&watches, source, max_events)
    }

    fn run_watch_with_source_for_watches<S: WatchEventSource>(
        &self,
        watches: &[Watch],
        source: &mut S,
        max_events: Option<usize>,
    ) -> Result<usize, OrbitError> {
        let mut seen_events = 0usize;
        let mut executions = 0usize;
        let mut debouncers: HashMap<String, DebounceQueueOne> = HashMap::new();

        loop {
            if let Some(limit) = max_events
                && seen_events >= limit
            {
                break;
            }

            let Some(event) = source.next_event()? else {
                break;
            };
            seen_events += 1;

            for watch in watches {
                if !watch_matches_path(watch, &event.path) {
                    continue;
                }

                let debouncer = debouncers
                    .entry(watch.id.clone())
                    .or_insert_with(|| DebounceQueueOne::new(DEFAULT_WATCH_DEBOUNCE_MS));

                if let Some(path) = debouncer.on_event(event.path.clone(), event.timestamp_ms)
                    && self.execute_watch_action(watch, &path)?
                {
                    executions += 1;
                }

                if let Some(path) = debouncer.on_tick(event.timestamp_ms)
                    && self.execute_watch_action(watch, &path)?
                {
                    executions += 1;
                }
            }
        }

        Ok(executions)
    }
}

fn watch_matches_path(watch: &Watch, event_path: &str) -> bool {
    let watch_path = Path::new(&watch.path);
    let event_path = Path::new(event_path);

    event_path == watch_path || event_path.starts_with(watch_path)
}
