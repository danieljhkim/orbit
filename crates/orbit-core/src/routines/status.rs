//! Read-only status projection for `orbit routine list` / `show` [ORB-10021]:
//! every routine with all three toggle layers (versioned `enabled`, versioned
//! host pinning, host-local pause), the computed next-due slot, and the last
//! recorded fire — so "why didn't this fire?" is one command.

use std::path::Path;

use chrono::{DateTime, Duration, Local, Utc};
use orbit_common::types::OrbitError;
use orbit_store::RoutineFireRecord;

use super::due::{parse_cron, truncate_to_minute};
use super::loader::{LoadedRoutine, RoutineLoadError, RoutineWorkspaceProvider, collect_routines};
use super::validation::{
    DEFAULT_QUIET_HOST_AFTER_SECONDS, DEFAULT_REGISTRY_CACHE_MAX_AGE_SECONDS, RoutinePinValidation,
    RoutinePlacementProjection, RoutinePlacementProvider, RoutineRegistryStatus,
    validate_routine_pins,
};

/// Full effective state of one routine on this host.
#[derive(Debug, Clone)]
pub struct RoutineStatus {
    /// The loaded definition plus its provenance.
    pub routine: LoadedRoutine,
    /// Whether this host's `host_id` appears in the routine's `hosts`.
    pub pinned_to_host: bool,
    /// Registry-aware pin eligibility and additive diagnostics.
    pub validation: RoutinePinValidation,
    /// Host-local pause, when one is set (RFC 3339 pause timestamp).
    pub paused_at: Option<String>,
    /// Next scheduled slot (RFC 3339, host-local), when computable.
    pub next_due: Option<String>,
    /// Most recent fire attempt recorded on this host.
    pub last_fire: Option<RoutineFireRecord>,
}

impl RoutineStatus {
    /// Whether the routine would currently fire on this host when due:
    /// enabled, pinned, and not paused.
    pub fn effective(&self) -> bool {
        self.routine.definition.enabled && self.pinned_to_host && self.paused_at.is_none()
    }
}

/// Everything `orbit routine list` renders.
#[derive(Debug)]
pub struct RoutineStatusReport {
    /// This host's identity.
    pub host_id: String,
    /// Stable machine identity used by registry-resolved pins.
    pub machine_id: String,
    /// Registry source/state used by this projection.
    pub registry: RoutineRegistryStatus,
    /// Per-routine status rows, in discovery order.
    pub statuses: Vec<RoutineStatus>,
    /// Fail-closed load failures (these routines are absent).
    pub load_errors: Vec<RoutineLoadError>,
}

/// Collect routine status from caller-supplied placement and workspace
/// providers. Registry/cache ownership remains outside Core.
pub fn routine_statuses_with_providers(
    global_root: &Path,
    placement_provider: &dyn RoutinePlacementProvider,
    workspace_provider: &dyn RoutineWorkspaceProvider,
    now_utc: DateTime<Utc>,
) -> Result<RoutineStatusReport, OrbitError> {
    let store = super::open_routine_store(global_root)?;
    let RoutinePlacementProjection {
        local_host,
        registry: registry_view,
    } = placement_provider.load_routine_placement(
        now_utc,
        Duration::seconds(DEFAULT_REGISTRY_CACHE_MAX_AGE_SECONDS),
    )?;
    let registry = registry_view.status();

    let discovered = workspace_provider.discover_workspaces(global_root)?;
    let mut load_errors = discovered.errors.clone();
    let mut collection = collect_routines(&discovered.entries, &local_host.host_id);
    load_errors.append(&mut collection.errors);

    let pauses = store.routine_pauses()?;
    let now = now_utc.with_timezone(&Local);

    let mut statuses = Vec::with_capacity(collection.routines.len());
    for routine in collection.routines {
        let next_due = parse_cron(&routine.definition.trigger.cron)
            .ok()
            .and_then(|cron| cron.find_next_occurrence(&now, false).ok())
            .and_then(|slot| truncate_to_minute(slot).ok())
            .map(|slot| slot.to_rfc3339());
        let last_fire = store.routine_latest_fire(&routine.definition.name)?;
        let paused_at = pauses
            .get(&routine.definition.name)
            .map(|pause| pause.paused_at.clone());
        let validation = validate_routine_pins(
            &local_host,
            routine.origin,
            &routine.definition.hosts,
            &registry_view,
            now_utc,
            Duration::seconds(DEFAULT_QUIET_HOST_AFTER_SECONDS),
        );
        let pinned_to_host = validation.eligible;
        statuses.push(RoutineStatus {
            routine,
            pinned_to_host,
            validation,
            paused_at,
            next_due,
            last_fire,
        });
    }

    Ok(RoutineStatusReport {
        host_id: local_host.host_id,
        machine_id: local_host.machine_id,
        registry,
        statuses,
        load_errors,
    })
}

/// Pause a routine on this host (host-local, never synced). Returns `false`
/// when it was already paused.
pub fn pause_routine(global_root: &Path, name: &str, actor: &str) -> Result<bool, OrbitError> {
    let store = super::open_routine_store(global_root)?;
    store.routine_pause(name, actor)
}

/// Clear a host-local pause. Returns `false` when it was not paused.
pub fn resume_routine(global_root: &Path, name: &str) -> Result<bool, OrbitError> {
    let store = super::open_routine_store(global_root)?;
    store.routine_resume(name)
}

/// Recent fire attempts for one routine, newest first (for `routine show`).
pub fn recent_fires(
    global_root: &Path,
    name: &str,
    limit: usize,
) -> Result<Vec<RoutineFireRecord>, OrbitError> {
    let store = super::open_routine_store(global_root)?;
    store.routine_recent_fires(name, limit)
}
