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

pub mod clock;
pub mod due;
pub mod host;
pub mod loader;
pub mod status;
pub mod sweep;

pub use clock::{ClockInstallReport, install_clock};
pub use due::{DueDecision, due_decision, parse_cron};
pub use host::{
    HOST_IDENTITY_SCHEMA_VERSION, HostIdentity, HostIdentityOutcome, HostIdentityState, HostMode,
    NewHostIdentity, ensure_host_identity, inspect_host_identity, load_host_identity, os_hostname,
};
pub use loader::{
    LoadedRoutine, RoutineCollection, RoutineLoadError, RoutineOrigin, collect_routines,
};
pub use status::{
    RoutineStatus, RoutineStatusReport, pause_routine, recent_fires, resume_routine,
    routine_statuses,
};
pub use sweep::{RoutineSweepReport, SweepOptions, SweepOutcome, run_sweep, run_sweep_at};

#[cfg(test)]
mod tests;
