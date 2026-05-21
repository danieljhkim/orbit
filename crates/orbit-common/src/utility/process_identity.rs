//! Stable cross-process identity tokens for job-run owner verification.
//!
//! On Unix, the token is derived from `ps -o lstart=` with the child
//! environment forced to `TZ=UTC` / `LC_ALL=C` / `LANG=C` so the persisted
//! value does not depend on the caller's locale or timezone. Tokens written
//! by this helper carry a [`STABLE_TOKEN_PREFIX`] so readers can distinguish
//! them from legacy unversioned values.

/// Prefix on versioned identity tokens. Persisted tokens that start with this
/// prefix were written by the stable strategy and must match exactly.
pub const STABLE_TOKEN_PREFIX: &str = "ps-lstart-utc-v1:";

/// Outcome of a single process-start identity probe.
///
/// Readers must distinguish a *probe failure* (`Unavailable`) from a
/// *process-not-found* result (`NoProcess`): the former indicates we cannot
/// tell whether the PID is alive (transient `ps` spawn error, etc.) and must
/// not be enough to terminalize a still-running worker.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProbeOutcome {
    /// `ps` succeeded and produced a non-empty versioned token.
    Token(String),
    /// `ps` exited non-zero or returned an empty token; the kernel has no
    /// process with this PID.
    NoProcess,
    /// `Command::output()` itself errored (spawn/IO failure). Identity cannot
    /// be probed; defer to other liveness signals.
    Unavailable,
}

#[cfg(unix)]
use std::io;
#[cfg(unix)]
use std::process::Command;

#[cfg(unix)]
fn lstart_raw(pid: u32, stable_env: bool) -> Result<Option<String>, io::Error> {
    let mut cmd = Command::new("ps");
    cmd.args(["-o", "lstart=", "-p", &pid.to_string()]);
    if stable_env {
        cmd.env("TZ", "UTC").env("LC_ALL", "C").env("LANG", "C");
    }
    let output = cmd.output()?;
    if !output.status.success() {
        return Ok(None);
    }
    let token = String::from_utf8_lossy(&output.stdout).trim().to_string();
    Ok((!token.is_empty()).then_some(token))
}

/// Probe a running process and classify the outcome.
#[cfg(unix)]
pub fn probe_process_start_identity(pid: u32) -> ProbeOutcome {
    match lstart_raw(pid, true) {
        Ok(Some(raw)) => ProbeOutcome::Token(format!("{STABLE_TOKEN_PREFIX}{raw}")),
        Ok(None) => ProbeOutcome::NoProcess,
        Err(_) => ProbeOutcome::Unavailable,
    }
}

#[cfg(not(unix))]
pub fn probe_process_start_identity(_pid: u32) -> ProbeOutcome {
    ProbeOutcome::Unavailable
}

/// Versioned, locale/timezone-stable process-start identity token. Writers
/// (and readers that only need a `String`) call this; consumers that need to
/// distinguish probe-not-found from probe-unavailable call
/// [`probe_process_start_identity`] instead.
pub fn process_start_identity_token(pid: u32) -> Option<String> {
    match probe_process_start_identity(pid) {
        ProbeOutcome::Token(s) => Some(s),
        ProbeOutcome::NoProcess | ProbeOutcome::Unavailable => None,
    }
}

/// Returns true when `persisted` is a legacy unversioned token whose value
/// matches either the caller-environment `ps -o lstart=` output or the
/// stable-environment one for this PID. Versioned tokens always return false
/// here so callers route them through [`process_start_identity_token`].
#[cfg(unix)]
pub fn legacy_lstart_matches(pid: u32, persisted: &str) -> bool {
    if persisted.starts_with(STABLE_TOKEN_PREFIX) {
        return false;
    }
    if let Ok(Some(stable_raw)) = lstart_raw(pid, true)
        && stable_raw == persisted
    {
        return true;
    }
    if let Ok(Some(ambient)) = lstart_raw(pid, false)
        && ambient == persisted
    {
        return true;
    }
    false
}

#[cfg(not(unix))]
pub fn legacy_lstart_matches(_pid: u32, _persisted: &str) -> bool {
    false
}

#[cfg(all(test, unix))]
mod tests;
