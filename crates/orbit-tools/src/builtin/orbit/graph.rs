use std::path::{Path, PathBuf};
use std::time::Duration;

use orbit_common::types::{OrbitError, ToolParam, ToolSchema};
use orbit_graph::{
    DEFAULT_IMPACT_DEPTH, DEFAULT_SHOW_MAX_BYTES, DEFAULT_TRACE_DEPTH, Graph, GraphError,
    RefConfidence, RefKind, RefOpts, SearchKind, SearchQuery, SyncMode, SyncPolicy,
};
use orbit_graph_extract::{Selector, SelectorParseError};
use serde::Serialize;
use serde_json::{Value, json};

use crate::{Tool, ToolContext};

const GRAPH_SYNC_WINDOW: Duration = Duration::from_millis(500);

pub struct OrbitGraphSyncTool;
pub struct OrbitGraphSearchTool;
pub struct OrbitGraphShowTool;
pub struct OrbitGraphRefsTool;
pub struct OrbitGraphCalleesTool;
pub struct OrbitGraphImpactTool;
pub struct OrbitGraphTraceTool;

impl Tool for OrbitGraphSyncTool {
    fn schema(&self) -> ToolSchema {
        schema(
            "orbit.graph.sync",
            "Use when the orbit-graph index may be stale and graph reads need current results. Prefer over grep when follow-up graph queries should reflect recent file changes.",
            vec![param(
                "full",
                "Run a full sync instead of an incremental auto sync.",
                "boolean",
                false,
            )],
        )
    }

    fn execute(&self, ctx: &ToolContext, input: Value) -> Result<Value, OrbitError> {
        let graph = open_graph(ctx, &input, SyncPolicy::Manual)?;
        graph_sync(&graph, &input)
    }
}

impl Tool for OrbitGraphSearchTool {
    fn schema(&self) -> ToolSchema {
        schema(
            "orbit.graph.search",
            "Use when finding symbols, notable strings, or config keys by text. Prefer over grep when structured symbol metadata or language filters matter.",
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
        )
    }

    fn execute(&self, ctx: &ToolContext, input: Value) -> Result<Value, OrbitError> {
        let graph = open_graph(ctx, &input, windowed_policy())?;
        graph_search(&graph, &input)
    }
}

impl Tool for OrbitGraphShowTool {
    fn schema(&self) -> ToolSchema {
        schema(
            "orbit.graph.show",
            "Use when inspecting one known orbit-graph selector's source and metadata, including UTF-8 `text` or fallback `bytes`. Prefer over grep when the selector is already known.",
            vec![
                param("selector", "Selector to show.", "string", true),
                param(
                    "max_bytes",
                    "Maximum source bytes returned in `text` or fallback `bytes`.",
                    "number",
                    false,
                ),
            ],
        )
    }

    fn execute(&self, ctx: &ToolContext, input: Value) -> Result<Value, OrbitError> {
        let graph = open_graph(ctx, &input, windowed_policy())?;
        graph_show(&graph, &input)
    }
}

impl Tool for OrbitGraphRefsTool {
    fn schema(&self) -> ToolSchema {
        schema(
            "orbit.graph.refs",
            "Use when finding inbound references and relations for a symbol selector. Prefer over grep when cross-file graph resolution is needed.",
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
        )
    }

    fn execute(&self, ctx: &ToolContext, input: Value) -> Result<Value, OrbitError> {
        let graph = open_graph(ctx, &input, windowed_policy())?;
        graph_refs(&graph, &input)
    }
}

impl Tool for OrbitGraphCalleesTool {
    fn schema(&self) -> ToolSchema {
        schema(
            "orbit.graph.callees",
            "Use when finding outbound calls from a symbol selector. Prefer over grep when call edges are needed instead of textual matches.",
            vec![param("symbol", "Symbol selector to query.", "string", true)],
        )
    }

    fn execute(&self, ctx: &ToolContext, input: Value) -> Result<Value, OrbitError> {
        let graph = open_graph(ctx, &input, windowed_policy())?;
        graph_callees(&graph, &input)
    }
}

impl Tool for OrbitGraphImpactTool {
    fn schema(&self) -> ToolSchema {
        schema(
            "orbit.graph.impact",
            "Use when estimating the blast radius from a symbol selector. Prefer over grep when transitive graph impact matters.",
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
        )
    }

    fn execute(&self, ctx: &ToolContext, input: Value) -> Result<Value, OrbitError> {
        let graph = open_graph(ctx, &input, windowed_policy())?;
        graph_impact(&graph, &input)
    }
}

impl Tool for OrbitGraphTraceTool {
    fn schema(&self) -> ToolSchema {
        schema(
            "orbit.graph.trace",
            "Use when tracing a command handler call tree from graph command metadata. Prefer over grep when call-tree structure matters.",
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
        )
    }

    fn execute(&self, ctx: &ToolContext, input: Value) -> Result<Value, OrbitError> {
        let graph = open_graph(ctx, &input, windowed_policy())?;
        graph_trace(&graph, &input)
    }
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

fn windowed_policy() -> SyncPolicy {
    SyncPolicy::Windowed {
        window: GRAPH_SYNC_WINDOW,
    }
}

fn open_graph(ctx: &ToolContext, input: &Value, policy: SyncPolicy) -> Result<Graph, OrbitError> {
    let worktree = resolve_worktree(ctx, input)?;
    Graph::open(worktree.as_path(), policy).map_err(graph_error_to_orbit)
}

fn resolve_worktree(ctx: &ToolContext, input: &Value) -> Result<PathBuf, OrbitError> {
    let workspace_root = ctx.workspace_root.as_deref().ok_or_else(|| {
        OrbitError::InvalidInput("workspace_root is required for orbit.graph.*".to_string())
    })?;
    let canonical_workspace_root = canonicalize_existing_dir(workspace_root, "workspace_root")?;
    let override_path = input
        .get("workspace_path")
        .or_else(|| input.get("workspace"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let Some(override_path) = override_path else {
        return Ok(canonical_workspace_root);
    };

    let candidate = resolve_candidate_path(override_path, workspace_root);
    let canonical_candidate = canonicalize_existing_dir(candidate.as_path(), "workspace_path")?;
    ensure_path_within_boundary(
        canonical_candidate.as_path(),
        canonical_workspace_root.as_path(),
        "workspace_path",
        "workspace_root",
    )?;
    Ok(canonical_candidate)
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
    let kind = super::optional_string(input, "kind")?
        .map(|value| parse_search_kind(value.as_str()))
        .transpose()?;
    let query = SearchQuery {
        query: super::required_string(input, &["query"], "query")?,
        kind,
        lang: super::optional_string(input, "lang")?,
        limit: optional_usize(input, "limit")?,
    };
    to_json(graph.search(&query).map_err(graph_error_to_orbit)?)
}

fn graph_show(graph: &Graph, input: &Value) -> Result<Value, OrbitError> {
    let selector = parse_selector(super::required_string(input, &["selector"], "selector")?)?;
    let max_bytes = optional_usize(input, "max_bytes")?.unwrap_or(DEFAULT_SHOW_MAX_BYTES);
    to_json(
        graph
            .show(&selector, max_bytes)
            .map_err(graph_error_to_orbit)?,
    )
}

fn graph_refs(graph: &Graph, input: &Value) -> Result<Value, OrbitError> {
    let selector = parse_selector(super::required_string(
        input,
        &["symbol", "selector"],
        "symbol",
    )?)?;
    let opts = RefOpts {
        confidence: optional_confidence(input)?,
        kind: super::optional_string(input, "kind")?
            .map(|value| parse_ref_kind(value.as_str()))
            .transpose()?,
    };
    to_json(graph.refs(&selector, &opts).map_err(graph_error_to_orbit)?)
}

fn graph_callees(graph: &Graph, input: &Value) -> Result<Value, OrbitError> {
    let selector = parse_selector(super::required_string(
        input,
        &["symbol", "selector"],
        "symbol",
    )?)?;
    Ok(json!({
        "callees": graph.callees(&selector).map_err(graph_error_to_orbit)?,
    }))
}

fn graph_impact(graph: &Graph, input: &Value) -> Result<Value, OrbitError> {
    let selector = parse_selector(super::required_string(input, &["selector"], "selector")?)?;
    let depth = optional_u8(input, "depth")?.unwrap_or(DEFAULT_IMPACT_DEPTH);
    let min_confidence = optional_confidence(input)?;
    to_json(
        graph
            .impact(&selector, depth, min_confidence)
            .map_err(graph_error_to_orbit)?,
    )
}

fn graph_trace(graph: &Graph, input: &Value) -> Result<Value, OrbitError> {
    let command_name = super::required_string(input, &["command_name", "command"], "command_name")?;
    let depth = optional_u8(input, "depth")?.unwrap_or(DEFAULT_TRACE_DEPTH);
    let min_confidence = optional_confidence(input)?;
    to_json(
        graph
            .trace(command_name.as_str(), depth, min_confidence)
            .map_err(graph_error_to_orbit)?,
    )
}

fn optional_confidence(input: &Value) -> Result<RefConfidence, OrbitError> {
    super::optional_string(input, "confidence")?
        .map(|value| parse_confidence(value.as_str()))
        .transpose()
        .map(|confidence| confidence.unwrap_or_default())
}

fn optional_usize(input: &Value, key: &str) -> Result<Option<usize>, OrbitError> {
    let Some(value) = input.get(key) else {
        return Ok(None);
    };
    let Some(raw) = value.as_u64() else {
        return Err(OrbitError::InvalidInput(format!(
            "`{key}` must be a non-negative integer"
        )));
    };
    usize::try_from(raw)
        .map(Some)
        .map_err(|error| OrbitError::InvalidInput(format!("`{key}` is too large: {error}")))
}

fn optional_u8(input: &Value, key: &str) -> Result<Option<u8>, OrbitError> {
    let Some(value) = input.get(key) else {
        return Ok(None);
    };
    let Some(raw) = value.as_u64() else {
        return Err(OrbitError::InvalidInput(format!(
            "`{key}` must be a non-negative integer"
        )));
    };
    u8::try_from(raw)
        .map(Some)
        .map_err(|error| OrbitError::InvalidInput(format!("`{key}` is too large: {error}")))
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

fn to_json<T: Serialize>(value: T) -> Result<Value, OrbitError> {
    serde_json::to_value(value)
        .map_err(|error| OrbitError::Execution(format!("serialize orbit-graph response: {error}")))
}

fn graph_error_to_orbit(error: GraphError) -> OrbitError {
    match error {
        GraphError::Io {
            operation,
            path,
            reason,
        } => OrbitError::Io(format!("{operation} at {}: {reason}", path.display())),
        GraphError::Sqlite { operation, reason } => {
            OrbitError::Execution(format!("{operation}: {reason}"))
        }
        GraphError::InvalidData { operation, reason } => {
            OrbitError::InvalidInput(format!("{operation}: {reason}"))
        }
        other => OrbitError::Execution(other.to_string()),
    }
}

fn selector_error_to_orbit(error: SelectorParseError) -> OrbitError {
    OrbitError::InvalidInput(format!("invalid selector: {error}"))
}

fn resolve_candidate_path(raw: &str, base: &Path) -> PathBuf {
    let path = Path::new(raw);
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        base.join(path)
    }
}

fn canonicalize_existing_dir(path: &Path, label: &str) -> Result<PathBuf, OrbitError> {
    let canonical = path.canonicalize().map_err(|error| {
        OrbitError::InvalidInput(format!("failed to canonicalize `{label}`: {error}"))
    })?;
    if !canonical.is_dir() {
        return Err(OrbitError::InvalidInput(format!(
            "`{label}` must reference an existing directory: {}",
            canonical.display()
        )));
    }
    Ok(canonical)
}

fn ensure_path_within_boundary(
    path: &Path,
    boundary: &Path,
    path_label: &str,
    boundary_label: &str,
) -> Result<(), OrbitError> {
    if !path.starts_with(boundary) {
        return Err(OrbitError::InvalidInput(format!(
            "`{path_label}` must stay within {boundary_label}: {}",
            path.display()
        )));
    }
    Ok(())
}
