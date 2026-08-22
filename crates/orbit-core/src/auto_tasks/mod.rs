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

use std::borrow::Cow;
use std::path::Path;

use orbit_common::OrbitError;
use orbit_common::protocol::yaml::parse_auto_task_yaml;

use crate::application::{
    ManagedAssetLayout, ManagedAssetReconciliation, reconcile_managed_assets,
};

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

/// Default definitions embedded in the Orbit binary and materialized into a
/// workspace on initialization. Defaults are deliberately inert: users must
/// explicitly mint one or enable it through the existing auto-task surface.
pub(crate) const DEFAULT_AUTO_TASK_FILES: &[(&str, &str)] = &[
    (
        "friction-curation",
        include_str!("../../assets/auto_tasks/friction-curation.yaml"),
    ),
    (
        "qa-sweep",
        include_str!("../../assets/auto_tasks/qa-sweep.yaml"),
    ),
    (
        "security-review",
        include_str!("../../assets/auto_tasks/security-review.yaml"),
    ),
];

/// Seed missing default auto-task definitions without changing an existing
/// workspace-authored definition. Each asset is parsed before it is written so
/// a release cannot install an unloadable default.
///
/// Seeding is manifest-aware: the digest Orbit wrote for each default is
/// recorded so a definition dropped from a later release can be retired by
/// content provenance instead of lingering in every existing workspace.
// ADR-0366: auto-tasks joined the ADR-0346 managed-asset mechanism.
pub(crate) fn seed_default_auto_tasks(
    orbit_dir: &Path,
) -> Result<ManagedAssetReconciliation, OrbitError> {
    let auto_tasks_dir = auto_tasks_dir(orbit_dir);
    reconcile_managed_assets(
        &auto_tasks_dir,
        "auto_task",
        ManagedAssetLayout::YamlStem,
        DEFAULT_AUTO_TASK_FILES,
        false,
        |name, content| {
            let definition = parse_auto_task_yaml(content).map_err(|error| {
                OrbitError::InvalidInput(format!(
                    "default auto-task `{name}` failed validation: {error}"
                ))
            })?;
            if definition.name != name {
                return Err(OrbitError::InvalidInput(format!(
                    "default auto-task file stem `{name}` does not match definition name `{}`",
                    definition.name
                )));
            }
            if definition.enabled {
                return Err(OrbitError::InvalidInput(format!(
                    "default auto-task `{name}` must ship disabled"
                )));
            }
            Ok(Cow::Borrowed(content))
        },
    )
}

#[cfg(test)]
mod tests;
