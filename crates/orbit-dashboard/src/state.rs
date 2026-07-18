//! Multi-workspace dashboard state (ORB-00030).
//!
//! The dashboard was originally coupled to a single `Arc<OrbitRuntime>` used
//! directly as axum state. To let one server serve every registered workspace
//! on the machine, state is generalized to a workspace-keyed, lazily-built
//! runtime map ([`DashboardState`]) and handlers receive their runtime through
//! the [`Ws`] extractor (which selects a workspace from the `?workspace=<id>`
//! query parameter, falling back to the configured default).
//!
//! [`DashboardState::single`] preserves the original single-workspace behavior:
//! one pre-built runtime, always selected, no lazy construction. `orbit web
//! serve` no longer reaches it (it always serves in global mode as of
//! ORB-10029); it is retained for [`crate::serve`] (callers embedding an
//! already-built `OrbitRuntime`) and for every existing handler test, which
//! builds an in-memory runtime and wants a trivial single-workspace harness.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};

use axum::extract::FromRequestParts;
use axum::http::StatusCode;
use axum::http::request::Parts;
use axum::response::{IntoResponse, Json, Response};
use orbit_common::types::WorkspaceStatus;
use orbit_core::{ActorIdentity, OrbitError, OrbitRuntime, workspace_registry};
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

/// Immutable, atomically-swapped view of the registered workspace set plus the
/// dropdown's default selection. A refresh ([`DashboardState::refresh`])
/// replaces the whole `Arc<Snapshot>` in one step, so a reader either sees the
/// old view or the new one — never a half-applied update.
pub(crate) struct Snapshot {
    entries: Vec<WsEntry>,
    default_workspace: Option<String>,
}

/// Where a refresh reloads the servable workspace set from. Present only in the
/// registry-backed mode built by [`crate::build_state`]; [`DashboardState::single`]
/// and [`DashboardState::global`] leave it `None`, making [`DashboardState::refresh`]
/// a no-op (their entries are supplied directly and never re-read).
pub(crate) struct RegistrySource {
    /// Path to `~/.orbit/workspaces.json` (or a test double).
    registry_path: PathBuf,
    /// The top-level `--root <path>` flag, if any, for default re-selection.
    root_override: Option<PathBuf>,
    /// Process cwd captured at startup, for default re-selection.
    cwd: Option<PathBuf>,
}

impl RegistrySource {
    pub(crate) fn new(
        registry_path: PathBuf,
        root_override: Option<PathBuf>,
        cwd: Option<PathBuf>,
    ) -> Self {
        Self {
            registry_path,
            root_override,
            cwd,
        }
    }

    /// Reload the authoritative registry into a fresh [`Snapshot`]. Stale-path
    /// workspaces are marked inactive (never deleted) via `validate_workspaces`.
    fn load(&self) -> Result<Snapshot, OrbitError> {
        let mut registry = workspace_registry::load_registry_from(&self.registry_path)?;
        workspace_registry::validate_workspaces(&mut registry);
        let default_workspace = crate::default_workspace_selection(
            &registry,
            self.root_override.as_deref(),
            self.cwd.as_deref(),
        );
        let entries = workspace_registry::local_workspaces(&registry)
            .map(|(workspace, checkout)| WsEntry {
                id: workspace.id.clone(),
                name: workspace.name.clone(),
                repo_root: checkout.repo_root.clone(),
                orbit_dir: checkout.orbit_dir.clone(),
                active: workspace.status == WorkspaceStatus::Active,
            })
            .collect();
        Ok(Snapshot {
            entries,
            default_workspace,
        })
    }
}

/// A built runtime plus the binding it was constructed from. The binding lets a
/// refresh evict a runtime whose workspace was rebound (root/orbit-dir changed)
/// and lets `runtime_for` detect a stale cache entry and rebuild.
struct CachedRuntime {
    repo_root: PathBuf,
    orbit_dir: PathBuf,
    runtime: Arc<OrbitRuntime>,
}

struct StateInner {
    /// Global orbit root (`~/.orbit`); passed as `global_root` when building
    /// per-workspace runtimes. Unused in single mode.
    global_root: PathBuf,
    /// Atomically-swapped registered workspace set + default selection.
    snapshot: Mutex<Arc<Snapshot>>,
    /// Lazily-built, cached runtimes keyed by workspace id.
    runtimes: Mutex<HashMap<String, CachedRuntime>>,
    /// Registry to reload from on refresh; `None` disables refresh (single /
    /// direct-entry global modes).
    source: Option<RegistrySource>,
    /// Serializes refreshes so a snapshot swap and its runtime eviction are one
    /// atomic step relative to other refreshes. Never held across runtime
    /// construction.
    refresh_lock: Mutex<()>,
}

/// Axum application state: the set of servable workspaces plus a lazy runtime
/// cache. Cheap to clone (single `Arc`).
#[derive(Clone)]
pub(crate) struct DashboardState {
    inner: Arc<StateInner>,
}

impl DashboardState {
    /// Single-workspace mode: serve exactly one pre-built runtime, always
    /// selected. No longer reachable from `orbit web serve` (ORB-10029);
    /// used by [`crate::serve`] and by every handler test (which builds an
    /// in-memory runtime and wants a trivial single-workspace harness).
    pub(crate) fn single(runtime: Arc<OrbitRuntime>) -> Self {
        let entry = WsEntry {
            id: SINGLE_WORKSPACE_ID.to_string(),
            name: SINGLE_WORKSPACE_ID.to_string(),
            repo_root: PathBuf::new(),
            orbit_dir: PathBuf::new(),
            active: true,
        };
        let mut runtimes = HashMap::new();
        runtimes.insert(
            SINGLE_WORKSPACE_ID.to_string(),
            CachedRuntime {
                repo_root: PathBuf::new(),
                orbit_dir: PathBuf::new(),
                runtime,
            },
        );
        Self::from_parts(
            PathBuf::new(),
            Snapshot {
                entries: vec![entry],
                default_workspace: Some(SINGLE_WORKSPACE_ID.to_string()),
            },
            runtimes,
            None,
        )
    }

    /// Global mode with an explicitly-supplied entry set (no registry reload).
    /// Used by handler tests; `default_workspace` (if any) is the workspace
    /// selected when a request omits `?workspace=`. [`DashboardState::refresh`]
    /// is a no-op here — the entries are fixed at construction. Production
    /// serving uses [`DashboardState::from_registry`] instead.
    #[cfg(test)]
    pub(crate) fn global(
        global_root: PathBuf,
        entries: Vec<WsEntry>,
        default_workspace: Option<String>,
    ) -> Self {
        Self::from_parts(
            global_root,
            Snapshot {
                entries,
                default_workspace,
            },
            HashMap::new(),
            None,
        )
    }

    /// Registry-backed global mode: the servable workspace set is (re)loaded
    /// from `source` on every [`DashboardState::refresh`], so native `orbit
    /// workspace init/remove` and binding changes become visible without a
    /// restart. The initial load is eager — a malformed registry at startup is
    /// fatal (matching the pre-refresh behavior), whereas a later malformed
    /// refresh retains the last valid snapshot.
    pub(crate) fn from_registry(
        global_root: PathBuf,
        source: RegistrySource,
    ) -> Result<Self, OrbitError> {
        let snapshot = source.load()?;
        Ok(Self::from_parts(
            global_root,
            snapshot,
            HashMap::new(),
            Some(source),
        ))
    }

    fn from_parts(
        global_root: PathBuf,
        snapshot: Snapshot,
        runtimes: HashMap<String, CachedRuntime>,
        source: Option<RegistrySource>,
    ) -> Self {
        Self {
            inner: Arc::new(StateInner {
                global_root,
                snapshot: Mutex::new(Arc::new(snapshot)),
                runtimes: Mutex::new(runtimes),
                source,
                refresh_lock: Mutex::new(()),
            }),
        }
    }

    /// The currently-servable workspace entries (a cheap clone of the live
    /// snapshot). Call [`DashboardState::refresh`] first at a request boundary
    /// to reflect on-disk registry mutations.
    pub(crate) fn entries(&self) -> Vec<WsEntry> {
        self.snapshot().entries.clone()
    }

    /// Global orbit root (`~/.orbit`) this server was launched against. Empty
    /// in single mode ([`DashboardState::single`]). Host-level views (routine
    /// scheduler health) read from here rather than any one workspace runtime,
    /// because routine fires live in the global store.
    pub(crate) fn global_root(&self) -> &std::path::Path {
        &self.inner.global_root
    }

    pub(crate) fn default_workspace(&self) -> Option<String> {
        self.snapshot().default_workspace.clone()
    }

    /// Reload the registered workspace set from the authoritative registry and
    /// reconcile the runtime cache. A no-op unless this state was built via
    /// [`DashboardState::from_registry`].
    ///
    /// Guarantees:
    /// - **Atomic swap.** The new snapshot replaces the old one in a single
    ///   assignment under the snapshot lock; readers never observe a partial
    ///   update.
    /// - **Keep-last-valid.** A malformed or unreadable registry leaves the
    ///   current snapshot untouched and emits a credential-safe diagnostic.
    /// - **No build under lock.** Eviction only drops cache entries; runtimes
    ///   are (re)built lazily in `runtime_for`, never here and never while a
    ///   registry/cache lock is held.
    pub(crate) fn refresh(&self) {
        let Some(source) = self.inner.source.as_ref() else {
            return;
        };
        // Serialize concurrent refreshes so the swap + eviction below is one
        // atomic step. Held across the registry read but never across runtime
        // construction (which only happens in `runtime_for`, off this lock).
        let _serialize = self
            .inner
            .refresh_lock
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let snapshot = match source.load() {
            Ok(snapshot) => snapshot,
            Err(error) => {
                // A malformed or partially-written registry must never replace
                // a good in-memory snapshot. The diagnostic names the registry
                // path and Orbit's own error message; it deliberately never
                // echoes the file's contents, so a tokenized `git_remote` in
                // the registry cannot leak into logs.
                tracing::warn!(
                    registry = %source.registry_path.display(),
                    error = %error,
                    "workspace registry refresh failed; retaining last valid workspace set"
                );
                return;
            }
        };
        // Bindings still servable after the swap; used to evict runtimes whose
        // workspace was removed, went inactive, or was rebound.
        let live: Vec<(String, PathBuf, PathBuf)> = snapshot
            .entries
            .iter()
            .filter(|entry| entry.active)
            .map(|entry| {
                (
                    entry.id.clone(),
                    entry.repo_root.clone(),
                    entry.orbit_dir.clone(),
                )
            })
            .collect();
        {
            let mut guard = self.lock_snapshot();
            *guard = Arc::new(snapshot);
        }
        let mut cache = self.lock_runtimes();
        cache.retain(|id, cached| {
            live.iter().any(|(live_id, repo_root, orbit_dir)| {
                live_id == id && *repo_root == cached.repo_root && *orbit_dir == cached.orbit_dir
            })
        });
    }

    /// Resolve (and lazily build + cache) the runtime for workspace `id`.
    ///
    /// Building happens outside the cache lock; a concurrent build for the same
    /// binding is harmless (idempotent) and the first cached value wins. A
    /// cache entry whose binding no longer matches the live snapshot (the
    /// workspace was rebound) is rebuilt and replaced.
    pub(crate) fn runtime_for(&self, id: &str) -> Result<Arc<OrbitRuntime>, WsRejection> {
        // Resolve the binding from the live snapshot, then release the snapshot
        // lock before touching the runtime cache or building a runtime.
        let (repo_root, orbit_dir) = {
            let snapshot = self.lock_snapshot();
            let entry = snapshot
                .entries
                .iter()
                .find(|entry| entry.id == id)
                .ok_or_else(|| WsRejection::unknown(id))?;
            if !entry.active {
                return Err(WsRejection::inactive(id));
            }
            (entry.repo_root.clone(), entry.orbit_dir.clone())
        };

        // Fast path: a cached runtime whose binding still matches.
        if let Some(runtime) = self.cached_matching(id, &repo_root, &orbit_dir) {
            return Ok(runtime);
        }

        // Build outside the lock (no lock held across construction).
        let runtime = OrbitRuntime::from_roots(&self.inner.global_root, &orbit_dir)
            .map_err(|e| WsRejection::build_failed(id, &e))?
            .with_actor(ActorIdentity::human("human"));
        let runtime = Arc::new(runtime);

        let mut cache = self.lock_runtimes();
        // A concurrent build for the same binding wins; a stale binding is
        // replaced with the freshly-built runtime.
        if let Some(existing) = cache.get(id)
            && existing.repo_root == repo_root
            && existing.orbit_dir == orbit_dir
        {
            return Ok(existing.runtime.clone());
        }
        cache.insert(
            id.to_string(),
            CachedRuntime {
                repo_root,
                orbit_dir,
                runtime: runtime.clone(),
            },
        );
        Ok(runtime)
    }

    /// Return the cached runtime for `id` iff its binding matches `(repo_root,
    /// orbit_dir)`; a mismatch (rebound workspace) reports absent so the caller
    /// rebuilds.
    fn cached_matching(
        &self,
        id: &str,
        repo_root: &Path,
        orbit_dir: &Path,
    ) -> Option<Arc<OrbitRuntime>> {
        let cache = self.lock_runtimes();
        cache.get(id).and_then(|cached| {
            (cached.repo_root == repo_root && cached.orbit_dir == orbit_dir)
                .then(|| cached.runtime.clone())
        })
    }

    /// Snapshot of the runtimes this server currently has open (built and
    /// cached), in registry order. In single mode this is the one pre-built
    /// runtime; in global mode only workspaces that have actually been
    /// served appear — health checks probe what the process holds open
    /// rather than force-building every registered workspace.
    pub(crate) fn open_runtimes(&self) -> Vec<(String, Arc<OrbitRuntime>)> {
        let snapshot = self.snapshot();
        let cache = self.lock_runtimes();
        snapshot
            .entries
            .iter()
            .filter_map(|entry| {
                cache
                    .get(&entry.id)
                    .map(|cached| (entry.id.clone(), cached.runtime.clone()))
            })
            .collect()
    }

    /// Cheap clone of the live snapshot `Arc`, taken under (and released with)
    /// the snapshot lock so callers hold no lock while reading it.
    fn snapshot(&self) -> Arc<Snapshot> {
        self.lock_snapshot().clone()
    }

    fn lock_snapshot(&self) -> std::sync::MutexGuard<'_, Arc<Snapshot>> {
        self.inner
            .snapshot
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }

    fn lock_runtimes(&self) -> std::sync::MutexGuard<'_, HashMap<String, CachedRuntime>> {
        // Recover from poisoning: the cache is an idempotent build cache, so a
        // panic in another thread cannot leave it logically inconsistent.
        self.inner
            .runtimes
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }
}

/// Rejection returned by the [`Ws`] extractor when a workspace cannot be
/// selected or built. Renders as a JSON `{ "error": ... }` body.
#[derive(Debug)]
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
        // Reconcile with the on-disk registry so a native add/remove/rebind
        // since the last request is honored before we resolve and route.
        state.refresh();
        let requested = parts.uri.query().and_then(workspace_from_query);
        let id = match requested {
            Some(id) => id,
            None => state
                .default_workspace()
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
