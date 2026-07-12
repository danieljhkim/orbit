//! Loopback client for the worker invoke daemon [ORB-10146].
//!
//! qa-sweep v2 submits a QA agent run per workspace to the worker daemon (the
//! same loopback service bridge's `agent_invoke` uses): `POST /invoke` enqueues
//! a run and returns a `run_id` (202, async), `GET /runs/{run_id}` polls it to a
//! terminal state, and `DELETE /runs/{run_id}` cancels it. The final agent
//! output text lives in `result.result` on a terminal run record.
//!
//! The HTTP surface is a thin wrapper; the request-body construction, response
//! parsing, terminal-state classification, and poll-until-terminal loop are all
//! pure/injectable so the sweep's worker-client failure paths (daemon down,
//! timeout, bad JSON) are unit-testable without a live daemon.

use std::time::{Duration, Instant};

use reqwest::blocking::Client;
use serde_json::Value;

/// Per-request HTTP timeout for submit/poll/cancel calls. The daemon answers
/// these promptly (`/invoke` is async); the *run's* budget is enforced by the
/// worker via `limits.wall_clock_secs`, not this timeout.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
/// Interval between run-status polls.
pub(crate) const POLL_INTERVAL: Duration = Duration::from_secs(5);

/// Terminal worker run statuses (from the daemon's `RunStatus` enum). Any other
/// status (`queued`, `running`, `unknown`) means keep polling.
const TERMINAL_STATUSES: &[&str] = &[
    "ok",
    "error",
    "timeout",
    "max_turns",
    "cost_exceeded",
    "cancelled",
    "interrupted",
];

/// The status a successfully-completed run reports.
pub(crate) const STATUS_OK: &str = "ok";

/// One QA agent run request to the worker daemon.
#[derive(Debug, Clone)]
pub(crate) struct WorkerRunRequest {
    /// The composed QA prompt.
    pub prompt: String,
    /// Worker provider family (`claude`, `codex`, `gemini`, `grok`).
    pub provider: String,
    /// Provider model string.
    pub model: String,
    /// Working directory for the run (the workspace repo root).
    pub cwd: String,
    /// Wall-clock budget in seconds (`limits.wall_clock_secs`).
    pub wall_clock_secs: u64,
    /// Turn budget (`limits.max_turns`); omitted for providers that reject it
    /// (notably Codex), so the caller passes `None` there.
    pub max_turns: Option<u32>,
    /// Explicit serialization lease key so concurrent sweeps do not run two
    /// agents against the same checkout at once.
    pub serialization_key: Option<String>,
}

/// A run's status as seen by one poll.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct WorkerRunStatus {
    /// Raw status string.
    pub status: String,
    /// Final agent output text, present once the run has executed.
    pub report_text: Option<String>,
}

/// A run that reached a terminal state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct WorkerTerminal {
    /// Terminal status string (`ok`, `error`, `timeout`, ...).
    pub status: String,
    /// Final agent output text, when present.
    pub report_text: Option<String>,
}

/// Failure of a worker interaction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum WorkerError {
    /// Could not connect to the daemon (down / wrong URL).
    Unreachable(String),
    /// Submit was rejected or otherwise failed.
    Submit(String),
    /// A poll request failed.
    Poll(String),
    /// A response was not the expected JSON shape.
    BadResponse(String),
    /// The run did not reach a terminal state within the budget.
    Timeout { run_id: String, waited_secs: u64 },
}

impl std::fmt::Display for WorkerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unreachable(message) => write!(f, "worker daemon unreachable: {message}"),
            Self::Submit(message) => write!(f, "worker run submission failed: {message}"),
            Self::Poll(message) => write!(f, "worker run poll failed: {message}"),
            Self::BadResponse(message) => write!(f, "worker returned a bad response: {message}"),
            Self::Timeout {
                run_id,
                waited_secs,
            } => write!(
                f,
                "worker run {run_id} did not finish within {waited_secs}s"
            ),
        }
    }
}

/// Blocking loopback client for the worker invoke daemon.
pub(crate) struct WorkerClient {
    base_url: String,
    http: Client,
}

impl WorkerClient {
    /// Build a client against `base_url` (no trailing slash).
    pub(crate) fn new(base_url: &str) -> Result<Self, WorkerError> {
        let http = Client::builder()
            .timeout(REQUEST_TIMEOUT)
            .build()
            .map_err(|error| WorkerError::Submit(format!("build http client: {error}")))?;
        Ok(Self {
            base_url: base_url.trim_end_matches('/').to_string(),
            http,
        })
    }

    /// Submit a run and return its `run_id` (does not wait for completion).
    pub(crate) fn submit(&self, request: &WorkerRunRequest) -> Result<String, WorkerError> {
        let body = build_invoke_body(request);
        let response = self
            .http
            .post(format!("{}/invoke", self.base_url))
            .json(&body)
            .send()
            .map_err(|error| send_error("submit run", error))?;
        let status = response.status();
        let text = response
            .text()
            .map_err(|error| WorkerError::BadResponse(format!("read /invoke body: {error}")))?;
        if !status.is_success() {
            return Err(WorkerError::Submit(format!(
                "worker /invoke returned HTTP {}: {}",
                status.as_u16(),
                text.trim()
            )));
        }
        parse_submit_response(&text)
    }

    /// Fetch one run's current status.
    pub(crate) fn get_run(&self, run_id: &str) -> Result<WorkerRunStatus, WorkerError> {
        let response = self
            .http
            .get(format!("{}/runs/{run_id}", self.base_url))
            .send()
            .map_err(|error| send_error("poll run", error))?;
        let status = response.status();
        let text = response
            .text()
            .map_err(|error| WorkerError::BadResponse(format!("read /runs body: {error}")))?;
        if !status.is_success() {
            return Err(WorkerError::Poll(format!(
                "worker /runs/{run_id} returned HTTP {}: {}",
                status.as_u16(),
                text.trim()
            )));
        }
        parse_run_status(&text)
    }

    /// Best-effort cancel of a run (used when the sweep gives up on a timeout).
    pub(crate) fn cancel(&self, run_id: &str) {
        let _ = self
            .http
            .delete(format!("{}/runs/{run_id}", self.base_url))
            .send();
    }

    /// Submit a run and poll it to a terminal state, cancelling on timeout.
    pub(crate) fn run_to_terminal(
        &self,
        request: &WorkerRunRequest,
        timeout: Duration,
    ) -> Result<(String, WorkerTerminal), WorkerError> {
        let run_id = self.submit(request)?;
        match await_terminal(timeout, POLL_INTERVAL, &run_id, || self.get_run(&run_id)) {
            Ok(terminal) => Ok((run_id, terminal)),
            Err(error) => {
                if matches!(error, WorkerError::Timeout { .. }) {
                    self.cancel(&run_id);
                }
                Err(error)
            }
        }
    }
}

/// Map a reqwest transport error to a typed worker error, treating connection
/// failures as an unreachable daemon.
fn send_error(context: &str, error: reqwest::Error) -> WorkerError {
    if error.is_connect() {
        WorkerError::Unreachable(format!("{context}: {error}"))
    } else if error.is_timeout() {
        WorkerError::Poll(format!("{context}: request timed out: {error}"))
    } else {
        WorkerError::BadResponse(format!("{context}: {error}"))
    }
}

/// Build the `POST /invoke` request body. `max_turns` is included only when set
/// (Codex rejects the control), and `max_cost_usd` is left to the daemon.
pub(crate) fn build_invoke_body(request: &WorkerRunRequest) -> Value {
    let mut limits = serde_json::Map::new();
    limits.insert(
        "wall_clock_secs".to_string(),
        Value::from(request.wall_clock_secs),
    );
    if let Some(max_turns) = request.max_turns {
        limits.insert("max_turns".to_string(), Value::from(max_turns));
    }

    let mut body = serde_json::Map::new();
    body.insert("prompt".to_string(), Value::from(request.prompt.clone()));
    body.insert(
        "provider".to_string(),
        Value::from(request.provider.clone()),
    );
    body.insert("model".to_string(), Value::from(request.model.clone()));
    body.insert("cwd".to_string(), Value::from(request.cwd.clone()));
    body.insert("limits".to_string(), Value::Object(limits));
    if let Some(key) = &request.serialization_key {
        body.insert("serialization_key".to_string(), Value::from(key.clone()));
    }
    Value::Object(body)
}

/// Extract the `run_id` from a `POST /invoke` 202 body.
pub(crate) fn parse_submit_response(text: &str) -> Result<String, WorkerError> {
    let value: Value = serde_json::from_str(text)
        .map_err(|error| WorkerError::BadResponse(format!("parse /invoke body: {error}")))?;
    value
        .get("run_id")
        .and_then(Value::as_str)
        .filter(|run_id| !run_id.trim().is_empty())
        .map(str::to_string)
        .ok_or_else(|| WorkerError::BadResponse("/invoke response missing 'run_id'".to_string()))
}

/// Parse a `GET /runs/{id}` body into a status + optional final text.
pub(crate) fn parse_run_status(text: &str) -> Result<WorkerRunStatus, WorkerError> {
    let value: Value = serde_json::from_str(text)
        .map_err(|error| WorkerError::BadResponse(format!("parse /runs body: {error}")))?;
    let status = value
        .get("status")
        .and_then(Value::as_str)
        .filter(|status| !status.trim().is_empty())
        .ok_or_else(|| WorkerError::BadResponse("/runs response missing 'status'".to_string()))?
        .to_string();
    Ok(WorkerRunStatus {
        status,
        report_text: extract_result_text(&value),
    })
}

/// The agent's final text from a run record: `result.result` when `result` is
/// an object, or the bare string on a dispatch-failure record.
fn extract_result_text(value: &Value) -> Option<String> {
    match value.get("result") {
        Some(Value::Object(map)) => map
            .get("result")
            .and_then(Value::as_str)
            .map(str::to_string),
        Some(Value::String(text)) => Some(text.clone()),
        _ => None,
    }
}

/// Whether a status string is terminal.
pub(crate) fn is_terminal(status: &str) -> bool {
    TERMINAL_STATUSES.contains(&status)
}

/// Poll `fetch` until it reports a terminal status or `timeout` elapses.
///
/// Always polls at least once before checking the deadline, so a run that is
/// already terminal returns immediately. Injectable `fetch` (and thus testable
/// without a live daemon).
pub(crate) fn await_terminal<F>(
    timeout: Duration,
    interval: Duration,
    run_id: &str,
    mut fetch: F,
) -> Result<WorkerTerminal, WorkerError>
where
    F: FnMut() -> Result<WorkerRunStatus, WorkerError>,
{
    let start = Instant::now();
    loop {
        let current = fetch()?;
        if is_terminal(&current.status) {
            return Ok(WorkerTerminal {
                status: current.status,
                report_text: current.report_text,
            });
        }
        if start.elapsed() >= timeout {
            return Err(WorkerError::Timeout {
                run_id: run_id.to_string(),
                waited_secs: start.elapsed().as_secs(),
            });
        }
        std::thread::sleep(interval);
    }
}
