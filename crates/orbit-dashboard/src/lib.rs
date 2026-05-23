//! `orbit-dashboard` — read-only web dashboard and JSON API server.
//!
//! This crate isolates the axum-based dashboard (HTML/JS assets + `/api/*`
//! handlers) from orbit-cli so that CLI changes do not force rebuilds of the
//! large dependency subtree (axum, etc). Behavior is identical to the prior
//! in-tree implementation.
//!
//! Public surface is deliberately tiny: `ServeArgs` (clap) + `serve()` entry
//! point. All routes, content types, defaults, and graceful shutdown are
//! preserved.

mod api;
mod log_format;
mod parse;
mod projections;

#[cfg(test)]
mod tests;

use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;

use axum::Router;
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use clap::Args;
use orbit_core::{OrbitError, OrbitRuntime};

const INDEX_HTML: &str = include_str!("../assets/dashboard/index.html");
const DASHBOARD_CSS: &str = include_str!("../assets/dashboard/dashboard.css");
// L-0021: Keep embedded dashboard JS modules in sync with /static routes.
const APP_JS: &str = include_str!("../assets/dashboard/app.js");
const COMMON_JS: &str = include_str!("../assets/dashboard/common.js");
const TASKS_JS: &str = include_str!("../assets/dashboard/tasks.js");
const AUDIT_JS: &str = include_str!("../assets/dashboard/audit.js");
const SCOREBOARD_JS: &str = include_str!("../assets/dashboard/scoreboard.js");
const LOG_TAIL_JS: &str = include_str!("../assets/dashboard/log-tail.js");
const DIAGNOSTICS_JS: &str = include_str!("../assets/dashboard/diagnostics.js");
const ROUTER_JS: &str = include_str!("../assets/dashboard/router.js");
const RUNS_JS: &str = include_str!("../assets/dashboard/runs.js");
const RUN_DETAIL_JS: &str = include_str!("../assets/dashboard/run-detail.js");
const REVIEW_THREADS_JS: &str = include_str!("../assets/dashboard/review-threads.js");

/// Arguments for `orbit web serve` (and the library entry point).
#[derive(Args, Clone)]
#[command(about = "Run the Orbit dashboard")]
pub struct ServeArgs {
    /// Host or IP to bind to. Defaults to loopback for safety.
    #[arg(long, default_value = "127.0.0.1")]
    pub host: IpAddr,

    /// Port to bind to.
    #[arg(long, default_value_t = 7878)]
    pub port: u16,

    /// Do not attempt to open the dashboard URL in a browser on startup.
    #[arg(long)]
    pub no_open: bool,
}

/// Boot the dashboard server and block until shutdown (ctrl-c or SIGTERM).
///
/// This is the single public entry point. The caller (typically orbit-cli's
/// thin `web` subcommand) supplies the shared `OrbitRuntime` and the clap
/// args. All routes, response shapes, and side-effects (browser open, banner)
/// are unchanged from the prior CLI-embedded version.
pub fn serve(runtime: &OrbitRuntime, args: ServeArgs) -> Result<(), OrbitError> {
    let addr = SocketAddr::new(args.host, args.port);
    let url = format!("http://{addr}");
    let runtime = Arc::new(runtime.clone());

    let app = Router::new()
        .route("/", get(serve_index))
        .route("/static/dashboard.css", get(serve_dashboard_css))
        .route("/static/app.js", get(serve_app_js))
        .route("/static/common.js", get(serve_common_js))
        .route("/static/tasks.js", get(serve_tasks_js))
        .route("/static/audit.js", get(serve_audit_js))
        .route("/static/scoreboard.js", get(serve_scoreboard_js))
        .route("/static/log-tail.js", get(serve_log_tail_js))
        .route("/static/diagnostics.js", get(serve_diagnostics_js))
        .route("/static/router.js", get(serve_router_js))
        .route("/static/runs.js", get(serve_runs_js))
        .route("/static/run-detail.js", get(serve_run_detail_js))
        .route("/static/review-threads.js", get(serve_review_threads_js))
        .route("/healthz", get(healthz))
        .nest("/api", api::router())
        .with_state(runtime);

    let tokio_runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|e| OrbitError::Execution(format!("tokio runtime: {e}")))?;

    tokio_runtime.block_on(async move {
        let listener = tokio::net::TcpListener::bind(addr)
            .await
            .map_err(|e| OrbitError::Io(format!("bind {addr}: {e}")))?;

        #[allow(clippy::print_stdout)]
        {
            println!("Dashboard listening on {url}");
        }

        if !args.no_open {
            open_browser(&url);
        }

        axum::serve(listener, app)
            .with_graceful_shutdown(shutdown_signal())
            .await
            .map_err(|e| OrbitError::Execution(format!("serve: {e}")))?;

        Ok::<(), OrbitError>(())
    })
}

async fn serve_index() -> Response {
    (
        [(
            header::CONTENT_TYPE,
            HeaderValue::from_static("text/html; charset=utf-8"),
        )],
        INDEX_HTML,
    )
        .into_response()
}

async fn serve_dashboard_css() -> Response {
    (
        [(
            header::CONTENT_TYPE,
            HeaderValue::from_static("text/css; charset=utf-8"),
        )],
        DASHBOARD_CSS,
    )
        .into_response()
}

async fn serve_app_js() -> Response {
    (
        [(
            header::CONTENT_TYPE,
            HeaderValue::from_static("application/javascript; charset=utf-8"),
        )],
        APP_JS,
    )
        .into_response()
}

async fn serve_common_js() -> Response {
    (
        [(
            header::CONTENT_TYPE,
            HeaderValue::from_static("application/javascript; charset=utf-8"),
        )],
        COMMON_JS,
    )
        .into_response()
}

async fn serve_tasks_js() -> Response {
    (
        [(
            header::CONTENT_TYPE,
            HeaderValue::from_static("application/javascript; charset=utf-8"),
        )],
        TASKS_JS,
    )
        .into_response()
}

async fn serve_audit_js() -> Response {
    (
        [(
            header::CONTENT_TYPE,
            HeaderValue::from_static("application/javascript; charset=utf-8"),
        )],
        AUDIT_JS,
    )
        .into_response()
}

async fn serve_scoreboard_js() -> Response {
    (
        [(
            header::CONTENT_TYPE,
            HeaderValue::from_static("application/javascript; charset=utf-8"),
        )],
        SCOREBOARD_JS,
    )
        .into_response()
}

async fn serve_log_tail_js() -> Response {
    (
        [(
            header::CONTENT_TYPE,
            HeaderValue::from_static("application/javascript; charset=utf-8"),
        )],
        LOG_TAIL_JS,
    )
        .into_response()
}

async fn serve_diagnostics_js() -> Response {
    (
        [(
            header::CONTENT_TYPE,
            HeaderValue::from_static("application/javascript; charset=utf-8"),
        )],
        DIAGNOSTICS_JS,
    )
        .into_response()
}

async fn serve_router_js() -> Response {
    (
        [(
            header::CONTENT_TYPE,
            HeaderValue::from_static("application/javascript; charset=utf-8"),
        )],
        ROUTER_JS,
    )
        .into_response()
}

async fn serve_runs_js() -> Response {
    (
        [(
            header::CONTENT_TYPE,
            HeaderValue::from_static("application/javascript; charset=utf-8"),
        )],
        RUNS_JS,
    )
        .into_response()
}

async fn serve_run_detail_js() -> Response {
    (
        [(
            header::CONTENT_TYPE,
            HeaderValue::from_static("application/javascript; charset=utf-8"),
        )],
        RUN_DETAIL_JS,
    )
        .into_response()
}

async fn serve_review_threads_js() -> Response {
    (
        [(
            header::CONTENT_TYPE,
            HeaderValue::from_static("application/javascript; charset=utf-8"),
        )],
        REVIEW_THREADS_JS,
    )
        .into_response()
}

async fn healthz() -> (StatusCode, &'static str) {
    (StatusCode::OK, "ok")
}

async fn shutdown_signal() {
    let ctrl_c = async {
        let _ = tokio::signal::ctrl_c().await;
    };

    #[cfg(unix)]
    let terminate = async {
        if let Ok(mut sig) =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        {
            sig.recv().await;
        }
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {}
        _ = terminate => {}
    }
}

fn open_browser(url: &str) {
    #[cfg(target_os = "macos")]
    let cmd = "open";
    #[cfg(all(unix, not(target_os = "macos")))]
    let cmd = "xdg-open";
    #[cfg(windows)]
    let cmd = "explorer";

    let _ = std::process::Command::new(cmd)
        .arg(url)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn();
}
