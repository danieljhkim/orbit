//! Routine scheduler health for remote observability [ORB-10138].
//!
//! `GET /api/routines` surfaces every routine visible from the global registry
//! with its last recorded fire (timestamp, outcome, duration) so a consumer can
//! tell — without box ssh — whether the scheduled sweeps (`ship-sweep`,
//! `auto-task-scheduler`) actually fired and succeeded. A stopped scheduler is visible as a
//! stale `last_fire` relative to the routine's `cron` cadence.
//!
//! Named "routines", not "sweeps": in orbit's model the sweeps are routines and
//! not every routine is a sweep (e.g. the default triage routine). Consumers
//! filter by `name` for the sweeps they care about.
//!
//! This is a host-level view — routine fires live in the global store, not any
//! one workspace runtime — so it reads `state.global_root()` directly and takes
//! no `?workspace=` selector.

use axum::extract::State;
use axum::response::{IntoResponse, Json, Response};
use chrono::{DateTime, Utc};
use orbit_cmd::remote_routines::routine_statuses;
use orbit_core::routines::{RoutineStatus, RoutineStatusReport};
use orbit_core::{RoutineFireRecord, RoutineFireState};
use serde_json::{Value, json};

use super::map_runtime_error;
use crate::state::DashboardState;

/// `GET /api/routines` — per-routine scheduler health for this host.
pub(super) async fn list_routine_health(State(state): State<DashboardState>) -> Response {
    match routine_statuses(state.global_root()) {
        Ok(report) => Json(report_json(&report, Utc::now())).into_response(),
        Err(e) => map_runtime_error(e),
    }
}

/// Project a status report into the wire JSON. `generated_at` is the server's
/// clock at query time, so a consumer can measure `last_fire` staleness against
/// a trusted reference rather than its own (possibly skewed) clock.
pub(super) fn report_json(report: &RoutineStatusReport, generated_at: DateTime<Utc>) -> Value {
    json!({
        "generated_at": generated_at.to_rfc3339(),
        "host_id": report.host_id,
        "routines": report.statuses.iter().map(status_json).collect::<Vec<_>>(),
        "load_errors": report.load_errors.iter().map(|e| json!({
            "source_workspace": e.source_workspace,
            "path": e.path.as_ref().map(|p| p.display().to_string()),
            "message": e.message,
        })).collect::<Vec<_>>(),
    })
}

fn status_json(status: &RoutineStatus) -> Value {
    let definition = &status.routine.definition;
    json!({
        "name": definition.name,
        "source": status.routine.source_workspace,
        "target": definition.target.as_ref_string(),
        "enabled": definition.enabled,
        "pinned_to_host": status.pinned_to_host,
        "paused_at": status.paused_at,
        "effective": status.effective(),
        "cron": definition.trigger.cron,
        "next_due": status.next_due,
        "last_fire": status.last_fire.as_ref().map(fire_json),
    })
}

/// One fire attempt, enriched with a coarse `ok` outcome and a wall-clock
/// duration (intent → last state change) for terminal fires.
pub(super) fn fire_json(fire: &RoutineFireRecord) -> Value {
    let finished = fire.state.is_terminal();
    json!({
        "slot": fire.slot,
        "attempt": fire.attempt,
        "state": fire.state.as_str(),
        "ok": fire_ok(fire.state),
        "run_id": fire.run_id,
        "detail": fire.detail,
        "started_at": fire.created_at,
        "finished_at": finished.then(|| fire.updated_at.clone()),
        "duration_ms": duration_ms(fire),
    })
}

/// Coarse pass/fail for a fire state: `Some(true)` succeeded, `Some(false)`
/// failed/timed-out/errored, `None` while still in flight (intent/dispatched).
pub(super) fn fire_ok(state: RoutineFireState) -> Option<bool> {
    match state {
        RoutineFireState::Succeeded => Some(true),
        RoutineFireState::Failed | RoutineFireState::TimedOut | RoutineFireState::Error => {
            Some(false)
        }
        RoutineFireState::Intent | RoutineFireState::Dispatched => None,
    }
}

/// Wall-clock duration (`created_at` → `updated_at`) in milliseconds for a
/// terminal fire; `None` while in flight or if either timestamp fails to parse.
pub(super) fn duration_ms(fire: &RoutineFireRecord) -> Option<i64> {
    if !fire.state.is_terminal() {
        return None;
    }
    let start = DateTime::parse_from_rfc3339(&fire.created_at).ok()?;
    let end = DateTime::parse_from_rfc3339(&fire.updated_at).ok()?;
    Some((end - start).num_milliseconds())
}
