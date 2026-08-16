//! The stateless sweep pass [ORB-10021]: what runs when the OS clock invokes
//! `orbit sweep` (ADR-0204). Modeled on `orbit run ship-sweep`: never
//! bootstraps a workspace from the caller's cwd, isolates per-routine
//! failures into report rows, and returns `Err` only for infrastructure
//! failures (registry unreadable, store unopenable) — an unconfigured host
//! is a clean no-op, because launchd/systemd will invoke this forever.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Duration, Local, Utc};
use orbit_common::OrbitError;
use orbit_common::observability::log_rotation::{self, LogRotationConfig};
use orbit_store::{RoutineFireIntentParams, RoutineFireRecord, RoutineFireState, Store};
use orbit_types::workflow::{JobRunState, OverlapPolicy};
use serde_json::json;

use crate::OrbitRuntime;
use crate::command::job::{RunOwnerLiveness, run_owner_liveness};

use super::due::{DueDecision, due_decision, parse_cron};
use super::loader::{
    LoadedRoutine, RoutineCollection, RoutineLoadError, RoutineWorkspaceProvider, collect_routines,
};
#[cfg(test)]
use super::validation::RoutineHostIdentity;
use super::validation::{
    RoutineHostIdentityView, RoutinePinValidation, RoutinePlacementProjection,
    RoutinePlacementProvider, RoutineRegistryStatus, RoutineRegistryView, validate_routine_pins,
};

/// Dispatch seam for the sweep. The production impl
/// ([`RuntimeDispatch`]) wraps one `OrbitRuntime` per source workspace and
/// dispatches through `submit_pipeline_run` / `show_job_run`; tests supply a
/// fake so the sweep's fire / retry / overlap / outcome-sync orchestration is
/// exercised deterministically without spawning pipeline workers.
pub(crate) trait RoutineDispatch {
    /// Submit `job_name` in the source workspace rooted at `source_orbit_dir`
    /// under `actor`, returning the dispatched run id.
    fn submit(
        &self,
        source_orbit_dir: &Path,
        job_name: &str,
        actor: &str,
    ) -> Result<String, OrbitError>;

    /// Current run state for a dispatched fire, when the run is queryable.
    fn run_state(&self, source_orbit_dir: &Path, run_id: &str) -> Option<JobRunState>;

    /// [ORB-10597] Whether the run's recorded owner process is still executing,
    /// asked independently of its persisted state. Consulted only for runs
    /// marked `interrupted`, which carries no teardown and so is not evidence
    /// that the work stopped.
    fn run_owner_liveness(&self, source_orbit_dir: &Path, run_id: &str) -> RunOwnerLiveness;
}

/// Production dispatch over the per-workspace runtimes discovered this pass.
pub(crate) struct RuntimeDispatch<'a> {
    runtimes: BTreeMap<PathBuf, &'a OrbitRuntime>,
}

impl RoutineDispatch for RuntimeDispatch<'_> {
    fn submit(
        &self,
        source_orbit_dir: &Path,
        job_name: &str,
        actor: &str,
    ) -> Result<String, OrbitError> {
        let runtime = self.runtimes.get(source_orbit_dir).ok_or_else(|| {
            OrbitError::WorkspaceError(format!(
                "no runtime for source workspace '{}'",
                source_orbit_dir.display()
            ))
        })?;
        runtime
            .submit_pipeline_run(job_name, json!({}), None, Some(actor))
            .map(|invoke| invoke.run_id)
    }

    fn run_state(&self, source_orbit_dir: &Path, run_id: &str) -> Option<JobRunState> {
        self.runtimes
            .get(source_orbit_dir)
            .and_then(|runtime| runtime.show_job_run(run_id).ok())
            .map(|run| run.state)
    }

    fn run_owner_liveness(&self, source_orbit_dir: &Path, run_id: &str) -> RunOwnerLiveness {
        self.runtimes
            .get(source_orbit_dir)
            .and_then(|runtime| runtime.show_job_run(run_id).ok())
            // An unreadable run is not evidence that its worker stopped.
            .map_or(RunOwnerLiveness::Unknown, |run| run_owner_liveness(&run))
    }
}

/// Options for one sweep pass.
#[derive(Debug, Clone, Copy, Default)]
pub struct SweepOptions {
    /// Report what would fire without recording or dispatching anything.
    pub dry_run: bool,
}

/// Per-routine outcome of one sweep pass.
#[derive(Debug, Clone)]
pub struct RoutineSweepReport {
    /// Routine name.
    pub routine: String,
    /// Source workspace name.
    pub source: String,
    /// Whether the definition is `committed` or `local` origin.
    pub origin: &'static str,
    /// One of: `fired`, `retry_fired`, `would_fire`, `baselined`,
    /// `would_baseline`, `skipped`, `error`.
    pub action: &'static str,
    /// Why, for `skipped`/`error` rows.
    pub reason: Option<String>,
    /// Scheduled slot consumed (RFC 3339, UTC), when a fire was involved.
    pub slot: Option<String>,
    /// Run id returned by dispatch, when one was submitted.
    pub run_id: Option<String>,
    /// Registry-aware eligibility and diagnostics evaluated before mutation.
    pub validation: RoutinePinValidation,
}

/// Result of one sweep pass.
#[derive(Debug, Default)]
pub struct SweepOutcome {
    /// Host identity the pass filtered against.
    pub host_id: String,
    /// Stable machine identity used by registry-resolved pins.
    pub machine_id: String,
    /// Registry source/state used for this pass.
    pub registry: RoutineRegistryStatus,
    /// True when another sweep held the lock and this pass exited early.
    pub lock_busy: bool,
    /// Per-routine outcomes.
    pub reports: Vec<RoutineSweepReport>,
    /// Fail-closed definition/load failures (those routines were absent).
    pub load_errors: Vec<RoutineLoadError>,
}

/// Run one sweep pass against the default global root with caller-supplied
/// placement and workspace providers.
pub fn run_sweep_with_providers(
    options: SweepOptions,
    local_host: super::validation::RoutineHostIdentity,
    placement_provider: &dyn RoutinePlacementProvider,
    workspace_provider: &dyn RoutineWorkspaceProvider,
) -> Result<SweepOutcome, OrbitError> {
    let global_root = crate::runtime::resolve_global_root()?;
    // The OS clock invokes this every minute forever; on macOS launchd
    // redirects stdout/stderr into `logs/sweep.log`. Opportunistically roll +
    // prune it here (rename-based, best-effort) so an always-on host cannot
    // grow it without bound. No-op until the file exceeds the
    // configured per-file budget. `run_sweep_at_with_providers` (the explicit
    // root seam) is left
    // untouched so tests never rotate real logs.
    log_rotation::rotate_and_prune(
        &super::clock::sweep_log_path(&global_root),
        &LogRotationConfig::load_global_best_effort(),
    );
    run_sweep_at_with_providers(
        &global_root,
        options,
        local_host,
        placement_provider,
        workspace_provider,
    )
}

/// Run one sweep pass against an explicit global root using injected remote
/// composition. Provider calls occur only after the sweep lock is held.
pub fn run_sweep_at_with_providers(
    global_root: &Path,
    options: SweepOptions,
    local_host: super::validation::RoutineHostIdentity,
    placement_provider: &dyn RoutinePlacementProvider,
    workspace_provider: &dyn RoutineWorkspaceProvider,
) -> Result<SweepOutcome, OrbitError> {
    // One pass per host at a time: overlapping invocations from a slow prior
    // pass must not double-fire. flock releases on process death, so a
    // crashed sweep never wedges the next one.
    let lock = orbit_store::try_acquire_routine_sweep_lock(&global_root.join("state"))?;
    let Some(_lock) = lock else {
        return Ok(SweepOutcome {
            host_id: local_host.host_id,
            machine_id: local_host.machine_id,
            lock_busy: true,
            ..SweepOutcome::default()
        });
    };

    let store = super::open_routine_store(global_root)?;
    let now_utc = Utc::now();
    let RoutinePlacementProjection {
        local_host,
        registry: registry_view,
    } = placement_provider.load_routine_placement()?;
    let registry = registry_view.status();

    // One runtime per active workspace; discovery and dispatch share them.
    let discovered = workspace_provider.discover_workspaces(global_root)?;
    let mut load_errors: Vec<RoutineLoadError> = discovered.errors.clone();

    let mut collection = collect_routines(&discovered.entries, &local_host.host_id);
    load_errors.append(&mut collection.errors);

    let dispatch = RuntimeDispatch {
        runtimes: discovered
            .entries
            .iter()
            .map(|(_, runtime)| (runtime.shared_root(), runtime))
            .collect(),
    };

    let reports = run_sweep_core_with_registry(
        &store,
        &local_host,
        &registry_view,
        &collection,
        &dispatch,
        options,
        now_utc,
    )?;

    Ok(SweepOutcome {
        host_id: local_host.host_id,
        machine_id: local_host.machine_id,
        registry,
        lock_busy: false,
        reports,
        load_errors,
    })
}

/// The dispatch-agnostic core of one sweep pass: outcome-sync
/// (unless dry-run), then per-routine due evaluation and fire/skip. Split out
/// from [`run_sweep_at_with_providers`] — which owns the lock, store, and workspace discovery
/// — so the orchestration can be driven against a temp store, a hand-built
/// [`RoutineCollection`], a fake [`RoutineDispatch`], and an explicit `now`.
#[cfg(test)]
pub(crate) fn run_sweep_core(
    store: &Store,
    host_id: &str,
    collection: &RoutineCollection,
    dispatch: &dyn RoutineDispatch,
    options: SweepOptions,
    now_utc: DateTime<Utc>,
) -> Result<Vec<RoutineSweepReport>, OrbitError> {
    let identity = RoutineHostIdentity {
        machine_id: format!("hm_standalone_{}", sanitize_test_machine_suffix(host_id)),
        host_id: host_id.to_string(),
    };
    run_sweep_core_with_registry(
        store,
        &identity,
        &RoutineRegistryView {
            owner_host_ids: Default::default(),
        },
        collection,
        dispatch,
        options,
        now_utc,
    )
}

/// Registry-aware core used by production and deterministic R2 fixtures.
pub(crate) fn run_sweep_core_with_registry(
    store: &Store,
    identity: &dyn RoutineHostIdentityView,
    registry_view: &RoutineRegistryView,
    collection: &RoutineCollection,
    dispatch: &dyn RoutineDispatch,
    options: SweepOptions,
    now_utc: DateTime<Utc>,
) -> Result<Vec<RoutineSweepReport>, OrbitError> {
    let validations: BTreeMap<String, RoutinePinValidation> = collection
        .routines
        .iter()
        .map(|routine| {
            (
                routine.definition.name.clone(),
                validate_routine_pins(
                    identity,
                    routine.origin,
                    &routine.definition.hosts,
                    registry_view,
                ),
            )
        })
        .collect();
    let routines_by_name: BTreeMap<String, &LoadedRoutine> = collection
        .routines
        .iter()
        .filter(|routine| {
            validations
                .get(&routine.definition.name)
                .is_some_and(|validation| validation.eligible)
        })
        .map(|routine| (routine.definition.name.clone(), routine))
        .collect();

    if !options.dry_run {
        sync_unresolved_fires(store, &routines_by_name, dispatch, now_utc)?;
    }

    let pauses = store.routine_pauses()?;

    let mut reports = Vec::new();
    for routine in &collection.routines {
        let validation = validations
            .get(&routine.definition.name)
            .cloned()
            .unwrap_or(RoutinePinValidation {
                eligible: false,
                diagnostics: Vec::new(),
            });
        let report = sweep_routine(
            store,
            routine,
            dispatch,
            &validation,
            &pauses,
            options,
            now_utc,
        )
        .unwrap_or_else(|error| RoutineSweepReport {
            routine: routine.definition.name.clone(),
            source: routine.source_workspace.clone(),
            origin: routine.origin.as_str(),
            action: "error",
            reason: Some(error.to_string()),
            slot: None,
            run_id: None,
            validation: validation.clone(),
        });
        reports.push(report);
    }

    Ok(reports)
}

fn sweep_routine(
    store: &Store,
    routine: &LoadedRoutine,
    dispatch: &dyn RoutineDispatch,
    validation: &RoutinePinValidation,
    pauses: &BTreeMap<String, orbit_store::RoutinePauseRecord>,
    options: SweepOptions,
    now_utc: DateTime<Utc>,
) -> Result<RoutineSweepReport, OrbitError> {
    let definition = &routine.definition;
    let name = &definition.name;

    // Toggle resolution order (2_design.md §4): versioned kill-switch →
    // versioned host pinning → host-local pause.
    if !definition.enabled {
        return Ok(skipped(routine, validation, "disabled_in_definition"));
    }
    if !validation.eligible {
        return Ok(skipped(routine, validation, "host_not_pinned"));
    }
    if pauses.contains_key(name) {
        return Ok(skipped(routine, validation, "paused_locally"));
    }

    let cron = parse_cron(&definition.trigger.cron)?;
    let now_local = now_utc.with_timezone(&Local);

    let Some(cursor) = store.routine_cursor(name)? else {
        // First observation on this host: record the baseline and fire
        // nothing — a routine never fires for slots that predate its
        // registration here.
        if options.dry_run {
            return Ok(action(routine, validation, "would_baseline"));
        }
        store.routine_record_baseline(name, &now_utc.to_rfc3339())?;
        return Ok(action(routine, validation, "baselined"));
    };

    let lower_bound_raw = cursor.last_slot.as_deref().unwrap_or(&cursor.baseline_at);
    let lower_bound = parse_rfc3339(lower_bound_raw)?.with_timezone(&Local);

    match due_decision(
        &cron,
        definition.trigger.missed_run,
        &lower_bound,
        &now_local,
    )? {
        DueDecision::Fire { slot, .. } => {
            let slot_utc = slot.with_timezone(&Utc).to_rfc3339();
            fire(
                store,
                routine,
                dispatch,
                validation,
                FireRequest {
                    slot: &slot_utc,
                    attempt: 1,
                    fired_action: "fired",
                    options,
                },
            )
        }
        DueDecision::NotDue => {
            // No new slot: a most-recent fire that failed (run-level) or
            // errored at dispatch may still have retry budget under the same
            // slot.
            if let Some(retry) = retry_candidate(store, routine, now_utc)? {
                return fire(
                    store,
                    routine,
                    dispatch,
                    validation,
                    FireRequest {
                        slot: &retry.slot,
                        attempt: retry.attempt + 1,
                        fired_action: "retry_fired",
                        options,
                    },
                );
            }
            Ok(skipped(routine, validation, "not_due"))
        }
    }
}

/// The most recent fire, when it failed with retry budget left and the fixed
/// backoff has elapsed.
///
/// Retryable means `Failed` — a run-level failure *or* a synchronous dispatch
/// failure: `fire` records a `submit_pipeline_run` that returns
/// `Err` as `Failed` (not `Error`) precisely because nothing dispatched, so it
/// is unambiguously safe to re-dispatch under the same slot. `Error` is
/// reserved for the *ambiguous* case — a crashed sweep's stale intent reclaimed
/// by the outcome sync, where a worker may have partially started — and stays
/// terminal so a make-up fire never races an orphaned run.
fn retry_candidate(
    store: &Store,
    routine: &LoadedRoutine,
    now_utc: DateTime<Utc>,
) -> Result<Option<RoutineFireRecord>, OrbitError> {
    let retries = routine.definition.policy.retries;
    if retries.max == 0 {
        return Ok(None);
    }
    let Some(latest) = store.routine_latest_fire(&routine.definition.name)? else {
        return Ok(None);
    };
    if latest.state != RoutineFireState::Failed {
        return Ok(None);
    }
    // attempt is 1-based: max=2 allows attempts 2 and 3.
    if latest.attempt > retries.max {
        return Ok(None);
    }
    let failed_at = parse_rfc3339(&latest.updated_at)?;
    if now_utc.signed_duration_since(failed_at) < Duration::minutes(retries.backoff_minutes as i64)
    {
        return Ok(None);
    }
    Ok(Some(latest))
}

struct FireRequest<'a> {
    slot: &'a str,
    attempt: u32,
    fired_action: &'static str,
    options: SweepOptions,
}

fn fire(
    store: &Store,
    routine: &LoadedRoutine,
    dispatch: &dyn RoutineDispatch,
    validation: &RoutinePinValidation,
    request: FireRequest<'_>,
) -> Result<RoutineSweepReport, OrbitError> {
    let FireRequest {
        slot,
        attempt,
        fired_action,
        options,
    } = request;
    let definition = &routine.definition;
    let name = &definition.name;

    if definition.policy.overlap == OverlapPolicy::Forbid
        && let Some(latest) = store.routine_latest_fire(name)?
        && !latest.state.is_terminal()
    {
        // Stale in-flight entries past the policy timeout were already
        // reclaimed by the outcome sync at the top of the pass, so anything
        // still non-terminal here is genuinely (believed) in flight.
        return Ok(RoutineSweepReport {
            slot: Some(slot.to_string()),
            ..skipped(routine, validation, "overlap_in_flight")
        });
    }

    if options.dry_run {
        return Ok(RoutineSweepReport {
            slot: Some(slot.to_string()),
            ..action(routine, validation, "would_fire")
        });
    }

    let claimed = store.routine_record_fire_intent(&RoutineFireIntentParams {
        routine_name: name.clone(),
        slot: slot.to_string(),
        attempt,
        source_workspace: routine.source_workspace.clone(),
    })?;
    if !claimed {
        return Ok(RoutineSweepReport {
            slot: Some(slot.to_string()),
            ..skipped(routine, validation, "slot_already_claimed")
        });
    }

    let actor = format!("routine/{name}");
    match dispatch.submit(
        &routine.source_orbit_dir,
        definition.target.job_name(),
        &actor,
    ) {
        Ok(run_id) => {
            store.routine_mark_fire_dispatched(name, slot, attempt, &run_id)?;
            Ok(RoutineSweepReport {
                routine: name.clone(),
                source: routine.source_workspace.clone(),
                origin: routine.origin.as_str(),
                action: fired_action,
                reason: None,
                slot: Some(slot.to_string()),
                run_id: Some(run_id),
                validation: validation.clone(),
            })
        }
        Err(error) => {
            // A synchronous dispatch failure means nothing was dispatched, so
            // record it as `Failed` — retryable under the same slot within
            // `policy.retries` — rather than the terminal `Error`
            // the outcome sync reserves for an ambiguous crash-orphaned intent.
            store.routine_mark_fire_outcome(
                name,
                slot,
                attempt,
                RoutineFireState::Failed,
                Some(&format!("dispatch failed: {error}")),
            )?;
            Ok(RoutineSweepReport {
                routine: name.clone(),
                source: routine.source_workspace.clone(),
                origin: routine.origin.as_str(),
                action: "error",
                reason: Some(format!("dispatch failed: {error}")),
                slot: Some(slot.to_string()),
                run_id: None,
                validation: validation.clone(),
            })
        }
    }
}

/// Bring unresolved fires up to date against actual run state, and reclaim
/// entries older than the routine's policy timeout (the staleness horizon —
/// without it, a sweep that crashed between intent and dispatch would block
/// `overlap: forbid` forever).
fn sync_unresolved_fires(
    store: &Store,
    routines_by_name: &BTreeMap<String, &LoadedRoutine>,
    dispatch: &dyn RoutineDispatch,
    now_utc: DateTime<Utc>,
) -> Result<(), OrbitError> {
    for fire in store.routine_unresolved_fires()? {
        // Eligibility was resolved before this mutation phase. A routine that
        // is no longer assigned to this machine must leave this machine's
        // prior fire history byte/logically untouched.
        let Some(routine) = routines_by_name.get(&fire.routine_name) else {
            continue;
        };
        let timeout_minutes = routine.definition.policy.timeout_minutes;
        let created_at = match parse_rfc3339(&fire.created_at) {
            Ok(value) => value,
            Err(_) => continue,
        };
        let expired =
            now_utc.signed_duration_since(created_at) > Duration::minutes(timeout_minutes as i64);

        match fire.state {
            // A recorded intent whose sweep died before dispatch: reclaim it
            // once past the timeout horizon, otherwise leave it for a later pass.
            RoutineFireState::Intent if expired => {
                store.routine_mark_fire_outcome(
                    &fire.routine_name,
                    &fire.slot,
                    fire.attempt,
                    RoutineFireState::Error,
                    Some("stale fire intent reclaimed (sweep died before dispatch)"),
                )?;
            }
            RoutineFireState::Dispatched => {
                let run_id = fire.run_id.as_deref();
                let run_state =
                    run_id.and_then(|run_id| dispatch.run_state(&routine.source_orbit_dir, run_id));
                // Reclaiming a fire past the policy timeout is what keeps a
                // routine from wedging forever; it is also the only sanctioned
                // way an `overlap:forbid` slot is freed while work may still be
                // in flight.
                let timed_out = || {
                    expired.then_some((
                        RoutineFireState::TimedOut,
                        Some("exceeded policy timeout without a terminal run state"),
                    ))
                };
                let outcome = match run_state {
                    Some(JobRunState::Success) => Some((RoutineFireState::Succeeded, None)),
                    Some(JobRunState::Failed) => Some((RoutineFireState::Failed, None)),
                    Some(JobRunState::Timeout) => Some((RoutineFireState::TimedOut, None)),
                    // Cancellation signals the owner and verifies its
                    // termination, so a cancelled run has genuinely stopped.
                    Some(JobRunState::Cancelled) => {
                        Some((RoutineFireState::Failed, Some("run cancelled")))
                    }
                    // A persisted in-flight state is not enough to hold an
                    // overlap slot after a restart. If the recorded owner is
                    // conclusively gone, no work can still be executing and
                    // the fire can be reconciled immediately. Alive and
                    // unknown owners remain protected until their terminal
                    // state or the normal policy timeout.
                    Some(JobRunState::Running | JobRunState::Retrying) => {
                        let liveness = run_id.map_or(RunOwnerLiveness::Unknown, |run_id| {
                            dispatch.run_owner_liveness(&routine.source_orbit_dir, run_id)
                        });
                        match liveness {
                            RunOwnerLiveness::Stopped => Some((
                                RoutineFireState::Failed,
                                Some("run owner stopped before recording a terminal outcome"),
                            )),
                            RunOwnerLiveness::Alive | RunOwnerLiveness::Unknown => timed_out(),
                        }
                    }
                    // [ORB-10597] Resolving a fire is what releases the
                    // `overlap:forbid` slot, and for `interrupted` alone the
                    // terminal state is not evidence that the work stopped:
                    // marking a run interrupted attaches no teardown, so a run
                    // condemned in error keeps executing. Releasing the slot
                    // then admits a second instance against the same surface
                    // while the first is still working.
                    //
                    // The distinction to draw is terminal-*and-stopped* versus
                    // terminal-*and-still-executing*, which the run's recorded
                    // owner answers. A stopped owner releases exactly as before
                    // — that case is correct and is why this arm cannot simply
                    // be deleted. An owner that is alive, or that this host
                    // cannot conclusively probe, is treated as still in flight:
                    // the slot stays held and the fire is reclaimed only by the
                    // policy timeout, the same bound every genuinely in-flight
                    // run already lives under. That bound is what keeps an
                    // unprobeable owner from wedging the routine forever.
                    Some(JobRunState::Interrupted) => {
                        let liveness = run_id.map_or(RunOwnerLiveness::Unknown, |run_id| {
                            dispatch.run_owner_liveness(&routine.source_orbit_dir, run_id)
                        });
                        match liveness {
                            RunOwnerLiveness::Stopped => {
                                Some((RoutineFireState::Failed, Some("run interrupted")))
                            }
                            RunOwnerLiveness::Alive | RunOwnerLiveness::Unknown => {
                                tracing::warn!(
                                    target: "orbit.core.routines",
                                    routine = %fire.routine_name,
                                    slot = %fire.slot,
                                    run_id = run_id.unwrap_or("-"),
                                    liveness = ?liveness,
                                    "routine run is marked interrupted but its worker has not \
                                     been shown to have stopped; holding the overlap slot",
                                );
                                timed_out()
                            }
                        }
                    }
                    // Still in flight (or unqueryable): reclaim once past the
                    // policy timeout, otherwise leave for a later pass.
                    _ => timed_out(),
                };
                if let Some((state, detail)) = outcome {
                    store.routine_mark_fire_outcome(
                        &fire.routine_name,
                        &fire.slot,
                        fire.attempt,
                        state,
                        detail,
                    )?;
                }
            }
            _ => {}
        }
    }
    Ok(())
}

fn skipped(
    routine: &LoadedRoutine,
    validation: &RoutinePinValidation,
    reason: &str,
) -> RoutineSweepReport {
    RoutineSweepReport {
        routine: routine.definition.name.clone(),
        source: routine.source_workspace.clone(),
        origin: routine.origin.as_str(),
        action: "skipped",
        reason: Some(reason.to_string()),
        slot: None,
        run_id: None,
        validation: validation.clone(),
    }
}

fn action(
    routine: &LoadedRoutine,
    validation: &RoutinePinValidation,
    action: &'static str,
) -> RoutineSweepReport {
    RoutineSweepReport {
        routine: routine.definition.name.clone(),
        source: routine.source_workspace.clone(),
        origin: routine.origin.as_str(),
        action,
        reason: None,
        slot: None,
        run_id: None,
        validation: validation.clone(),
    }
}

#[cfg(test)]
fn sanitize_test_machine_suffix(host_id: &str) -> String {
    let suffix: String = host_id
        .bytes()
        .map(|byte| {
            if byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-') {
                char::from(byte)
            } else {
                '_'
            }
        })
        .collect();
    if suffix.is_empty() {
        "host".to_string()
    } else {
        suffix
    }
}

fn parse_rfc3339(raw: &str) -> Result<DateTime<Utc>, OrbitError> {
    DateTime::parse_from_rfc3339(raw)
        .map(|value| value.with_timezone(&Utc))
        .map_err(|error| OrbitError::Store(format!("invalid stored timestamp '{raw}': {error}")))
}
