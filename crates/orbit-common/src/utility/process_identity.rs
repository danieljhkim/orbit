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

/// Liveness of a PID recorded earlier in a run, judged against the identity
/// token captured when the PID was recorded [ORB-10496].
///
/// The distinction that matters to an operator is `Alive` vs `Exited`: a
/// long-running agent subprocess and a dead one otherwise look identical from
/// outside the process tree. `Unknown` means the host cannot answer (non-Unix),
/// and must never be read as "dead".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProcessLiveness {
    /// The PID exists and is not a reaped zombie. When an identity token was
    /// recorded it also still matches, so this is the same process.
    Alive,
    /// The PID is gone, is an unreaped zombie, or is now held by a different
    /// process (recorded identity token no longer matches).
    Exited,
    /// Liveness cannot be probed on this platform.
    Unknown,
}

impl ProcessLiveness {
    /// Stable wire token for JSON/table surfaces.
    pub fn as_str(self) -> &'static str {
        match self {
            ProcessLiveness::Alive => "alive",
            ProcessLiveness::Exited => "exited",
            ProcessLiveness::Unknown => "unknown",
        }
    }
}

/// Probe whether a recorded PID is still the live process it was recorded as.
///
/// `pid_start_time` is the token captured alongside the PID (see
/// [`process_start_identity_token`]). It guards against PID reuse: a live PID
/// whose versioned token disagrees with the current process is reported
/// `Exited`, because the recorded process is gone even though the number was
/// recycled. A live PID with no token, or one whose probe could not run (`ps`
/// unavailable in a sandbox), stays `Alive` — a probe that cannot answer must
/// not be enough to declare a running process dead.
#[cfg(unix)]
pub fn probe_process_liveness(pid: u32, pid_start_time: Option<&str>) -> ProcessLiveness {
    if !process_is_alive(pid) {
        return ProcessLiveness::Exited;
    }
    let Some(persisted) = pid_start_time else {
        return ProcessLiveness::Alive;
    };
    if !persisted.starts_with(STABLE_TOKEN_PREFIX) {
        // A legacy unversioned token that cannot be re-derived is unverifiable,
        // not falsified. The PID is alive either way.
        return ProcessLiveness::Alive;
    }
    match probe_process_start_identity(pid) {
        ProbeOutcome::Token(current) if current == persisted => ProcessLiveness::Alive,
        ProbeOutcome::Token(_) => ProcessLiveness::Exited,
        // `kill(pid, 0)` above already saw the PID; a `ps` that disagrees is a
        // race or a blocked probe, not proof of death.
        ProbeOutcome::NoProcess | ProbeOutcome::Unavailable => ProcessLiveness::Alive,
    }
}

#[cfg(not(unix))]
pub fn probe_process_liveness(_pid: u32, _pid_start_time: Option<&str>) -> ProcessLiveness {
    ProcessLiveness::Unknown
}

/// True when `pid` names a process that can still run.
///
/// Linux keeps unreaped zombies visible to `kill(pid, 0)`; they are reported
/// dead here because they can no longer do work. `EPERM` counts as alive: the
/// process exists, we merely may not signal it.
#[cfg(unix)]
pub fn process_is_alive(pid: u32) -> bool {
    if pid == 0 || pid > i32::MAX as u32 {
        return false;
    }
    #[cfg(target_os = "linux")]
    if matches!(linux_process_state(pid), Some(('Z', _))) {
        return false;
    }
    // Safety: signal 0 performs existence/permission checking only.
    let rc = unsafe { libc::kill(pid as libc::pid_t, 0) };
    if rc == 0 {
        return true;
    }
    std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
}

#[cfg(not(unix))]
pub fn process_is_alive(_pid: u32) -> bool {
    false
}

/// Linux `/proc/<pid>/stat` run state and process-group id.
///
/// Exposed so callers that need the zombie distinction for a *group* (not just
/// a single PID) can reuse the same parse instead of re-deriving it.
#[cfg(target_os = "linux")]
pub fn linux_process_state(pid: u32) -> Option<(char, libc::pid_t)> {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    let (_, tail) = stat.rsplit_once(')')?;
    let mut fields = tail.split_whitespace();
    let state = fields.next()?.chars().next()?;
    let _parent_pid = fields.next()?;
    let process_group = fields.next()?.parse().ok()?;
    Some((state, process_group))
}
