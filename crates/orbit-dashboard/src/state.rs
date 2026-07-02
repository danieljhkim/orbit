//! Multi-workspace dashboard state (ORB-00030).
//!
//! The dashboard was originally coupled to a single `Arc<OrbitRuntime>` used
//! directly as axum state. To let one server serve every registered workspace
//! on the machine, state is generalized to a workspace-keyed, lazily-built
//! runtime map ([`DashboardState`]) and handlers receive their runtime through
//! the [`Ws`] extractor (which selects a workspace from the `?workspace=<id>`
//! query parameter, falling back to the configured default).
//!
//! [`DashboardState::single`] preserves the original single-workspace behavior
//! so `serve()` inside a workspace and every existing handler test keep working
//! unchanged: one pre-built runtime, always selected, no lazy construction.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use axum::extract::FromRequestParts;
use axum::http::StatusCode;
use axum::http::request::Parts;
use axum::response::{IntoResponse, Json, Response};
use orbit_core::{ActorIdentity, OrbitError, OrbitRuntime};
use serde_json::json;

/// Synthetic workspace id used by [`DashboardState::single`].
pub(crate) const SINGLE_WORKSPACE_ID: &str = "default";

/// One registered workspace the dashboard can serve.
///
/// `orbit_dir` is the workspace's `.orbit` directory — the value passed to
/// [`OrbitRuntime::from_roots`] as the workspace root. `active` mirrors the
/// registry status: inactive (stale-path) entries are listed but never built.
#[derive(Clone, Debug)]
pub(crate) struct WsEntry {
    pub(crate) id: String,
    pub(crate) name: String,
    pub(crate) repo_root: PathBuf,
    pub(crate) orbit_dir: PathBuf,
    pub(crate) active: bool,
}

struct StateInner {
    /// Global orbit root (`~/.orbit`); passed as `global_root` when building
    /// per-workspace runtimes. Unused in single mode.
    global_root: PathBuf,
    entries: Vec<WsEntry>,
    default_workspace: Option<String>,
    /// Lazily-built, cached runtimes keyed by workspace id.
    runtimes: Mutex<HashMap<String, Arc<OrbitRuntime>>>,
}

/// Axum application state: the set of servable workspaces plus a lazy runtime
/// cache. Cheap to clone (single `Arc`).
#[derive(Clone)]
pub(crate) struct DashboardState {
    inner: Arc<StateInner>,
}

impl DashboardState {
    /// Single-workspace mode: serve exactly one pre-built runtime, always
    /// selected. Preserves the pre-ORB-00030 behavior and keeps every handler
    /// test (which builds an in-memory runtime) working unchanged.
    pub(crate) fn single(runtime: Arc<OrbitRuntime>) -> Self {
        let entry = WsEntry {
            id: SINGLE_WORKSPACE_ID.to_string(),
            name: SINGLE_WORKSPACE_ID.to_string(),
            repo_root: PathBuf::new(),
            orbit_dir: PathBuf::new(),
            active: true,
        };
        let mut runtimes = HashMap::new();
        runtimes.insert(SINGLE_WORKSPACE_ID.to_string(), runtime);
        Self {
            inner: Arc::new(StateInner {
                global_root: PathBuf::new(),
                entries: vec![entry],
                default_workspace: Some(SINGLE_WORKSPACE_ID.to_string()),
                runtimes: Mutex::new(runtimes),
            }),
        }
    }

    /// Global mode: serve every registered workspace, building runtimes on
    /// first access. `default_workspace` (if any) is the workspace selected
    /// when a request omits `?workspace=`.
    pub(crate) fn global(
        global_root: PathBuf,
        entries: Vec<WsEntry>,
        default_workspace: Option<String>,
    ) -> Self {
        Self {
            inner: Arc::new(StateInner {
                global_root,
                entries,
                default_workspace,
                runtimes: Mutex::new(HashMap::new()),
            }),
        }
    }

    pub(crate) fn entries(&self) -> &[WsEntry] {
        &self.inner.entries
    }

    pub(crate) fn default_workspace(&self) -> Option<&str> {
        self.inner.default_workspace.as_deref()
    }

    /// Resolve (and lazily build + cache) the runtime for workspace `id`.
    ///
    /// Building happens outside the cache lock; a concurrent build for the same
    /// id is harmless (idempotent) and the first cached value wins.
    pub(crate) fn runtime_for(&self, id: &str) -> Result<Arc<OrbitRuntime>, WsRejection> {
        let entry = self
            .inner
            .entries
            .iter()
            .find(|e| e.id == id)
            .ok_or_else(|| WsRejection::unknown(id))?;
        if !entry.active {
            return Err(WsRejection::inactive(id));
        }

        // Fast path: already cached.
        if let Some(rt) = self.lock_runtimes().get(id).cloned() {
            return Ok(rt);
        }

        // Build outside the lock (no lock held across construction).
        let runtime = OrbitRuntime::from_roots(&self.inner.global_root, &entry.orbit_dir)
            .map_err(|e| WsRejection::build_failed(id, &e))?
            .with_actor(ActorIdentity::human("human"));
        let runtime = Arc::new(runtime);

        let mut cache = self.lock_runtimes();
        let cached = cache
            .entry(id.to_string())
            .or_insert_with(|| runtime.clone());
        Ok(cached.clone())
    }

    fn lock_runtimes(&self) -> std::sync::MutexGuard<'_, HashMap<String, Arc<OrbitRuntime>>> {
        // Recover from poisoning: the cache is an idempotent build cache, so a
        // panic in another thread cannot leave it logically inconsistent.
        self.inner
            .runtimes
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

/// Rejection returned by the [`Ws`] extractor when a workspace cannot be
/// selected or built. Renders as a JSON `{ "error": ... }` body.
pub(crate) struct WsRejection {
    status: StatusCode,
    message: String,
}

impl WsRejection {
    fn unknown(id: &str) -> Self {
        Self {
            status: StatusCode::NOT_FOUND,
            message: format!("unknown workspace: {id}"),
        }
    }

    fn inactive(id: &str) -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            message: format!("workspace '{id}' is inactive (its path no longer exists)"),
        }
    }

    fn build_failed(id: &str, err: &OrbitError) -> Self {
        Self {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            message: format!("failed to open workspace '{id}': {err}"),
        }
    }

    fn no_default() -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            message: "no workspace selected and no default is configured; \
                      pass ?workspace=<id>"
                .to_string(),
        }
    }
}

impl IntoResponse for WsRejection {
    fn into_response(self) -> Response {
        (self.status, Json(json!({ "error": self.message }))).into_response()
    }
}

/// Extractor yielding the `Arc<OrbitRuntime>` for the request's workspace.
///
/// Selection order: the `?workspace=<id>` query parameter, else the state's
/// configured default. Handlers destructure it as `Ws(runtime)` — a drop-in
/// replacement for the former `State(runtime): State<Arc<OrbitRuntime>>`.
pub(crate) struct Ws(pub(crate) Arc<OrbitRuntime>);

#[axum::async_trait]
impl FromRequestParts<DashboardState> for Ws {
    type Rejection = WsRejection;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &DashboardState,
    ) -> Result<Self, Self::Rejection> {
        let requested = parts.uri.query().and_then(workspace_from_query);
        let id = match requested {
            Some(id) => id,
            None => state
                .default_workspace()
                .map(str::to_string)
                .ok_or_else(WsRejection::no_default)?,
        };
        Ok(Ws(state.runtime_for(&id)?))
    }
}

/// Extract the `workspace` value from a raw query string (percent-decoded),
/// ignoring empty values so `?workspace=` behaves like an omitted parameter.
fn workspace_from_query(query: &str) -> Option<String> {
    url::form_urlencoded::parse(query.as_bytes())
        .find(|(k, _)| k == "workspace")
        .map(|(_, v)| v.into_owned())
        .filter(|v| !v.is_empty())
}
