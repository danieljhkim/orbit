//! `/healthz` — liveness plus opt-in readiness diagnostics [ORB-10005].
//!
//! Plain `GET /healthz` stays the cheap liveness probe it has always been:
//! an unconditional `200 ok` proving the process accepts connections.
//!
//! `GET /healthz?detailed=true` runs cheap, time-bounded checks and returns
//! per-check JSON: the store SQLite database accepts writes (write lock
//! acquired and rolled back) and the code-graph index is readable for every
//! workspace this server has open, plus the global JSONL log sink
//! (`~/.orbit/state/logs/orbit.jsonl`, ORB-00415) accepting appends. Overall
//! `200` when nothing failed, `503` otherwise — point uptime monitoring at
//! the detailed form. Checks probe only workspaces the server actually has
//! open (see `DashboardState::open_runtimes`); absent subsystems (no graph
//! index yet) report `skip`, never failure.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Json, Response};
use orbit_core::OrbitRuntime;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::log_format::resolve_log_path;
use crate::state::DashboardState;

/// Upper bound per individual check. Keeps the endpoint responsive even when
/// a database is wedged behind a hung writer.
const CHECK_TIMEOUT: Duration = Duration::from_secs(2);

#[derive(Deserialize, Default)]
pub(crate) struct HealthQuery {
    #[serde(default)]
    pub(crate) detailed: Option<bool>,
}

/// One check outcome. `status` is `ok`, `fail`, or `skip`.
struct CheckOutcome {
    name: &'static str,
    workspace: Option<String>,
    status: &'static str,
    detail: String,
}

impl CheckOutcome {
    fn to_json(&self) -> Value {
        let mut value = json!({
            "name": self.name,
            "status": self.status,
            "detail": self.detail,
        });
        if let Some(workspace) = &self.workspace
            && let Some(map) = value.as_object_mut()
        {
            map.insert("workspace".to_string(), json!(workspace));
        }
        value
    }
}

pub(crate) async fn healthz(
    State(state): State<DashboardState>,
    Query(query): Query<HealthQuery>,
) -> Response {
    if query.detailed != Some(true) {
        return (StatusCode::OK, "ok").into_response();
    }
    let log_path = resolve_log_path(None).map_err(|error| error.to_string());
    detailed_response(&state, log_path).await
}

/// Detailed health with an injectable log-sink path so tests stay hermetic
/// (the handler resolves `ORBIT_LOG_PATH` / `~/.orbit/state/logs/`).
pub(crate) async fn detailed_response(
    state: &DashboardState,
    log_path: Result<PathBuf, String>,
) -> Response {
    let mut checks: Vec<CheckOutcome> = Vec::new();

    for (workspace, runtime) in state.open_runtimes() {
        checks.push(store_writable_check(workspace.clone(), runtime.clone()).await);
        checks.push(graph_index_check(workspace, runtime).await);
    }
    checks.push(log_sink_check(log_path).await);

    let failed = checks.iter().any(|check| check.status == "fail");
    let status = if failed {
        StatusCode::SERVICE_UNAVAILABLE
    } else {
        StatusCode::OK
    };
    let body = json!({
        "status": if failed { "fail" } else { "ok" },
        "workspaces_open": state.open_runtimes().len(),
        "checks": checks.iter().map(CheckOutcome::to_json).collect::<Vec<_>>(),
    });
    (status, Json(body)).into_response()
}

/// Store SQLite accepts writes: `BEGIN IMMEDIATE` + rollback, no mutation.
async fn store_writable_check(workspace: String, runtime: Arc<OrbitRuntime>) -> CheckOutcome {
    let outcome = run_blocking_check(move || {
        runtime
            .health_check_store_writable()
            .map_err(|error| error.to_string())
    })
    .await;
    match outcome {
        Ok(detail) => CheckOutcome {
            name: "sqlite_writable",
            workspace: Some(workspace),
            status: "ok",
            detail,
        },
        Err(detail) => CheckOutcome {
            name: "sqlite_writable",
            workspace: Some(workspace),
            status: "fail",
            detail,
        },
    }
}

/// Code-graph index readable, when one has been built (skip otherwise).
async fn graph_index_check(workspace: String, runtime: Arc<OrbitRuntime>) -> CheckOutcome {
    let outcome = run_blocking_check(move || {
        Ok::<_, String>(
            runtime
                .health_check_graph_index()
                .map(|result| result.map_err(|error| error.to_string())),
        )
    })
    .await;
    match outcome {
        Ok(None) => CheckOutcome {
            name: "graph_index",
            workspace: Some(workspace),
            status: "skip",
            detail: "no graph index built".to_string(),
        },
        Ok(Some(Ok(detail))) => CheckOutcome {
            name: "graph_index",
            workspace: Some(workspace),
            status: "ok",
            detail,
        },
        Ok(Some(Err(detail))) | Err(detail) => CheckOutcome {
            name: "graph_index",
            workspace: Some(workspace),
            status: "fail",
            detail,
        },
    }
}

/// Global JSONL log sink accepts appends (ORB-00415).
async fn log_sink_check(log_path: Result<PathBuf, String>) -> CheckOutcome {
    let path = match log_path {
        Ok(path) => path,
        Err(detail) => {
            return CheckOutcome {
                name: "log_sink",
                workspace: None,
                status: "fail",
                detail,
            };
        }
    };
    let probe_path = path.clone();
    let outcome = run_blocking_check(move || probe_log_sink(&probe_path)).await;
    match outcome {
        Ok(()) => CheckOutcome {
            name: "log_sink",
            workspace: None,
            status: "ok",
            detail: format!("{} accepts appends", path.display()),
        },
        Err(detail) => CheckOutcome {
            name: "log_sink",
            workspace: None,
            status: "fail",
            detail: format!("{}: {detail}", path.display()),
        },
    }
}

/// Open the sink for append (creating the file like the tracing layer does)
/// without writing anything.
fn probe_log_sink(path: &Path) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    std::fs::OpenOptions::new()
        .append(true)
        .create(true)
        .open(path)
        .map(|_| ())
        .map_err(|error| error.to_string())
}

/// Run a blocking probe off the async worker with a hard time bound. Panics
/// and timeouts degrade to a failed check — never a crashed handler.
async fn run_blocking_check<T, F>(probe: F) -> Result<T, String>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T, String> + Send + 'static,
{
    let bounded = tokio::time::timeout(CHECK_TIMEOUT, tokio::task::spawn_blocking(probe));
    match bounded.await {
        Ok(Ok(result)) => result,
        Ok(Err(join_error)) => Err(format!("check panicked: {join_error}")),
        Err(_) => Err(format!(
            "check timed out after {}s",
            CHECK_TIMEOUT.as_secs()
        )),
    }
}
