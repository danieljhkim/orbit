use std::fmt::{Display, Formatter};
use std::str::FromStr;
use std::thread;

use serde::Serialize;
use serde_json::Value;

/// Environment variable used to select the graph query backend.
pub const ORBIT_GRAPH_BACKEND_ENV: &str = "ORBIT_GRAPH_BACKEND";

/// Runtime backend selector for graph query surfaces.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum GraphBackend {
    /// Route queries through the legacy `orbit-knowledge` surface.
    Legacy,
    /// Route queries through the new SQLite-backed `orbit-graph` surface.
    New,
    /// Return new-backend results while shadowing the same query against legacy.
    Both,
}

impl GraphBackend {
    /// Stable lowercase label used in CLI args, JSON payloads, and audit rows.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Legacy => "legacy",
            Self::New => "new",
            Self::Both => "both",
        }
    }

    /// Resolve the effective backend with CLI/tool input taking precedence
    /// over the environment, and `legacy` as the final default.
    pub fn resolve(override_backend: Option<Self>) -> Result<Self, GraphBackendParseError> {
        Self::resolve_from(
            override_backend,
            std::env::var(ORBIT_GRAPH_BACKEND_ENV).ok(),
        )
    }

    /// Testable resolver variant with an injected environment value.
    pub fn resolve_from(
        override_backend: Option<Self>,
        env_backend: Option<String>,
    ) -> Result<Self, GraphBackendParseError> {
        if let Some(backend) = override_backend {
            return Ok(backend);
        }
        let Some(raw) = env_backend else {
            return Ok(Self::Legacy);
        };
        let raw = raw.trim();
        if raw.is_empty() {
            return Ok(Self::Legacy);
        }
        raw.parse()
    }
}

impl Display for GraphBackend {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for GraphBackend {
    type Err = GraphBackendParseError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.trim() {
            "legacy" => Ok(Self::Legacy),
            "new" => Ok(Self::New),
            "both" => Ok(Self::Both),
            other => Err(GraphBackendParseError {
                value: other.to_string(),
            }),
        }
    }
}

/// Parse error for [`GraphBackend`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GraphBackendParseError {
    value: String,
}

impl Display for GraphBackendParseError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "invalid graph backend `{}`; expected legacy, new, or both",
            self.value
        )
    }
}

impl std::error::Error for GraphBackendParseError {}

/// Graph query name used for structured backend routing logs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GraphQueryKind {
    /// Synchronize the selected graph backend.
    Sync,
    /// Search symbols, strings, or configs.
    Search,
    /// Show one selector.
    Show,
    /// Find inbound references for a selector.
    Refs,
    /// Find outbound calls from a selector.
    Callees,
    /// Traverse the bounded blast radius around a selector.
    Impact,
    /// Trace command-handler calls.
    Trace,
}

impl GraphQueryKind {
    /// Stable query label used in structured logs.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Sync => "sync",
            Self::Search => "search",
            Self::Show => "show",
            Self::Refs => "refs",
            Self::Callees => "callees",
            Self::Impact => "impact",
            Self::Trace => "trace",
        }
    }
}

/// Route a graph query to the selected backend.
///
/// `both` returns the new backend result as primary, runs legacy as a shadow
/// query, and logs shadow failures or JSON differences without failing the
/// primary query.
pub fn route_query<E, N, L>(
    backend: GraphBackend,
    query: GraphQueryKind,
    run_new: N,
    run_legacy: L,
) -> Result<Value, E>
where
    E: Display + Send,
    N: FnOnce() -> Result<Value, E> + Send,
    L: FnOnce() -> Result<Value, E> + Send,
{
    match backend {
        GraphBackend::Legacy => run_legacy(),
        GraphBackend::New => run_new(),
        GraphBackend::Both => route_both(query, run_new, run_legacy),
    }
}

fn route_both<E, N, L>(query: GraphQueryKind, run_new: N, run_legacy: L) -> Result<Value, E>
where
    E: Display + Send,
    N: FnOnce() -> Result<Value, E> + Send,
    L: FnOnce() -> Result<Value, E> + Send,
{
    let (primary, shadow) = thread::scope(|scope| {
        let shadow = scope.spawn(run_legacy);
        let primary = run_new();
        let shadow = shadow.join();
        (primary, shadow)
    });

    match shadow {
        Ok(Ok(shadow_value)) => {
            if let Ok(primary_value) = primary.as_ref() {
                log_diff_if_needed(query, primary_value, &shadow_value);
            }
        }
        Ok(Err(error)) => {
            tracing::warn!(
                target: "orbit.graph.backend",
                query = query.as_str(),
                primary_backend = GraphBackend::New.as_str(),
                shadow_backend = GraphBackend::Legacy.as_str(),
                error = %error,
                "graph backend shadow query failed"
            );
        }
        Err(_) => {
            tracing::warn!(
                target: "orbit.graph.backend",
                query = query.as_str(),
                primary_backend = GraphBackend::New.as_str(),
                shadow_backend = GraphBackend::Legacy.as_str(),
                "graph backend shadow query panicked"
            );
        }
    }

    primary
}

fn log_diff_if_needed(query: GraphQueryKind, primary: &Value, shadow: &Value) {
    if primary == shadow {
        return;
    }

    let primary_json = serde_json::to_vec(primary).unwrap_or_default();
    let shadow_json = serde_json::to_vec(shadow).unwrap_or_default();
    let primary_hash = blake3::hash(primary_json.as_slice()).to_hex().to_string();
    let shadow_hash = blake3::hash(shadow_json.as_slice()).to_hex().to_string();
    tracing::warn!(
        target: "orbit.graph.backend",
        query = query.as_str(),
        primary_backend = GraphBackend::New.as_str(),
        shadow_backend = GraphBackend::Legacy.as_str(),
        primary_len = primary_json.len(),
        shadow_len = shadow_json.len(),
        primary_hash,
        shadow_hash,
        "graph backend shadow diff"
    );
}
