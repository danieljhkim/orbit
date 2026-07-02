//! `orbit-dashboard` — read-only web dashboard and JSON API server.
//!
//! This crate isolates the axum-based dashboard (HTML/JS assets + `/api/*`
//! handlers) from orbit-cli so that CLI changes do not force rebuilds of the
//! large dependency subtree (axum, etc). Behavior is identical to the prior
//! in-tree implementation.
//!
//! Public surface is deliberately tiny: `ServeArgs` (clap) plus two entry
//! points — `serve()` for a caller-supplied runtime and `serve_from_env()`,
//! which resolves the workspace(s) to serve from the environment (single
//! workspace, or every registered workspace in global mode). All routes,
//! content types, defaults, and graceful shutdown are preserved.

mod api;
mod connect;
mod log_format;
mod parse;
mod projections;
mod state;

#[cfg(test)]
mod tests;

pub use connect::{ConnectArgs, connect};

use std::net::{IpAddr, SocketAddr};
use std::path::Path;
use std::sync::Arc;

use axum::Router;
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use clap::Args;
use orbit_common::types::{WorkspaceRegistry, WorkspaceStatus};
use orbit_core::{ActorIdentity, OrbitError, OrbitRuntime, workspace_registry};

const INDEX_HTML: &str = include_str!("../assets/dashboard/index.html");
const DASHBOARD_CSS: &str = include_str!("../assets/dashboard/dashboard.css");
const MARKED_JS: &str = include_str!("../assets/dashboard/marked.umd.js");
const PURIFY_JS: &str = include_str!("../assets/dashboard/purify.min.js");
// L-0021: Keep embedded dashboard JS modules in sync with /static routes.
const APP_JS: &str = include_str!("../assets/dashboard/app.js");
const COMMON_JS: &str = include_str!("../assets/dashboard/common.js");
const MARKDOWN_JS: &str = include_str!("../assets/dashboard/markdown.js");
const TASKS_JS: &str = include_str!("../assets/dashboard/tasks.js");
const AUDIT_JS: &str = include_str!("../assets/dashboard/audit.js");
const SCOREBOARD_JS: &str = include_str!("../assets/dashboard/scoreboard.js");
const LOG_TAIL_JS: &str = include_str!("../assets/dashboard/log-tail.js");
const DIAGNOSTICS_JS: &str = include_str!("../assets/dashboard/diagnostics.js");
const ROUTER_JS: &str = include_str!("../assets/dashboard/router.js");
const RUNS_JS: &str = include_str!("../assets/dashboard/runs.js");
const RUN_DETAIL_JS: &str = include_str!("../assets/dashboard/run-detail.js");
const REVIEW_THREADS_JS: &str = include_str!("../assets/dashboard/review-threads.js");
const DASHBOARD_CSP: &str = concat!(
    "default-src 'self'; ",
    "script-src 'self'; ",
    "style-src 'self' 'unsafe-inline' https://fonts.googleapis.com; ",
    "font-src 'self' https://fonts.gstatic.com; ",
    "img-src 'self' data:; ",
    "connect-src 'self'; ",
    "object-src 'none'; ",
    "base-uri 'none'; ",
    "frame-ancestors 'none'"
);

/// Conventional loopback port for the dashboard. Shared by `web serve`'s
/// `--port` default and `web connect`'s local/remote port preference so the
/// two surfaces agree on one number.
pub(crate) const DEFAULT_DASHBOARD_PORT: u16 = 7878;

/// Arguments for `orbit web serve` (and the library entry point).
#[derive(Args, Clone)]
#[command(about = "Run the Orbit dashboard")]
pub struct ServeArgs {
    /// Host or IP to bind to. Defaults to loopback for safety.
    #[arg(long, default_value = "127.0.0.1")]
    pub host: IpAddr,

    /// Port to bind to.
    #[arg(long, default_value_t = DEFAULT_DASHBOARD_PORT)]
    pub port: u16,

    /// Do not attempt to open the dashboard URL in a browser on startup.
    #[arg(long)]
    pub no_open: bool,

    /// Serve every registered workspace (machine-wide view) instead of only
    /// the current one. Implied automatically when run outside any workspace.
    #[arg(long)]
    pub global: bool,
}

/// Boot the dashboard for a single, already-built runtime and block until
/// shutdown (ctrl-c or SIGTERM). Retained for callers that already hold an
/// `OrbitRuntime`; `orbit web serve` uses [`serve_from_env`] instead.
pub fn serve(runtime: &OrbitRuntime, args: ServeArgs) -> Result<(), OrbitError> {
    let state = state::DashboardState::single(Arc::new(runtime.clone()));
    run_server(&args, state)
}

/// Boot the dashboard, resolving which workspace(s) to serve from the current
/// environment, and block until shutdown.
///
/// Unlike [`serve`], this needs no pre-built runtime, so it works from any
/// directory — the entry point for `orbit web serve` (dispatched before the
/// CLI's eager workspace initialization, which would otherwise fail outside a
/// workspace). Inside a workspace without `--global` it preserves the original
/// single-workspace behavior; otherwise it serves every registered workspace.
pub fn serve_from_env(args: ServeArgs, root_override: Option<&Path>) -> Result<(), OrbitError> {
    let state = build_state(&args, root_override)?;
    run_server(&args, state)
}

/// Resolve dashboard state from the environment.
///
/// - Inside a workspace, no `--global`: single mode over that workspace.
/// - `--global`, or outside any workspace: global mode over every registered
///   workspace (stale-path entries are listed but marked inactive and never
///   built).
fn build_state(
    args: &ServeArgs,
    root_override: Option<&Path>,
) -> Result<state::DashboardState, OrbitError> {
    let cwd_runtime = OrbitRuntime::try_initialize_existing(root_override)?;

    if !args.global
        && let Some(runtime) = cwd_runtime
    {
        let runtime = runtime.with_actor(ActorIdentity::human("human"));
        return Ok(state::DashboardState::single(Arc::new(runtime)));
    }

    let global_root = workspace_registry::global_orbit_dir()?;
    let mut registry = workspace_registry::load_registry()?;
    workspace_registry::validate_workspaces(&mut registry);

    let default_workspace = std::env::current_dir()
        .ok()
        .and_then(|cwd| default_workspace_for_cwd(&registry, &cwd));

    let entries = registry
        .workspaces
        .iter()
        .map(|ws| state::WsEntry {
            id: ws.id.clone(),
            name: ws.name.clone(),
            repo_root: ws.root.clone(),
            orbit_dir: ws.orbit_dir.clone(),
            active: ws.status == WorkspaceStatus::Active,
        })
        .collect();

    Ok(state::DashboardState::global(
        global_root,
        entries,
        default_workspace,
    ))
}

/// Best-effort default when serving globally: the registered workspace whose
/// repo root is the longest prefix of `cwd`, if the server was launched inside
/// one. `None` means the frontend opens on the aggregate "all workspaces" view.
fn default_workspace_for_cwd(registry: &WorkspaceRegistry, cwd: &Path) -> Option<String> {
    registry
        .workspaces
        .iter()
        .filter(|ws| ws.status == WorkspaceStatus::Active && cwd.starts_with(&ws.root))
        .max_by_key(|ws| ws.root.as_os_str().len())
        .map(|ws| ws.id.clone())
}

/// Build the axum app and block on the tokio runtime until graceful shutdown.
fn run_server(args: &ServeArgs, state: state::DashboardState) -> Result<(), OrbitError> {
    check_bindable_host(args.host, args.port)?;

    let addr = SocketAddr::new(args.host, args.port);
    let url = format!("http://{addr}");
    let no_open = args.no_open;

    let app = Router::new()
        .route("/", get(serve_index))
        .route("/static/dashboard.css", get(serve_dashboard_css))
        .route("/static/marked.umd.js", get(serve_marked_js))
        .route("/static/purify.min.js", get(serve_purify_js))
        .route("/static/app.js", get(serve_app_js))
        .route("/static/common.js", get(serve_common_js))
        .route("/static/markdown.js", get(serve_markdown_js))
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
        .with_state(state);

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

        if !no_open {
            open_browser(&url);
        }

        axum::serve(listener, app)
            .with_graceful_shutdown(shutdown_signal())
            .await
            .map_err(|e| OrbitError::Execution(format!("serve: {e}")))?;

        Ok::<(), OrbitError>(())
    })
}

/// Reject binding the dashboard to anything other than a loopback address.
///
/// SECURITY (ORB-00360): the dashboard has no authentication of its own. The
/// only request-level check is [`api::require_localhost_origin`], a
/// browser-CSRF mitigation that inspects the client-supplied `Origin` header
/// and is trivially spoofable by any non-browser client (curl, a LAN script).
/// It is NOT an access-control boundary. Binding to a non-loopback address
/// would expose the full unauthenticated read/write API to the network, so we
/// refuse. For remote access, bind loopback and front the dashboard with an
/// authenticated tunnel/reverse proxy (e.g. `ssh -L`).
fn check_bindable_host(host: IpAddr, port: u16) -> Result<(), OrbitError> {
    if host.is_loopback() {
        return Ok(());
    }
    Err(OrbitError::InvalidInput(format!(
        "refusing to bind dashboard to non-loopback address {host}: the \
         dashboard is unauthenticated and the Origin check is not an \
         access-control boundary. Bind a loopback address (127.0.0.1 or ::1) \
         and use an authenticated tunnel/reverse proxy (e.g. \
         `ssh -L {port}:localhost:{port} <host>`) for remote access."
    )))
}

async fn serve_index() -> Response {
    dashboard_response("text/html; charset=utf-8", INDEX_HTML)
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

async fn serve_marked_js() -> Response {
    dashboard_response("application/javascript; charset=utf-8", MARKED_JS)
}

async fn serve_purify_js() -> Response {
    dashboard_response("application/javascript; charset=utf-8", PURIFY_JS)
}

async fn serve_app_js() -> Response {
    dashboard_response("application/javascript; charset=utf-8", APP_JS)
}

async fn serve_common_js() -> Response {
    dashboard_response("application/javascript; charset=utf-8", COMMON_JS)
}

async fn serve_markdown_js() -> Response {
    dashboard_response("application/javascript; charset=utf-8", MARKDOWN_JS)
}

async fn serve_tasks_js() -> Response {
    dashboard_response("application/javascript; charset=utf-8", TASKS_JS)
}

async fn serve_audit_js() -> Response {
    dashboard_response("application/javascript; charset=utf-8", AUDIT_JS)
}

async fn serve_scoreboard_js() -> Response {
    dashboard_response("application/javascript; charset=utf-8", SCOREBOARD_JS)
}

async fn serve_log_tail_js() -> Response {
    dashboard_response("application/javascript; charset=utf-8", LOG_TAIL_JS)
}

async fn serve_diagnostics_js() -> Response {
    dashboard_response("application/javascript; charset=utf-8", DIAGNOSTICS_JS)
}

async fn serve_router_js() -> Response {
    dashboard_response("application/javascript; charset=utf-8", ROUTER_JS)
}

async fn serve_runs_js() -> Response {
    dashboard_response("application/javascript; charset=utf-8", RUNS_JS)
}

async fn serve_run_detail_js() -> Response {
    dashboard_response("application/javascript; charset=utf-8", RUN_DETAIL_JS)
}

async fn serve_review_threads_js() -> Response {
    dashboard_response("application/javascript; charset=utf-8", REVIEW_THREADS_JS)
}

fn dashboard_response(content_type: &'static str, body: &'static str) -> Response {
    let mut response = (
        [(header::CONTENT_TYPE, HeaderValue::from_static(content_type))],
        body,
    )
        .into_response();
    response.headers_mut().insert(
        header::CONTENT_SECURITY_POLICY,
        HeaderValue::from_static(DASHBOARD_CSP),
    );
    response
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

pub(crate) fn open_browser(url: &str) {
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
