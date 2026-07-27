//! Routines [ORB-10021]: Orbit as the constellation's single scheduler.
//!
//! A routine is a durable, git-versioned YAML definition of recurring work —
//! a cron trigger, a catalog target, host pinning, and a retry/overlap
//! policy — living under `.orbit/routines/` in a routine-source workspace
//! (`[routines] role = "source"`). The stateless [`run_sweep`] pass, invoked
//! every minute by the OS clock (see [`clock`]), fires whatever is due on
//! the current host through the existing v2 run machinery. Definitions are
//! shared across hosts via git; all scheduler state is host-local and never
//! synced (ADR-0204..ADR-0208; design in `docs/design/routines/`).

use std::path::Path;

use orbit_common::types::OrbitError;
use orbit_store::Store;

pub mod clock;
pub mod due;
pub mod loader;
pub mod status;
pub mod sweep;
pub mod validation;

pub use clock::{ClockInstallReport, install_clock};
pub use due::{DueDecision, due_decision, parse_cron};
pub use loader::{
    DiscoveredWorkspaces, LoadedRoutine, RoutineCollection, RoutineLoadError, RoutineOrigin,
    RoutineWorkspaceProvider, collect_routines,
};
pub use status::{
    RoutineStatus, RoutineStatusReport, pause_routine, recent_fires, resume_routine,
    routine_statuses_with_providers,
};
pub use sweep::{
    RoutineSweepReport, SweepOptions, SweepOutcome, run_sweep_at_with_providers,
    run_sweep_with_providers,
};
pub use validation::{
    DEFAULT_QUIET_HOST_AFTER_SECONDS, DEFAULT_REGISTRY_CACHE_MAX_AGE_SECONDS,
    RoutineDiagnosticSeverity, RoutineHostIdentity, RoutineHostIdentityView, RoutinePinValidation,
    RoutinePlacementProjection, RoutinePlacementProvider, RoutineRegistryCacheView,
    RoutineRegistryStatus, RoutineRegistryView, RoutineValidationDiagnostic, validate_routine_pins,
};

/// Open the one config-resolved machine-local scheduler/registry store.
///
/// Status, mutation, and sweep paths must share this resolver with the host
/// registry. Opening `<global_root>/orbit.db` independently can make routine
/// validation observe a different registry authority from the writer.
fn open_routine_store(global_root: &Path) -> Result<Store, OrbitError> {
    let database = crate::config::resolved_audit_db_path(global_root, global_root)?;
    Store::open(&database)
}

#[cfg(test)]
mod tests;
