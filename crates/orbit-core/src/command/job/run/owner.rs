//! Owner process signalling, identity classification, and liveness probes.
//!
//! All Unix-specific; non-Unix shims return neutral outcomes without signalling.

use orbit_common::types::OrbitError;
use orbit_common::types::{JobRun, JobRunState};
#[cfg(unix)]
use orbit_common::utility::process_identity::{
    ProbeOutcome, STABLE_TOKEN_PREFIX, legacy_lstart_matches, probe_process_start_identity,
};
#[cfg(unix)]
use std::thread;
#[cfg(unix)]
use std::time::{Duration, Instant};

#[cfg(unix)]
pub(super) const RUN_OWNER_TERMINATION_GRACE: Duration = Duration::from_secs(2);
#[cfg(unix)]
pub(super) const RUN_OWNER_TERMINATION_POLL: Duration = Duration::from_millis(50);

/// Attempts to signal (TERM then KILL) the recorded owner process / process group
/// for a running job, returning a short outcome token for telemetry.
#[cfg(unix)]
pub(super) fn signal_run_owner_process(run: &JobRun) -> Result<String, OrbitError> {
    let Some(pid) = run.pid else {
        return Ok("no_pid".to_string());
    };
    if pid == std::process::id() {
        return Ok("self_not_signalled".to_string());
    }
    if !matches!(classify_run_owner(run), OwnerIdentity::Verified) {
        return Ok("owner_identity_mismatch".to_string());
    }

    let pgid = owner_process_group_id(pid);
    if let Some(pgid) = pgid
        && pgid > 1
    {
        if pgid == unsafe { libc::getpgrp() } {
            return Ok("owner_process_group_matches_current_process".to_string());
        }
        match send_signal_to_process_group(pgid, libc::SIGTERM) {
            Ok(()) => {}
            Err(error) if error.raw_os_error() == Some(libc::ESRCH) => {
                return Ok("already_exited".to_string());
            }
            Err(error) => {
                return Err(OrbitError::Execution(format!(
                    "failed to signal job run owner process group {pgid} for pid {pid}: {error}"
                )));
            }
        }

        if wait_for_process_group_exit(pgid, RUN_OWNER_TERMINATION_GRACE) {
            return Ok("terminated_process_group".to_string());
        }

        match send_signal_to_process_group(pgid, libc::SIGKILL) {
            Ok(()) => {}
            Err(error) if error.raw_os_error() == Some(libc::ESRCH) => {
                return Ok("terminated_process_group".to_string());
            }
            Err(error) => {
                return Err(OrbitError::Execution(format!(
                    "failed to kill job run owner process group {pgid} for pid {pid}: {error}"
                )));
            }
        }
        let _ = wait_for_process_group_exit(pgid, RUN_OWNER_TERMINATION_GRACE);
        return Ok("killed_process_group".to_string());
    }

    // Fallback for platforms/configurations where the owner process group
    // cannot be resolved. The PID identity guard above still protects against
    // killing a reused PID.
    send_signal_to_pid(pid, libc::SIGTERM)?;
    if wait_for_owner_exit(pid, RUN_OWNER_TERMINATION_GRACE) {
        Ok("terminated_owner".to_string())
    } else {
        send_signal_to_pid(pid, libc::SIGKILL)?;
        let _ = wait_for_owner_exit(pid, RUN_OWNER_TERMINATION_GRACE);
        Ok("killed_owner".to_string())
    }
}

#[cfg(not(unix))]
pub(super) fn signal_run_owner_process(_run: &JobRun) -> Result<String, OrbitError> {
    Ok("unsupported_platform".to_string())
}

#[cfg(unix)]
fn send_signal_to_pid(pid: u32, signal: libc::c_int) -> Result<(), OrbitError> {
    let rc = unsafe { libc::kill(pid as libc::pid_t, signal) };
    if rc == 0 {
        return Ok(());
    }
    let err = std::io::Error::last_os_error();
    if err.raw_os_error() == Some(libc::ESRCH) {
        return Ok(());
    }
    Err(OrbitError::Execution(format!(
        "failed to signal job run owner pid {pid}: {err}",
    )))
}

#[cfg(unix)]
fn owner_process_group_id(pid: u32) -> Option<libc::pid_t> {
    if pid == 0 || pid > i32::MAX as u32 {
        return None;
    }
    let pgid = unsafe { libc::getpgid(pid as libc::pid_t) };
    if pgid > 0 { Some(pgid) } else { None }
}

#[cfg(unix)]
fn send_signal_to_process_group(pgid: libc::pid_t, signal: libc::c_int) -> std::io::Result<()> {
    let rc = unsafe { libc::kill(-pgid, signal) };
    if rc == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}

#[cfg(unix)]
fn wait_for_owner_exit(pid: u32, timeout: Duration) -> bool {
    let started = Instant::now();
    while started.elapsed() < timeout {
        if !process_is_alive(pid) {
            return true;
        }
        thread::sleep(RUN_OWNER_TERMINATION_POLL);
    }
    !process_is_alive(pid)
}

#[cfg(unix)]
fn wait_for_process_group_exit(pgid: libc::pid_t, timeout: Duration) -> bool {
    let started = Instant::now();
    while started.elapsed() < timeout {
        if !process_group_is_alive(pgid) {
            return true;
        }
        thread::sleep(RUN_OWNER_TERMINATION_POLL);
    }
    !process_group_is_alive(pgid)
}

#[cfg(unix)]
fn process_group_is_alive(pgid: libc::pid_t) -> bool {
    if pgid <= 1 {
        return false;
    }
    let rc = unsafe { libc::kill(-pgid, 0) };
    if rc == 0 {
        return true;
    }
    std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
}

/// Returns true only for Running runs whose owner is conclusively stale
/// (Mismatch or Missing). Other classifications keep the run live.
#[cfg(unix)]
pub(super) fn running_run_owner_is_stale(run: &JobRun) -> bool {
    running_run_owner_stale_reason(run).is_some()
}

#[cfg(not(unix))]
pub(super) fn running_run_owner_is_stale(_run: &JobRun) -> bool {
    false
}

/// Returns `Some(reason)` only when a running run's owner is conclusively
/// either mismatched or missing — those are the two states that warrant
/// finalizing the run as failed. `ProbeUnavailable` and `LegacyLiveUnverified`
/// classifications never appear here: they keep the run Running.
#[cfg(unix)]
pub(super) fn running_run_owner_stale_reason(run: &JobRun) -> Option<OwnerIdentity> {
    if run.state != JobRunState::Running {
        return None;
    }
    match classify_run_owner(run) {
        identity @ (OwnerIdentity::Mismatch | OwnerIdentity::Missing) => Some(identity),
        OwnerIdentity::Verified
        | OwnerIdentity::LegacyLiveUnverified
        | OwnerIdentity::ProbeUnavailable => None,
    }
}

#[cfg(not(unix))]
#[allow(dead_code)]
pub(super) fn running_run_owner_stale_reason(_run: &JobRun) -> Option<()> {
    None
}

/// Outcome of comparing a persisted owner identity against the live process.
///
/// Only `Mismatch` and `Missing` warrant finalizing the run as failed.
///
/// - `Verified` — versioned token (or legacy token re-derived under either
///   environment) matches the live process: the worker is the original owner.
/// - `Mismatch` — versioned persisted token disagrees with the live process's
///   current token: a different process is holding the PID. Stale.
/// - `LegacyLiveUnverified` — legacy (pre-ORB-00036) unversioned token cannot
///   be re-derived under either environment, but `kill(pid, 0)` confirms the
///   PID is still alive. Stays Running; cancellation still refuses to signal
///   it (PID-reuse protection).
/// - `ProbeUnavailable` — the `ps` invocation itself failed (spawn error,
///   IO error, etc.) and `kill(pid, 0)` confirms the PID is still alive.
///   A transient probe failure must never terminalize a live worker.
/// - `Missing` — no PID recorded, or both the probe and `kill(pid, 0)`
///   agree the PID is gone. Stale.
#[cfg(unix)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum OwnerIdentity {
    Verified,
    Mismatch,
    LegacyLiveUnverified,
    ProbeUnavailable,
    Missing,
}

#[cfg(unix)]
pub(super) fn classify_run_owner(run: &JobRun) -> OwnerIdentity {
    classify_run_owner_with_probes(
        run.pid,
        run.pid_start_time.as_deref(),
        probe_process_start_identity,
        |pid| legacy_lstart_matches(pid, run.pid_start_time.as_deref().unwrap_or_default()),
        process_is_alive,
    )
}

/// Inner, testable form of [`classify_run_owner`] with the probes injected.
/// Production callers go through [`classify_run_owner`]; tests pass
/// deterministic closures to exercise rare probe states (Unavailable,
/// NoProcess-but-alive race) without needing real misbehaving processes.
#[cfg(unix)]
pub(super) fn classify_run_owner_with_probes<P, L, A>(
    pid: Option<u32>,
    persisted: Option<&str>,
    probe: P,
    legacy_match: L,
    is_alive: A,
) -> OwnerIdentity
where
    P: FnOnce(u32) -> ProbeOutcome,
    L: FnOnce(u32) -> bool,
    A: FnOnce(u32) -> bool,
{
    let Some(pid) = pid else {
        return OwnerIdentity::Missing;
    };
    let Some(persisted) = persisted else {
        return if is_alive(pid) {
            OwnerIdentity::LegacyLiveUnverified
        } else {
            OwnerIdentity::Missing
        };
    };
    if persisted.starts_with(STABLE_TOKEN_PREFIX) {
        return match probe(pid) {
            ProbeOutcome::Token(current) if current == persisted => OwnerIdentity::Verified,
            ProbeOutcome::Token(_) => OwnerIdentity::Mismatch,
            ProbeOutcome::NoProcess => {
                if is_alive(pid) {
                    // Race: `ps` returned no-process but `kill(pid, 0)` still
                    // sees the PID. Defer finalization until the probe agrees.
                    OwnerIdentity::ProbeUnavailable
                } else {
                    OwnerIdentity::Missing
                }
            }
            ProbeOutcome::Unavailable => {
                if is_alive(pid) {
                    OwnerIdentity::ProbeUnavailable
                } else {
                    OwnerIdentity::Missing
                }
            }
        };
    }
    if legacy_match(pid) {
        OwnerIdentity::Verified
    } else if is_alive(pid) {
        OwnerIdentity::LegacyLiveUnverified
    } else {
        OwnerIdentity::Missing
    }
}

#[cfg(unix)]
pub(super) fn process_is_alive(pid: u32) -> bool {
    if pid == 0 || pid > i32::MAX as u32 {
        return false;
    }
    // Safety: signal 0 performs existence/permission checking only.
    let rc = unsafe { libc::kill(pid as libc::pid_t, 0) };
    if rc == 0 {
        return true;
    }
    std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
}

/// Builds the diagnostic message recorded in the failure step when a stale
/// owner causes a Running run to be reconciled to Failed.
#[cfg(unix)]
pub(super) fn stale_job_run_message(run: &JobRun, reason: Option<OwnerIdentity>) -> String {
    let reason_str = match reason {
        Some(OwnerIdentity::Mismatch) => "token_mismatch",
        Some(OwnerIdentity::Missing) => "process_not_found",
        // ProbeUnavailable / Verified / LegacyLiveUnverified never reach the
        // stale-message path, but a future caller could; keep them tagged so
        // the diagnostic is never silently wrong.
        Some(OwnerIdentity::ProbeUnavailable) => "probe_unavailable",
        Some(OwnerIdentity::Verified) => "verified",
        Some(OwnerIdentity::LegacyLiveUnverified) => "legacy_live_unverified",
        None => "unknown",
    };
    format!(
        "job run marked failed because recorded worker process is no longer alive (reason={}, pid={}, pid_start_time={})",
        reason_str,
        run.pid
            .map(|pid| pid.to_string())
            .unwrap_or_else(|| "-".to_string()),
        run.pid_start_time.as_deref().unwrap_or("-")
    )
}

#[cfg(not(unix))]
pub(super) fn stale_job_run_message(run: &JobRun, _reason: Option<()>) -> String {
    format!(
        "job run marked failed because recorded worker process is no longer alive (reason=unknown, pid={}, pid_start_time={})",
        run.pid
            .map(|pid| pid.to_string())
            .unwrap_or_else(|| "-".to_string()),
        run.pid_start_time.as_deref().unwrap_or("-")
    )
}
