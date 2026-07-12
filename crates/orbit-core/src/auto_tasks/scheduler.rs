//! The generic auto-task scheduler [ORB-10149]: one pass that fires every due,
//! enabled definition and mints a task from its template.
//!
//! This is the single scheduler surface — periodic work is data, not code. It
//! runs as the deterministic `run_auto_task_scheduler` action, wrapped in the
//! `auto_task_scheduler_pipeline` job, fired by the seeded `auto_task_scheduler`
//! routine so its fires show up on the dashboard routines surface.
//!
//! Invariants that must hold (acceptance criteria):
//! - **catch-up collapses** — a downtime gap produces one task, not one per
//!   missed slot (see [`super::schedule`]).
//! - **dedupe** — `skip_if_open` never fires while a prior instance created by
//!   the same definition is still open, so a stalled backlog never accumulates
//!   identical tasks.
//! - **provenance** — every minted task carries the `auto-task:<name>` tag,
//!   which is also how `skip_if_open` finds prior instances.

use chrono::{DateTime, Utc};
use orbit_common::types::{
    AutoTaskDefinition, DedupePolicy, OrbitError, Task, TaskStatus, auto_task_tag,
};
use serde_json::{Value, json};

use crate::OrbitRuntime;
use crate::command::task::TaskAddParams;

use super::loader::{AutoTaskLoadError, collect_auto_tasks};
use super::schedule::{AutoTaskDueDecision, decide_due};
use super::state::{AutoTaskCursor, cursor_state_path, load_cursor_state, upsert_cursor};

/// Per-definition outcome of one scheduler pass.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AutoTaskFireReport {
    /// Definition name.
    pub name: String,
    /// One of: `fired`, `would_fire`, `baselined`, `would_baseline`, `skipped`.
    pub action: &'static str,
    /// Why, for `skipped` rows.
    pub reason: Option<String>,
    /// Scheduled slot consumed (RFC 3339, UTC), when a fire was involved.
    pub slot: Option<String>,
    /// Task minted by a fire.
    pub task_id: Option<String>,
}

/// Result of one scheduler pass.
#[derive(Debug, Default)]
pub struct AutoTaskSchedulerOutcome {
    /// Per-definition outcomes.
    pub reports: Vec<AutoTaskFireReport>,
    /// Fail-closed load failures (those definitions were absent this pass).
    pub errors: Vec<AutoTaskLoadError>,
}

/// Options for one scheduler pass.
#[derive(Debug, Clone, Copy, Default)]
pub struct SchedulerOptions {
    /// Report what would fire without recording or creating anything.
    pub dry_run: bool,
}

/// Run one scheduler pass over the definitions in `runtime`'s workspace at an
/// explicit `now` (the test seam). Loads definitions from
/// `<orbit_dir>/auto_tasks/`, cursors from `<orbit_dir>/state/auto-tasks.json`.
pub fn run_auto_task_scheduler_at(
    runtime: &OrbitRuntime,
    now: DateTime<Utc>,
    options: SchedulerOptions,
) -> Result<AutoTaskSchedulerOutcome, OrbitError> {
    let orbit_dir = runtime.paths().orbit_dir.clone();
    let state_path = cursor_state_path(&runtime.paths().state_dir);

    let collection = collect_auto_tasks(&orbit_dir);
    let cursors = load_cursor_state(&state_path);

    let mut reports = Vec::new();
    for loaded in &collection.definitions {
        let definition = &loaded.definition;
        let cursor = cursors.definitions.get(&definition.name);
        let report = fire_definition(runtime, definition, cursor, &state_path, now, options)
            .unwrap_or_else(|error| AutoTaskFireReport {
                name: definition.name.clone(),
                action: "skipped",
                reason: Some(format!("error: {error}")),
                slot: None,
                task_id: None,
            });
        reports.push(report);
    }

    Ok(AutoTaskSchedulerOutcome {
        reports,
        errors: collection.errors,
    })
}

fn fire_definition(
    runtime: &OrbitRuntime,
    definition: &AutoTaskDefinition,
    cursor: Option<&AutoTaskCursor>,
    state_path: &std::path::Path,
    now: DateTime<Utc>,
    options: SchedulerOptions,
) -> Result<AutoTaskFireReport, OrbitError> {
    if !definition.enabled {
        return Ok(skipped(definition, "disabled"));
    }

    // First observation: record the baseline and fire nothing — a definition
    // never mints tasks for slots that predate its registration on this host.
    let Some(cursor) = cursor else {
        if options.dry_run {
            return Ok(action(definition, "would_baseline"));
        }
        upsert_cursor(
            state_path,
            &definition.name,
            AutoTaskCursor {
                baseline_at: now.to_rfc3339(),
                last_slot: None,
                last_fired_at: None,
                last_task_id: None,
            },
        )?;
        return Ok(action(definition, "baselined"));
    };

    let baseline = parse_rfc3339(&cursor.baseline_at)?;
    let last_slot = cursor.last_slot.as_deref().map(parse_rfc3339).transpose()?;

    match decide_due(&definition.schedule, baseline, last_slot, now)? {
        AutoTaskDueDecision::NotDue => Ok(skipped(definition, "not_due")),
        AutoTaskDueDecision::Fire { slot } => {
            // Dedupe: never fire while a prior instance is still open, so a
            // stalled backlog cannot accumulate identical tasks. The cursor is
            // deliberately left unadvanced so the pending occurrence fires
            // (once, collapsed) as soon as the queue drains.
            if definition.dedupe == DedupePolicy::SkipIfOpen
                && has_open_instance(runtime, definition)?
            {
                return Ok(AutoTaskFireReport {
                    slot: Some(slot),
                    ..skipped(definition, "dedupe_open")
                });
            }

            if options.dry_run {
                return Ok(AutoTaskFireReport {
                    slot: Some(slot),
                    ..action(definition, "would_fire")
                });
            }

            let task = mint_task(runtime, definition)?;
            upsert_cursor(
                state_path,
                &definition.name,
                AutoTaskCursor {
                    baseline_at: cursor.baseline_at.clone(),
                    last_slot: Some(slot.clone()),
                    last_fired_at: Some(now.to_rfc3339()),
                    last_task_id: Some(task.id.clone()),
                },
            )?;
            Ok(AutoTaskFireReport {
                name: definition.name.clone(),
                action: "fired",
                reason: None,
                slot: Some(slot),
                task_id: Some(task.id),
            })
        }
    }
}

/// True when a task previously minted by this definition (identified by the
/// `auto-task:<name>` provenance tag) is still in a non-terminal status.
fn has_open_instance(
    runtime: &OrbitRuntime,
    definition: &AutoTaskDefinition,
) -> Result<bool, OrbitError> {
    let tag = auto_task_tag(&definition.name);
    let tasks = runtime.list_tasks_by_tags(std::slice::from_ref(&tag))?;
    Ok(tasks.iter().any(|task| is_open_status(task.status)))
}

/// Statuses that count as "a prior instance is still open" for dedupe. Done,
/// archived, and rejected are closed; everything else is in flight.
fn is_open_status(status: TaskStatus) -> bool {
    !matches!(
        status,
        TaskStatus::Done | TaskStatus::Archived | TaskStatus::Rejected
    )
}

fn mint_task(runtime: &OrbitRuntime, definition: &AutoTaskDefinition) -> Result<Task, OrbitError> {
    let template = &definition.template;
    let mut tags = template.tags.clone();
    tags.push(auto_task_tag(&definition.name));

    runtime.add_task(TaskAddParams {
        title: template.title.clone(),
        description: template.description.clone(),
        acceptance_criteria: template.acceptance_criteria.clone(),
        tags,
        priority: template.priority,
        task_type: Some(template.task_type),
        status: Some(template.status),
        crew: template.crew.clone(),
        system_created: true,
        ..TaskAddParams::default()
    })
}

/// Run one scheduler pass now and project it to the deterministic-action JSON
/// contract (kept in sync with `run_auto_task_scheduler.yaml` output_schema).
pub fn run_scheduler_action_json(
    runtime: &OrbitRuntime,
    input: &Value,
) -> Result<Value, OrbitError> {
    let dry_run = input
        .get("dry_run")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let outcome = run_auto_task_scheduler_at(runtime, Utc::now(), SchedulerOptions { dry_run })?;

    let created: Vec<&AutoTaskFireReport> = outcome
        .reports
        .iter()
        .filter(|report| report.action == "fired")
        .collect();
    let reports = outcome
        .reports
        .iter()
        .map(|report| {
            json!({
                "name": report.name,
                "action": report.action,
                "reason": report.reason,
                "slot": report.slot,
                "task_id": report.task_id,
            })
        })
        .collect::<Vec<_>>();
    let errors = outcome
        .errors
        .iter()
        .map(|error| {
            json!({
                "path": error.path.as_ref().map(|p| p.display().to_string()),
                "message": error.message,
            })
        })
        .collect::<Vec<_>>();

    Ok(json!({
        "dry_run": dry_run,
        "definitions": outcome.reports.len(),
        "created": created.len(),
        "created_task_ids": created
            .iter()
            .filter_map(|report| report.task_id.clone())
            .collect::<Vec<_>>(),
        "reports": reports,
        "load_errors": errors,
    }))
}

fn skipped(definition: &AutoTaskDefinition, reason: &str) -> AutoTaskFireReport {
    AutoTaskFireReport {
        name: definition.name.clone(),
        action: "skipped",
        reason: Some(reason.to_string()),
        slot: None,
        task_id: None,
    }
}

fn action(definition: &AutoTaskDefinition, action: &'static str) -> AutoTaskFireReport {
    AutoTaskFireReport {
        name: definition.name.clone(),
        action,
        reason: None,
        slot: None,
        task_id: None,
    }
}

fn parse_rfc3339(raw: &str) -> Result<DateTime<Utc>, OrbitError> {
    DateTime::parse_from_rfc3339(raw)
        .map(|value| value.with_timezone(&Utc))
        .map_err(|error| OrbitError::Store(format!("invalid stored timestamp '{raw}': {error}")))
}
