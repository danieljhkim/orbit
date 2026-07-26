//! Auto-tasks [ORB-10149]: dynamically-defined recurring task templates + one
//! generic scheduler.
//!
//! Every periodic need in orbit used to be bespoke code (qa-sweep, ship-sweep,
//! …). Auto-tasks replace that pattern with a primitive: a definition is a
//! git-versioned YAML record ([`loader`]) with a schedule, an `enabled` toggle,
//! a task template, and a dedupe policy. A single generic scheduler
//! ([`scheduler`]) fires the due, enabled definitions and mints tasks from
//! their templates — periodic work becomes data, not code. The scheduler runs
//! as the deterministic `run_auto_task_scheduler` action, wrapped in a job,
//! fired by a seeded routine (so its fires are observable on the dashboard
//! routines surface).
//!
//! - [`loader`] — discover + parse definitions, fail-closed.
//! - [`schedule`] — due-math (cron reuses the routine machinery; interval is
//!   native), with catch-up collapse.
//! - [`state`] — host-local, workspace-scoped last-fired cursors.
//! - [`scheduler`] — the pass itself + the deterministic-action projection.
//! - [`crud`] — the shared add/list/show/update/toggle/mint domain surface.

pub mod crud;
pub mod loader;
pub mod schedule;
pub mod scheduler;
pub mod state;

pub use crud::{AutoTaskAddParams, AutoTaskUpdateParams};
pub use loader::{
    AutoTaskCollection, AutoTaskLoadError, LoadedAutoTask, auto_tasks_dir, collect_auto_tasks,
    definition_path,
};
pub use schedule::{AutoTaskDueDecision, decide_due, validate_schedule};
pub use scheduler::{
    AutoTaskFireReport, AutoTaskSchedulerOutcome, SchedulerOptions, run_auto_task_scheduler_at,
    run_scheduler_action_json,
};
pub use state::{AutoTaskCursor, AutoTaskCursorState, cursor_state_path, load_cursor_state};

#[cfg(test)]
mod tests;
