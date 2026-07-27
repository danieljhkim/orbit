//! `Embedder` implementation that talks to the installed companion binary
//! over JSON-Lines stdio. The subprocess is kept alive across requests via
//! a `Mutex<ChildIo>`; `Drop` sends `Exit` and reaps the child.
//!
//! ## Retry hygiene (ORB-10006)
//!
//! Transport-level failures — spawn resource exhaustion, a crashed/exited
//! companion (EOF), broken pipes — are transient: the request is retried a
//! bounded number of times with exponential backoff + full jitter,
//! respawning the companion between attempts. Companion-reported RPC errors
//! and protocol violations are permanent and surface immediately.

use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread;
use std::time::Duration;

use orbit_common::types::OrbitError;
use orbit_common::utility::jitter::JitterRng;

use crate::companion::locate_companion;
use crate::embedder::{DEFAULT_MODEL, Embedder};
use crate::rpc::{RpcRequest, RpcResponse, RpcResult, rpc_error_to_orbit};

/// Total request attempts (first try + respawn retries).
const RPC_MAX_ATTEMPTS: u32 = 3;
/// Base of the exponential backoff bound between attempts.
const RPC_RETRY_INITIAL_BACKOFF_MS: u64 = 50;
/// Cap on the backoff bound.
const RPC_RETRY_BACKOFF_CAP_MS: u64 = 1_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CompanionStderr {
    Inherit,
    Suppress,
}

/// Classification of a single RPC attempt failure (ORB-10006).
pub(crate) enum RequestFailure {
    /// The companion is gone or the pipe broke — a respawn may fix it.
    Transient(String),
    /// Deterministic failure (companion-reported error, protocol violation,
    /// serialization) — retrying cannot fix it.
    Permanent(OrbitError),
}

/// Whether a spawn `io::Error` is deterministic. `NotFound` / rejected
/// permissions won't change between retry attempts; resource exhaustion
/// (EAGAIN, ENOMEM, EMFILE, ...) and anything unrecognized stay transient.
pub(crate) fn spawn_error_is_permanent(error: &std::io::Error) -> bool {
    matches!(
        error.kind(),
        std::io::ErrorKind::NotFound | std::io::ErrorKind::PermissionDenied
    )
}

/// Deterministic exponential bound for the jittered sleep before retry
/// `attempt` (1-based): `min(cap, initial * 2^(attempt-1))`.
pub(crate) fn retry_backoff_bound_ms(attempt: u32) -> u64 {
    RPC_RETRY_INITIAL_BACKOFF_MS
        .saturating_mul(1u64 << attempt.saturating_sub(1).min(20))
        .min(RPC_RETRY_BACKOFF_CAP_MS)
}

pub struct SubprocessEmbedder {
    model_id: String,
    dim: usize,
    max_input_tokens: usize,
    next_id: AtomicU64,
    io: Mutex<ChildIo>,
    /// Respawn context: the companion path/model/stderr the child was
    /// started with, reused when a transport failure forces a respawn.
    companion_path: PathBuf,
    model_arg: String,
    stderr_mode: CompanionStderr,
}

struct ChildIo {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
}

impl SubprocessEmbedder {
    pub fn new() -> Result<Self, OrbitError> {
        Self::with_model(DEFAULT_MODEL)
    }

    pub fn with_model(model: &str) -> Result<Self, OrbitError> {
        Self::with_path_and_model(locate_companion()?, model)
    }

    pub fn with_path_and_model(path: PathBuf, model: &str) -> Result<Self, OrbitError> {
        Self::with_path_model_and_stderr(path, model, CompanionStderr::Inherit)
    }

    pub(crate) fn quiet_with_model(model: &str) -> Result<Self, OrbitError> {
        Self::with_path_model_and_stderr(locate_companion()?, model, CompanionStderr::Suppress)
    }

    fn with_path_model_and_stderr(
        path: PathBuf,
        model: &str,
        stderr: CompanionStderr,
    ) -> Result<Self, OrbitError> {
        let io = spawn_companion_with_retry(&path, model, stderr)?;
        let mut embedder = Self {
            model_id: String::new(),
            dim: 0,
            max_input_tokens: 0,
            next_id: AtomicU64::new(1),
            io: Mutex::new(io),
            companion_path: path,
            model_arg: model.to_string(),
            stderr_mode: stderr,
        };
        let info = embedder.request(RpcRequest::Info { id: 0 })?;
        let RpcResult::Info {
            model_id,
            dim,
            max_input_tokens,
            ..
        } = info
        else {
            return Err(OrbitError::AgentProtocolViolation(
                "companion returned non-info response to info request".to_string(),
            ));
        };
        embedder.model_id = model_id;
        embedder.dim = dim;
        embedder.max_input_tokens = max_input_tokens;
        Ok(embedder)
    }

    fn request(&self, request: RpcRequest) -> Result<RpcResult, OrbitError> {
        let request = match request {
            RpcRequest::Info { id: 0 } => RpcRequest::Info { id: 1 },
            RpcRequest::Info { .. } => RpcRequest::Info {
                id: self.next_request_id(),
            },
            RpcRequest::Embed { texts, .. } => RpcRequest::Embed {
                id: self.next_request_id(),
                texts,
            },
            RpcRequest::TokenCount { text, .. } => RpcRequest::TokenCount {
                id: self.next_request_id(),
                text,
            },
            RpcRequest::Exit { .. } => RpcRequest::Exit {
                id: self.next_request_id(),
            },
        };
        let line = serde_json::to_string(&request)
            .map_err(|error| OrbitError::Execution(error.to_string()))?;
        let id = request.id();

        let mut io = self
            .io
            .lock()
            .map_err(|error| OrbitError::Execution(format!("companion mutex poisoned: {error}")))?;
        let mut jitter = JitterRng::seeded(&self.model_arg);
        let mut last_transient = String::new();
        for attempt in 0..RPC_MAX_ATTEMPTS {
            if attempt > 0 {
                let sleep_ms = jitter.full_jitter(retry_backoff_bound_ms(attempt));
                thread::sleep(Duration::from_millis(sleep_ms));
                match spawn_companion_child(&self.companion_path, &self.model_arg, self.stderr_mode)
                {
                    Ok(fresh) => {
                        // Reap the dead/wedged child before dropping its
                        // handles so it doesn't linger as a zombie.
                        let _ = io.child.kill();
                        let _ = io.child.wait();
                        *io = fresh;
                    }
                    Err(error) => {
                        if spawn_error_is_permanent(&error) {
                            return Err(spawn_error_to_orbit(&self.companion_path, &error));
                        }
                        last_transient = format!("companion respawn failed: {error}");
                        tracing::warn!(
                            attempt,
                            error = %error,
                            "search companion respawn failed; will retry"
                        );
                        continue;
                    }
                }
            }
            match request_once(&mut io, &line, id) {
                Ok(result) => return Ok(result),
                Err(RequestFailure::Permanent(error)) => return Err(error),
                Err(RequestFailure::Transient(message)) => {
                    tracing::warn!(
                        attempt,
                        error = %message,
                        "search companion RPC transport failure; respawning companion"
                    );
                    last_transient = message;
                }
            }
        }
        Err(OrbitError::Execution(format!(
            "search companion RPC failed after {RPC_MAX_ATTEMPTS} attempts: {last_transient}"
        )))
    }

    fn next_request_id(&self) -> u64 {
        self.next_id.fetch_add(1, Ordering::Relaxed)
    }
}

/// One request/response round-trip against the current companion child.
/// Transport failures (write/read errors, EOF) are transient; malformed or
/// mismatched responses and companion-reported errors are permanent.
fn request_once(io: &mut ChildIo, line: &str, id: u64) -> Result<RpcResult, RequestFailure> {
    io.stdin
        .write_all(line.as_bytes())
        .and_then(|_| io.stdin.write_all(b"\n"))
        .and_then(|_| io.stdin.flush())
        .map_err(|error| {
            RequestFailure::Transient(format!("failed to write companion RPC: {error}"))
        })?;

    let mut response_line = String::new();
    let read = io.stdout.read_line(&mut response_line).map_err(|error| {
        RequestFailure::Transient(format!("failed to read companion RPC: {error}"))
    })?;
    if read == 0 {
        return Err(RequestFailure::Transient(
            "search companion exited before sending a response".to_string(),
        ));
    }
    let response: RpcResponse = serde_json::from_str(&response_line).map_err(|error| {
        RequestFailure::Permanent(OrbitError::AgentProtocolViolation(error.to_string()))
    })?;
    match response {
        RpcResponse::Result {
            id: response_id,
            result,
        } if response_id == id => Ok(result),
        RpcResponse::Error {
            id: response_id,
            error,
        } if response_id == id => Err(RequestFailure::Permanent(rpc_error_to_orbit(error))),
        other => Err(RequestFailure::Permanent(
            OrbitError::AgentProtocolViolation(format!(
                "companion response id mismatch for request {id}: {other:?}"
            )),
        )),
    }
}

/// Spawn the companion child, retrying transient spawn failures (resource
/// exhaustion) with jittered backoff. Deterministic failures — binary
/// missing, permission denied — surface immediately.
fn spawn_companion_with_retry(
    path: &Path,
    model: &str,
    stderr: CompanionStderr,
) -> Result<ChildIo, OrbitError> {
    let mut jitter = JitterRng::seeded(model);
    let mut last_error: Option<std::io::Error> = None;
    for attempt in 0..RPC_MAX_ATTEMPTS {
        if attempt > 0 {
            let sleep_ms = jitter.full_jitter(retry_backoff_bound_ms(attempt));
            thread::sleep(Duration::from_millis(sleep_ms));
        }
        match spawn_companion_child(path, model, stderr) {
            Ok(io) => return Ok(io),
            Err(error) => {
                if spawn_error_is_permanent(&error) {
                    return Err(spawn_error_to_orbit(path, &error));
                }
                tracing::warn!(
                    attempt,
                    error = %error,
                    "transient search companion spawn failure; will retry"
                );
                last_error = Some(error);
            }
        }
    }
    match last_error {
        Some(error) => Err(spawn_error_to_orbit(path, &error)),
        None => Err(OrbitError::Execution(
            "failed to spawn search companion".to_string(),
        )),
    }
}

fn spawn_error_to_orbit(path: &Path, error: &std::io::Error) -> OrbitError {
    OrbitError::Execution(format!(
        "failed to spawn search companion '{}': {error}",
        path.display()
    ))
}

/// Single spawn attempt; returns the raw `io::Error` so callers can classify.
fn spawn_companion_child(
    path: &Path,
    model: &str,
    stderr: CompanionStderr,
) -> Result<ChildIo, std::io::Error> {
    let mut child = Command::new(path)
        .arg("--model")
        .arg(model)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(stderr.stdio())
        .spawn()?;
    let stdin = child
        .stdin
        .take()
        .ok_or_else(|| std::io::Error::other("companion stdin unavailable"))?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| std::io::Error::other("companion stdout unavailable"))?;
    Ok(ChildIo {
        child,
        stdin,
        stdout: BufReader::new(stdout),
    })
}

impl CompanionStderr {
    fn stdio(self) -> Stdio {
        match self {
            Self::Inherit => Stdio::inherit(),
            Self::Suppress => Stdio::null(),
        }
    }
}

impl Embedder for SubprocessEmbedder {
    fn model_id(&self) -> &str {
        &self.model_id
    }

    fn dim(&self) -> usize {
        self.dim
    }

    fn max_input_tokens(&self) -> usize {
        self.max_input_tokens
    }

    fn embed(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>, OrbitError> {
        let result = self.request(RpcRequest::Embed {
            id: 0,
            texts: texts.iter().map(|text| (*text).to_string()).collect(),
        })?;
        match result {
            RpcResult::Embed { vectors } => Ok(vectors),
            _ => Err(OrbitError::AgentProtocolViolation(
                "companion returned non-embed response to embed request".to_string(),
            )),
        }
    }

    fn token_count(&self, text: &str) -> Result<usize, OrbitError> {
        let result = self.request(RpcRequest::TokenCount {
            id: 0,
            text: text.to_string(),
        })?;
        match result {
            RpcResult::TokenCount { tokens } => Ok(tokens),
            _ => Err(OrbitError::AgentProtocolViolation(
                "companion returned non-token_count response".to_string(),
            )),
        }
    }
}

impl Drop for SubprocessEmbedder {
    fn drop(&mut self) {
        let Ok(mut io) = self.io.lock() else {
            return;
        };
        if let Ok(line) = serde_json::to_string(&RpcRequest::Exit { id: 9_999_999 }) {
            let _ = io.stdin.write_all(line.as_bytes());
            let _ = io.stdin.write_all(b"\n");
            let _ = io.stdin.flush();
            let mut response = String::new();
            let _ = io.stdout.read_line(&mut response);
        }
        let _ = io.child.wait();
    }
}
