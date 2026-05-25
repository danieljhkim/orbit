use std::collections::HashMap;
use std::env;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use orbit_common::types::{
    OrbitError, ToolParam, ToolSchema, ToolSessionContext, optional_string, optional_u32_alias,
    required_string,
};
use orbit_graph::{
    DEFAULT_IMPACT_DEPTH, DEFAULT_SHOW_MAX_BYTES, DEFAULT_TRACE_DEPTH, Graph, GraphError,
    RefConfidence, RefKind, RefOpts, SearchKind, SearchQuery, SyncMode, SyncPolicy,
};
use orbit_graph_extract::{Selector, SelectorParseError};
use serde_json::{Value, json};

const GRAPH_SYNC_TOOL: &str = "orbit.graph.sync";
const GRAPH_SEARCH_TOOL: &str = "orbit.graph.search";
const GRAPH_SHOW_TOOL: &str = "orbit.graph.show";
const GRAPH_REFS_TOOL: &str = "orbit.graph.refs";
const GRAPH_CALLEES_TOOL: &str = "orbit.graph.callees";
const GRAPH_IMPACT_TOOL: &str = "orbit.graph.impact";
const GRAPH_TRACE_TOOL: &str = "orbit.graph.trace";

const GRAPH_TOOL_NAMES: &[&str] = &[
    GRAPH_SYNC_TOOL,
    GRAPH_SEARCH_TOOL,
    GRAPH_SHOW_TOOL,
    GRAPH_REFS_TOOL,
    GRAPH_CALLEES_TOOL,
    GRAPH_IMPACT_TOOL,
    GRAPH_TRACE_TOOL,
];

const GRAPH_SYNC_WINDOW: Duration = Duration::from_millis(500);

pub(super) struct GraphToolRegistry {
    graphs: Mutex<HashMap<PathBuf, Arc<Graph>>>,
}

impl GraphToolRegistry {
    pub(super) fn new() -> Self {
        Self {
            graphs: Mutex::new(HashMap::new()),
        }
    }

    pub(super) fn schemas(&self) -> Vec<ToolSchema> {
        graph_tool_schemas()
    }

    pub(super) fn is_graph_tool(&self, name: &str) -> bool {
        GRAPH_TOOL_NAMES.contains(&name)
    }

    pub(super) fn call_tool(
        &self,
        name: &str,
        input: Value,
        session_context: ToolSessionContext,
    ) -> Result<Value, OrbitError> {
        let worktree = resolve_worktree(&input, &session_context)?;
        let graph = self.graph_for_worktree(worktree.as_path())?;
        match name {
            GRAPH_SYNC_TOOL => graph_sync(graph.as_ref(), &input),
            GRAPH_SEARCH_TOOL => graph_search(graph.as_ref(), &input),
            GRAPH_SHOW_TOOL => graph_show(graph.as_ref(), &input),
            GRAPH_REFS_TOOL => graph_refs(graph.as_ref(), &input),
            GRAPH_CALLEES_TOOL => graph_callees(graph.as_ref(), &input),
            GRAPH_IMPACT_TOOL => graph_impact(graph.as_ref(), &input),
            GRAPH_TRACE_TOOL => graph_trace(graph.as_ref(), &input),
            _ => Err(OrbitError::not_found(
                orbit_common::types::NotFoundKind::Tool,
                name.to_string(),
            )),
        }
    }

    fn graph_for_worktree(&self, worktree: &Path) -> Result<Arc<Graph>, OrbitError> {
        let mut graphs = self
            .graphs
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(graph) = graphs.get(worktree).cloned() {
            return Ok(graph);
        }

        let graph = Arc::new(
            Graph::open(
                worktree,
                SyncPolicy::Windowed {
                    window: GRAPH_SYNC_WINDOW,
                },
            )
            .map_err(graph_error_to_orbit)?,
        );
        tracing::debug!(
            target: "orbit.mcp.graph",
            worktree = %worktree.display(),
            "opened orbit graph handle"
        );
        graphs.insert(worktree.to_path_buf(), Arc::clone(&graph));
        Ok(graph)
    }

    #[cfg(test)]
    pub(super) fn cached_worktree_count(&self) -> usize {
        self.graphs
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .len()
    }
}

pub(super) fn graph_tool_schemas() -> Vec<ToolSchema> {
    vec![
        schema(
            GRAPH_SYNC_TOOL,
            "Synchronize the orbit-graph index for the current worktree.",
            vec![param(
                "full",
                "Run a full sync instead of an incremental auto sync.",
                "boolean",
                false,
            )],
        ),
        schema(
            GRAPH_SEARCH_TOOL,
            "Search orbit-graph symbols, notable strings, and config keys.",
            vec![
                param("query", "Search query text.", "string", true),
                param(
                    "kind",
                    "Optional result kind: symbol, string, or config.",
                    "string",
                    false,
                ),
                param("lang", "Optional language filter.", "string", false),
                param("limit", "Maximum number of matches.", "number", false),
            ],
        ),
        schema(
            GRAPH_SHOW_TOOL,
            "Show source and metadata for an orbit-graph selector. UTF-8 source is returned as `text`; non-UTF-8 source omits `text` and returns fallback `bytes`.",
            vec![
                param("selector", "Selector to show.", "string", true),
                param(
                    "max_bytes",
                    "Maximum source bytes returned in `text` or fallback `bytes`.",
                    "number",
                    false,
                ),
            ],
        ),
        schema(
            GRAPH_REFS_TOOL,
            "Find inbound references and relations for an orbit-graph symbol selector.",
            vec![
                param("symbol", "Symbol selector to query.", "string", true),
                param("confidence", "Minimum confidence floor.", "string", false),
                param(
                    "kind",
                    "Optional reference or relation kind filter.",
                    "string",
                    false,
                ),
            ],
        ),
        schema(
            GRAPH_CALLEES_TOOL,
            "Find outbound calls from an orbit-graph symbol selector.",
            vec![param("symbol", "Symbol selector to query.", "string", true)],
        ),
        schema(
            GRAPH_IMPACT_TOOL,
            "Return a bounded orbit-graph blast-radius traversal.",
            vec![
                param("selector", "Origin selector.", "string", true),
                param("depth", "Maximum traversal depth.", "number", false),
                param(
                    "confidence",
                    "Minimum confidence floor. Defaults to same_module.",
                    "string",
                    false,
                ),
            ],
        ),
        schema(
            GRAPH_TRACE_TOOL,
            "Trace a command handler call tree from orbit-graph command metadata.",
            vec![
                param("command_name", "Command name to trace.", "string", true),
                param("depth", "Maximum call-tree depth.", "number", false),
                param(
                    "confidence",
                    "Minimum confidence floor. Defaults to same_module.",
                    "string",
                    false,
                ),
            ],
        ),
    ]
}

fn schema(name: &str, description: &str, mut parameters: Vec<ToolParam>) -> ToolSchema {
    parameters.push(param(
        "workspace_path",
        "Optional worktree path for this graph request.",
        "string",
        false,
    ));
    parameters.push(param(
        "workspace",
        "Optional worktree path alias for this graph request.",
        "string",
        false,
    ));
    ToolSchema {
        name: name.to_string(),
        description: description.to_string(),
        parameters,
        builtin: true,
    }
}

fn param(name: &str, description: &str, param_type: &str, required: bool) -> ToolParam {
    ToolParam {
        name: name.to_string(),
        description: description.to_string(),
        param_type: param_type.to_string(),
        required,
    }
}

fn graph_sync(graph: &Graph, input: &Value) -> Result<Value, OrbitError> {
    let full = input.get("full").and_then(Value::as_bool).unwrap_or(false);
    let report = graph
        .sync(if full { SyncMode::Full } else { SyncMode::Auto })
        .map_err(graph_error_to_orbit)?;
    Ok(json!({
        "files_indexed": report.files_indexed,
        "files_changed": report.files_changed,
        "files_removed": report.files_removed,
        "duration_ms": report.duration.as_millis(),
    }))
}

fn graph_search(graph: &Graph, input: &Value) -> Result<Value, OrbitError> {
    let kind = optional_string(input, "kind")?
        .map(|value| parse_search_kind(value.as_str()))
        .transpose()?;
    let query = SearchQuery {
        query: required_string(input, &["query"], "query")?,
        kind,
        lang: optional_string(input, "lang")?,
        limit: optional_usize(input, "limit")?,
    };
    to_json(graph.search(&query).map_err(graph_error_to_orbit)?)
}

fn graph_show(graph: &Graph, input: &Value) -> Result<Value, OrbitError> {
    let selector = parse_selector(required_string(input, &["selector"], "selector")?)?;
    let max_bytes = optional_usize(input, "max_bytes")?.unwrap_or(DEFAULT_SHOW_MAX_BYTES);
    to_json(
        graph
            .show(&selector, max_bytes)
            .map_err(graph_error_to_orbit)?,
    )
}

fn graph_refs(graph: &Graph, input: &Value) -> Result<Value, OrbitError> {
    let selector = parse_selector(required_string(input, &["symbol", "selector"], "symbol")?)?;
    let opts = RefOpts {
        confidence: optional_string(input, "confidence")?
            .map(|value| parse_confidence(value.as_str()))
            .transpose()?
            .unwrap_or(RefConfidence::SameModule),
        kind: optional_string(input, "kind")?
            .map(|value| parse_ref_kind(value.as_str()))
            .transpose()?,
    };
    to_json(graph.refs(&selector, &opts).map_err(graph_error_to_orbit)?)
}

fn graph_callees(graph: &Graph, input: &Value) -> Result<Value, OrbitError> {
    let selector = parse_selector(required_string(input, &["symbol", "selector"], "symbol")?)?;
    Ok(json!({
        "callees": graph.callees(&selector).map_err(graph_error_to_orbit)?,
    }))
}

fn graph_impact(graph: &Graph, input: &Value) -> Result<Value, OrbitError> {
    let selector = parse_selector(required_string(input, &["selector"], "selector")?)?;
    let depth = optional_u8(input, "depth")?.unwrap_or(DEFAULT_IMPACT_DEPTH);
    let min_confidence = optional_confidence(input)?;
    to_json(
        graph
            .impact(&selector, depth, min_confidence)
            .map_err(graph_error_to_orbit)?,
    )
}

fn graph_trace(graph: &Graph, input: &Value) -> Result<Value, OrbitError> {
    let command_name = required_string(input, &["command_name", "command"], "command_name")?;
    let depth = optional_u8(input, "depth")?.unwrap_or(DEFAULT_TRACE_DEPTH);
    let min_confidence = optional_confidence(input)?;
    to_json(
        graph
            .trace(command_name.as_str(), depth, min_confidence)
            .map_err(graph_error_to_orbit)?,
    )
}

fn optional_confidence(input: &Value) -> Result<RefConfidence, OrbitError> {
    optional_string(input, "confidence")?
        .map(|value| parse_confidence(value.as_str()))
        .transpose()
        .map(|confidence| confidence.unwrap_or_default())
}

fn parse_selector(raw: String) -> Result<Selector, OrbitError> {
    raw.parse::<Selector>().map_err(selector_error_to_orbit)
}

fn parse_search_kind(raw: &str) -> Result<SearchKind, OrbitError> {
    SearchKind::parse(raw).ok_or_else(|| {
        OrbitError::InvalidInput("`kind` must be one of symbol, string, config".to_string())
    })
}

fn parse_confidence(raw: &str) -> Result<RefConfidence, OrbitError> {
    match raw {
        "exact" => Ok(RefConfidence::Exact),
        "import" | "import_resolved" => Ok(RefConfidence::ImportResolved),
        "same_module" => Ok(RefConfidence::SameModule),
        "fuzzy" | "fuzzy_name" => Ok(RefConfidence::FuzzyName),
        _ => Err(OrbitError::InvalidInput(
            "`confidence` must be one of exact, import, same_module, fuzzy".to_string(),
        )),
    }
}

fn parse_ref_kind(raw: &str) -> Result<RefKind, OrbitError> {
    match raw {
        "call" => Ok(RefKind::Call),
        "type" => Ok(RefKind::Type),
        "use" => Ok(RefKind::Use),
        "trait_bound" => Ok(RefKind::TraitBound),
        "impl" => Ok(RefKind::Impl),
        "extends" => Ok(RefKind::Extends),
        "implements" => Ok(RefKind::Implements),
        _ => Err(OrbitError::InvalidInput(
            "`kind` must be one of call, type, use, trait_bound, impl, extends, implements"
                .to_string(),
        )),
    }
}

fn optional_usize(input: &Value, key: &str) -> Result<Option<usize>, OrbitError> {
    optional_u32_alias(input, &[key])?
        .map(usize::try_from)
        .transpose()
        .map_err(|error| OrbitError::InvalidInput(format!("`{key}` is too large: {error}")))
}

fn optional_u8(input: &Value, key: &str) -> Result<Option<u8>, OrbitError> {
    optional_u32_alias(input, &[key])?
        .map(u8::try_from)
        .transpose()
        .map_err(|error| OrbitError::InvalidInput(format!("`{key}` must fit in u8: {error}")))
}

fn resolve_worktree(
    input: &Value,
    session_context: &ToolSessionContext,
) -> Result<PathBuf, OrbitError> {
    let requested = match optional_string(input, "workspace_path")? {
        Some(path) => Some(path),
        None => optional_string(input, "workspace")?,
    };
    let raw_path = requested
        .as_deref()
        .or(session_context.workspace.as_deref());
    let candidate = match raw_path {
        Some(raw) => absolutize(raw)?,
        None => env::current_dir().map_err(OrbitError::from)?,
    };
    let canonical = canonicalize_dir(candidate.as_path(), "worktree")?;

    if let (Some(session_workspace), Some(_)) = (session_context.workspace.as_deref(), requested) {
        let session_root = canonicalize_dir(absolutize(session_workspace)?.as_path(), "workspace")?;
        if !canonical.starts_with(session_root.as_path()) {
            return Err(OrbitError::InvalidInput(format!(
                "`workspace_path` must stay within initialized workspace `{}`",
                session_root.display()
            )));
        }
    }

    Ok(canonical)
}

fn absolutize(raw: &str) -> Result<PathBuf, OrbitError> {
    let path = Path::new(raw);
    if path.is_absolute() {
        Ok(path.to_path_buf())
    } else {
        Ok(env::current_dir().map_err(OrbitError::from)?.join(path))
    }
}

fn canonicalize_dir(path: &Path, label: &str) -> Result<PathBuf, OrbitError> {
    let canonical = path.canonicalize().map_err(|error| {
        OrbitError::InvalidInput(format!(
            "`{label}` must be an existing directory ({}): {error}",
            path.display()
        ))
    })?;
    if !canonical.is_dir() {
        return Err(OrbitError::InvalidInput(format!(
            "`{label}` must be a directory: {}",
            canonical.display()
        )));
    }
    Ok(canonical)
}

fn to_json<T: serde::Serialize>(value: T) -> Result<Value, OrbitError> {
    serde_json::to_value(value)
        .map_err(|error| OrbitError::Execution(format!("serialize graph tool response: {error}")))
}

fn graph_error_to_orbit(error: GraphError) -> OrbitError {
    match error {
        GraphError::Io { .. } => OrbitError::Io(error.to_string()),
        GraphError::InvalidData { .. } => OrbitError::InvalidInput(error.to_string()),
        GraphError::Sqlite { .. } | GraphError::Unimplemented => {
            OrbitError::Execution(error.to_string())
        }
        _ => OrbitError::Execution(error.to_string()),
    }
}

fn selector_error_to_orbit(error: SelectorParseError) -> OrbitError {
    OrbitError::InvalidInput(format!("invalid selector: {error}"))
}
