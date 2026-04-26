//! `orbit web` — the Orbit dashboard.
//!
//! `orbit web serve` boots a small axum server bound to loopback by default,
//! exposing the existing CLI JSON output via `/api/*` and the embedded SPA
//! shell at `/`.

mod api;

use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;

use axum::Router;
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use clap::{Args, Subcommand};
use orbit_core::{OrbitError, OrbitRuntime};

use crate::command::Execute;

const INDEX_HTML: &str = include_str!("../../../assets/dashboard/index.html");
const APP_JS: &str = include_str!("../../../assets/dashboard/app.js");

#[derive(Args)]
#[command(
    about = "Run the Orbit dashboard",
    arg_required_else_help = true,
    subcommand_required = true
)]
pub struct WebCommand {
    #[command(subcommand)]
    pub command: WebSubcommand,
}

impl Execute for WebCommand {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        self.command.execute(runtime)
    }
}

#[derive(Subcommand)]
pub enum WebSubcommand {
    /// Run the Orbit dashboard
    Serve(ServeArgs),
}

impl Execute for WebSubcommand {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        match self {
            WebSubcommand::Serve(args) => args.execute(runtime),
        }
    }
}

#[derive(Args)]
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

impl Execute for ServeArgs {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        let addr = SocketAddr::new(self.host, self.port);
        let url = format!("http://{addr}");
        let runtime = Arc::new(runtime.clone());

        let app = Router::new()
            .route("/", get(serve_index))
            .route("/static/app.js", get(serve_app_js))
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

            println!("Dashboard listening on {url}");

            if !self.no_open {
                open_browser(&url);
            }

            axum::serve(listener, app)
                .with_graceful_shutdown(shutdown_signal())
                .await
                .map_err(|e| OrbitError::Execution(format!("serve: {e}")))?;

            Ok::<(), OrbitError>(())
        })
    }
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
