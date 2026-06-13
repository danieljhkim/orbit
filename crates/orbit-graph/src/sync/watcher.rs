//! Background watcher for long-lived graph handles.

use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use notify::{Config, Event, RecommendedWatcher, RecursiveMode, Watcher};

use crate::{GraphError, SyncMode};

const IDLE_POLL: Duration = Duration::from_millis(50);
const IGNORED_TOP_LEVEL_DIRS: &[&str] = &[
    ".git",
    ".orbit",
    "build",
    "dist",
    "node_modules",
    "target",
    "venv",
    ".venv",
    "__pycache__",
];

pub(crate) struct SyncWatcher {
    stop: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
}

impl SyncWatcher {
    pub(crate) fn start(
        db_path: PathBuf,
        worktree_root: PathBuf,
        debounce: Duration,
    ) -> Result<Self, GraphError> {
        let stop = Arc::new(AtomicBool::new(false));
        let ready = Arc::new((Mutex::new(None), Condvar::new()));
        let thread_stop = Arc::clone(&stop);
        let thread_ready = Arc::clone(&ready);
        let handle = thread::Builder::new()
            .name("orbit-graph-sync-watcher".to_string())
            .spawn(move || {
                watcher_thread(db_path, worktree_root, debounce, thread_stop, thread_ready);
            })
            .map_err(|source| {
                GraphError::invalid_data("spawn graph sync watcher", source.to_string())
            })?;

        let start_result = wait_for_start(ready.as_ref());
        if let Err(reason) = start_result {
            stop.store(true, Ordering::Relaxed);
            let _ = handle.join();
            return Err(GraphError::invalid_data("start graph sync watcher", reason));
        }

        Ok(Self {
            stop,
            handle: Some(handle),
        })
    }
}

impl Drop for SyncWatcher {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(handle) = self.handle.take()
            && handle.join().is_err()
        {
            tracing::warn!("graph sync watcher thread panicked during shutdown");
        }
    }
}

type StartState = (Mutex<Option<Result<(), String>>>, Condvar);

fn wait_for_start(ready: &StartState) -> Result<(), String> {
    let (lock, cvar) = ready;
    let mut state = lock
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    while state.is_none() {
        state = cvar
            .wait(state)
            .unwrap_or_else(std::sync::PoisonError::into_inner);
    }
    state
        .as_ref()
        .cloned()
        .unwrap_or_else(|| Err("watcher did not report startup".to_string()))
}

fn watcher_thread(
    db_path: PathBuf,
    worktree_root: PathBuf,
    debounce: Duration,
    stop: Arc<AtomicBool>,
    ready: Arc<StartState>,
) {
    let (event_tx, event_rx) = mpsc::channel();
    let mut watcher = match RecommendedWatcher::new(
        move |event| {
            let _ = event_tx.send(event);
        },
        Config::default(),
    ) {
        Ok(watcher) => watcher,
        Err(error) => {
            notify_start(ready.as_ref(), Err(error.to_string()));
            return;
        }
    };
    if let Err(error) = watcher.watch(worktree_root.as_path(), RecursiveMode::Recursive) {
        notify_start(ready.as_ref(), Err(error.to_string()));
        return;
    }
    notify_start(ready.as_ref(), Ok(()));
    event_loop(event_rx, stop, db_path, worktree_root, debounce);
}

fn notify_start(ready: &StartState, result: Result<(), String>) {
    let (lock, cvar) = ready;
    let mut state = lock
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    *state = Some(result);
    cvar.notify_all();
}

fn event_loop(
    event_rx: Receiver<notify::Result<Event>>,
    stop: Arc<AtomicBool>,
    db_path: PathBuf,
    worktree_root: PathBuf,
    debounce: Duration,
) {
    let mut pending_sync = false;
    while !stop.load(Ordering::Relaxed) {
        let timeout = if pending_sync { debounce } else { IDLE_POLL };
        match event_rx.recv_timeout(timeout) {
            Ok(Ok(event)) => {
                if event_requires_sync(worktree_root.as_path(), &event) {
                    pending_sync = true;
                }
            }
            Ok(Err(error)) => {
                tracing::warn!(
                    target: "orbit.graph.sync",
                    error = %error,
                    "graph sync watcher reported an error; scheduling full diff"
                );
                pending_sync = true;
            }
            Err(RecvTimeoutError::Timeout) if pending_sync => {
                pending_sync = false;
                run_background_sync(db_path.as_path(), worktree_root.as_path());
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => break,
        }
    }
}

fn run_background_sync(db_path: &Path, worktree_root: &Path) {
    if let Err(error) = super::run(db_path, worktree_root, SyncMode::Auto) {
        tracing::warn!(
            target: "orbit.graph.sync",
            error = %error,
            "background graph sync failed"
        );
    }
}

fn event_requires_sync(worktree_root: &Path, event: &Event) -> bool {
    event.paths.is_empty()
        || event
            .paths
            .iter()
            .any(|path| path_requires_sync(worktree_root, path.as_path()))
}

fn path_requires_sync(worktree_root: &Path, path: &Path) -> bool {
    let relative = path.strip_prefix(worktree_root).unwrap_or(path);
    !matches!(
        relative.components().next(),
        Some(Component::Normal(name)) if IGNORED_TOP_LEVEL_DIRS
            .iter()
            .any(|ignored| name == *ignored)
    )
}
