use std::fs;
use std::path::Path;
use std::process::Command;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use orbit_common::types::{
    McpCapability, McpToolDefinition, McpToolPlacement, NotFoundKind, OrbitError, ToolParam,
    ToolSchema, ToolSessionContext,
};
use serde_json::{Value, json};
use tempfile::TempDir;

use super::super::graph::{GraphToolRegistry, graph_tool_definitions};
use super::support::{StubHost, WireServer, test_mcp_definitions, tool_schema};
use orbit_mcp::{
    McpHost, McpServerComposition, McpToolExtension, McpToolExtensionRegistration, OrbitToolServer,
};

use super::super::schema::RemoteInputSchemaResolver;

#[test]
fn graph_tool_schemas_cover_cli_parameters() {
    let definitions = graph_tool_definitions().expect("graph definitions are valid");
    assert!(definitions.iter().all(|definition| {
        definition.policy.placement() == McpToolPlacement::LocalDerived
            && definition.policy.allowed_capabilities()
                == &[McpCapability::Agent, McpCapability::Operator]
                    .into_iter()
                    .collect()
    }));
    let schemas: Vec<_> = definitions
        .into_iter()
        .map(|definition| definition.schema)
        .collect();
    let names: Vec<_> = schemas.iter().map(|schema| schema.name.as_str()).collect();
    assert_eq!(
        names,
        vec![
            "orbit.graph.sync",
            "orbit.graph.search",
            "orbit.graph.show",
            "orbit.graph.refs",
            "orbit.graph.callees",
            "orbit.graph.impact",
            "orbit.graph.trace",
            "orbit.graph.overview",
            "orbit.graph.implementors",
            "orbit.graph.deps",
        ]
    );

    assert_param_names(&schemas[0], &with_workspace_params(&["full"]));
    assert_param_names(
        &schemas[1],
        &with_workspace_params(&["query", "kind", "lang", "limit"]),
    );
    assert_param_names(
        &schemas[2],
        &with_workspace_params(&["selector", "max_bytes"]),
    );
    assert_param_names(
        &schemas[3],
        &with_workspace_params(&["symbol", "confidence", "kind"]),
    );
    assert_param_names(&schemas[4], &with_workspace_params(&["symbol"]));
    assert_param_names(
        &schemas[5],
        &with_workspace_params(&["selector", "depth", "confidence"]),
    );
    assert_param_names(
        &schemas[6],
        &with_workspace_params(&["command_name", "depth", "confidence"]),
    );
    assert_param_names(&schemas[7], &with_workspace_params(&["scope", "format"]));
    assert_param_names(&schemas[8], &with_workspace_params(&["selector"]));
    assert_param_names(&schemas[9], &with_workspace_params(&["selector"]));
    assert_workspace_params_optional_strings(&schemas);
}

#[test]
fn graph_tool_schema_bytes_stay_under_budget() {
    const LEGACY_BASELINE_BYTES: usize = 10_995;
    const MAX_BYTES: usize = 8_246;

    let definitions = graph_tool_definitions().expect("graph definitions are valid");
    let total = definitions
        .iter()
        .map(|definition| {
            serde_json::to_string(&definition.schema)
                .expect("serialize graph schema")
                .len()
        })
        .sum::<usize>();

    assert!(
        total > 0,
        "the budget must measure the active Remote graph surface"
    );
    assert!(
        total <= MAX_BYTES,
        "graph tool schema bytes grew to {total} (legacy baseline {LEGACY_BASELINE_BYTES}, max {MAX_BYTES})"
    );
}

#[tokio::test]
async fn combined_schemas_replace_known_host_graph_tools_and_preserve_unknown_ones() {
    let host = Arc::new(StubHost {
        schemas: vec![
            tool_schema("orbit.graph.search"),
            tool_schema("orbit.graph.pack"),
            tool_schema("orbit.task.show"),
        ],
    });
    let (server, _) =
        server_with_graph_extension(host, ToolSessionContext::trusted_local(None, None, None))
            .await;
    let tools = server.list_tools().await.tools;
    let names: Vec<_> = tools.iter().map(|tool| tool.name.as_ref()).collect();

    assert_eq!(
        tools
            .iter()
            .filter(|tool| tool.name.as_ref() == "orbit_graph_search")
            .count(),
        1
    );
    assert!(names.contains(&"orbit_graph_pack"));
    assert!(names.contains(&"orbit_task_show"));
    assert!(names.contains(&"orbit_graph_sync"));
    assert!(names.contains(&"orbit_graph_callees"));
    assert!(names.contains(&"orbit_graph_impact"));
    assert!(names.contains(&"orbit_graph_trace"));
}

#[tokio::test]
async fn reexposed_graph_schema_still_crosses_in_process_policy_seam() {
    struct ReexposedGraphHost {
        host_calls: AtomicUsize,
        in_process_calls: AtomicUsize,
    }

    impl McpHost for ReexposedGraphHost {
        fn list_mcp_tool_definitions(&self) -> Result<Vec<McpToolDefinition>, OrbitError> {
            test_mcp_definitions(vec![tool_schema("orbit.graph.search")])
        }

        fn call_tool(
            &self,
            _name: &str,
            _input: Value,
            _session_context: ToolSessionContext,
        ) -> Result<Value, OrbitError> {
            self.host_calls.fetch_add(1, Ordering::SeqCst);
            Ok(json!({ "bypassed": true }))
        }

        fn call_in_process_tool(
            &self,
            name: &str,
            _input: Value,
            _session_context: ToolSessionContext,
            _dispatch: &mut dyn FnMut(Value, ToolSessionContext) -> Result<Value, OrbitError>,
        ) -> Result<Value, OrbitError> {
            self.in_process_calls.fetch_add(1, Ordering::SeqCst);
            Err(OrbitError::not_found(NotFoundKind::Tool, name.to_string()))
        }
    }

    let host = Arc::new(ReexposedGraphHost {
        host_calls: AtomicUsize::new(0),
        in_process_calls: AtomicUsize::new(0),
    });
    let (server, _) = server_with_graph_extension(
        host.clone(),
        ToolSessionContext::trusted_local(None, None, None),
    )
    .await;

    let result = server
        .call("orbit.graph.search", json!({ "query": "dispatch" }))
        .await;

    assert!(result.is_error.unwrap_or(false));
    assert_eq!(host.in_process_calls.load(Ordering::SeqCst), 1);
    assert_eq!(host.host_calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn combined_schemas_use_adapter_graph_tools_when_host_has_no_graph_surface() {
    let host = Arc::new(StubHost {
        schemas: vec![tool_schema("orbit.task.show")],
    });
    let (server, _) =
        server_with_graph_extension(host, ToolSessionContext::trusted_local(None, None, None))
            .await;
    let tools = server.list_tools().await.tools;
    let names: Vec<_> = tools.iter().map(|tool| tool.name.as_ref()).collect();

    assert!(names.contains(&"orbit_task_show"));
    assert!(names.contains(&"orbit_graph_sync"));
    assert!(names.contains(&"orbit_graph_search"));
    assert!(names.contains(&"orbit_graph_trace"));
}

#[tokio::test]
async fn remote_graph_aware_tools_list_matches_wire_snapshot() {
    let host = Arc::new(StubHost {
        schemas: vec![
            ToolSchema {
                name: "orbit.task.add".to_string(),
                description: "Create a task record in the fixture store.".to_string(),
                parameters: vec![
                    snapshot_param("title", "Task title", "string", true),
                    snapshot_param("type", "Optional task type", "string", false),
                    snapshot_param("tags", "Optional tags", "string_list", false),
                ],
                builtin: true,
            },
            ToolSchema {
                name: "orbit.task.show".to_string(),
                description: "Show one task record from the fixture store.".to_string(),
                parameters: vec![snapshot_param("id", "Task id", "string", true)],
                builtin: true,
            },
            ToolSchema {
                name: "orbit.task.list".to_string(),
                description: "List every task record in the fixture store.".to_string(),
                parameters: Vec::new(),
                builtin: true,
            },
        ],
    });
    let (server, _) =
        server_with_graph_extension(host, ToolSessionContext::trusted_local(None, None, None))
            .await;
    let tools = serde_json::to_value(server.list_tools().await.tools).expect("serialize tools");
    let expected: Value = serde_json::from_str(include_str!("snapshots/wire_tools_list.json"))
        .expect("parse checked-in Remote wire snapshot");

    assert_eq!(tools, expected, "Remote MCP tools/list schema drift");
}

#[tokio::test]
async fn graph_tools_invoke_in_process_fixture() {
    let worktree = fixture_worktree();
    let host = Arc::new(StubHost {
        schemas: Vec::new(),
    });
    let (server, graph_tools) =
        server_with_graph_extension(host, agent_workspace_context(worktree.path())).await;

    let sync = call_json(
        &server,
        "orbit.graph.sync",
        json!({
            "full": true
        }),
    )
    .await;
    assert!(sync["files_indexed"].as_u64().expect("files_indexed") >= 1);
    assert!(sync.get("duration_ms").is_some());

    let search = call_json(
        &server,
        "orbit.graph.search",
        json!({
            "query": "helper",
            "kind": "symbol",
            "limit": 5
        }),
    )
    .await;
    assert_array_field(&search, "matches");

    let show = call_json(
        &server,
        "orbit.graph.show",
        json!({
            "selector": "symbol:src/lib.rs#entry:function",
            "max_bytes": 256
        }),
    )
    .await;
    assert_eq!(show["metadata"]["file"], "src/lib.rs");
    assert!(
        show["source"]
            .as_str()
            .is_some_and(|source| source.contains("pub fn entry")),
        "source should be UTF-8 text in {show}"
    );
    assert!(show.get("bytes").is_none());

    let refs = call_json(
        &server,
        "orbit.graph.refs",
        json!({
            "symbol": "symbol:src/lib.rs#helper:function",
            "confidence": "fuzzy",
            "kind": "call"
        }),
    )
    .await;
    assert!(refs.get("target").is_some());
    assert_array_field(&refs, "refs");
    assert_array_field(&refs, "relations");

    let callees = call_json(
        &server,
        "orbit.graph.callees",
        json!({
            "symbol": "symbol:src/lib.rs#entry:function"
        }),
    )
    .await;
    assert_array_field(&callees, "callees");

    let impact = call_json(
        &server,
        "orbit.graph.impact",
        json!({
            "selector": "symbol:src/lib.rs#entry:function",
            "depth": 2,
            "confidence": "same_module"
        }),
    )
    .await;
    assert_array_field(&impact, "touched");
    assert!(impact.get("visited_nodes").is_some());

    let trace = call_json(
        &server,
        "orbit.graph.trace",
        json!({
            "command_name": "missing-command",
            "depth": 2,
            "confidence": "same_module"
        }),
    )
    .await;
    assert!(trace["root"].is_null());
    assert_eq!(trace["visited_nodes"], 0);

    let overview = call_json(
        &server,
        "orbit.graph.overview",
        json!({
            "format": "full"
        }),
    )
    .await;
    assert_eq!(overview["format"], "full");
    assert!(overview["total_files"].as_u64().expect("total_files") >= 1);
    assert!(
        overview["total_symbols"].as_u64().expect("total_symbols") >= 3,
        "fixture defines helper/entry/caller: {overview}"
    );
    assert_array_field(&overview, "files");

    let implementors = call_json(
        &server,
        "orbit.graph.implementors",
        json!({
            "selector": "symbol:src/lib.rs#Missing:trait"
        }),
    )
    .await;
    assert_eq!(implementors["trait_name"], "Missing");
    assert_array_field(&implementors, "implementors");

    let deps = call_json(
        &server,
        "orbit.graph.deps",
        json!({
            "selector": "file:src/lib.rs"
        }),
    )
    .await;
    assert_eq!(deps["scope"], "file:src/lib.rs");
    assert_array_field(&deps, "imports");

    assert_eq!(graph_tools.cached_worktree_count(), 1);
}

#[tokio::test]
async fn graph_tool_errors_are_structured_mcp_tool_errors() {
    let worktree = fixture_worktree();
    let host = Arc::new(StubHost {
        schemas: Vec::new(),
    });
    let (server, _) =
        server_with_graph_extension(host, agent_workspace_context(worktree.path())).await;

    let result = server
        .call("orbit.graph.show", json!({ "selector": "not-a-selector" }))
        .await;

    assert!(result.is_error.unwrap_or(false));
    let payload = result.structured_content.expect("structured error payload");
    assert_eq!(payload["code"], "invalid_input");
    assert!(
        payload["message"]
            .as_str()
            .expect("message")
            .contains("invalid selector")
    );
}

#[tokio::test]
async fn graph_show_returns_labeled_byte_fallback_for_non_utf8_source() {
    let worktree = fixture_worktree();
    let source = b"pub fn broken() { \xFF }\n";
    fs::write(worktree.path().join("src/non_utf8.rs"), source).expect("write non-utf8 source");
    run_git(worktree.path(), ["add", "src/non_utf8.rs"]);
    run_git(worktree.path(), ["commit", "-m", "add non-utf8 fixture"]);

    let host = Arc::new(StubHost {
        schemas: Vec::new(),
    });
    let (server, _) =
        server_with_graph_extension(host, agent_workspace_context(worktree.path())).await;

    call_json(
        &server,
        "orbit.graph.sync",
        json!({
            "full": true
        }),
    )
    .await;
    let show = call_json(
        &server,
        "orbit.graph.show",
        json!({
            "selector": "file:src/non_utf8.rs",
            "max_bytes": 256
        }),
    )
    .await;

    assert_eq!(
        show["source"],
        json!({
            "encoding": "bytes",
            "bytes": source
        })
    );
    assert!(show.get("bytes").is_none());
}

#[tokio::test]
async fn graph_show_rejects_out_of_workspace_path_without_session_workspace() {
    // ORB-00361: with no announced session workspace, a client-supplied
    // `workspace_path` outside the process working directory must be rejected
    // before any graph is opened or indexed — no arbitrary-directory source read.
    let outside = TempDir::new().expect("temp dir outside cwd");
    fs::create_dir_all(outside.path().join("src")).expect("create src");
    fs::write(outside.path().join("src/lib.rs"), "pub fn secret() {}\n").expect("write file");

    let host = Arc::new(StubHost {
        schemas: Vec::new(),
    });
    let (server, graph_tools) =
        server_with_graph_extension(host, ToolSessionContext::trusted_local(None, None, None))
            .await;
    // Intentionally do NOT announce a session workspace: session_context.workspace
    // stays None, which is the unguarded path before this fix.

    let result = server
        .call(
            "orbit.graph.show",
            json!({
                "workspace_path": outside.path().display().to_string(),
                "selector": "symbol:src/lib.rs#secret:function",
                "max_bytes": 256
            }),
        )
        .await;

    assert!(result.is_error.unwrap_or(false), "call must be rejected");
    let payload = result.structured_content.expect("structured error payload");
    assert_eq!(payload["code"], "invalid_input");
    assert!(
        payload["message"]
            .as_str()
            .expect("message")
            .contains("requires a validated exact checkout workspace selector"),
        "unexpected error message: {payload}"
    );
    // No source bytes were returned and no graph was opened/indexed for the
    // out-of-bounds directory.
    assert!(payload.get("bytes").is_none());
    assert!(payload.get("source").is_none());
    assert_eq!(graph_tools.cached_worktree_count(), 0);
}

async fn server_with_graph_extension(
    host: Arc<dyn McpHost>,
    context: ToolSessionContext,
) -> (WireServer, Arc<GraphToolRegistry>) {
    let graph_tools = Arc::new(GraphToolRegistry::new());
    let extension: Arc<dyn McpToolExtension> = graph_tools.clone();
    let workspace = context.workspace.clone();
    let composition = McpServerComposition::new()
        .with_tool_extension(McpToolExtensionRegistration::advertised(extension))
        .with_input_schema_resolver(Arc::new(RemoteInputSchemaResolver));
    let server = OrbitToolServer::new_with_context_and_composition(host, context, composition);
    (
        WireServer::new(server, workspace.as_deref()).await,
        graph_tools,
    )
}

fn snapshot_param(name: &str, description: &str, param_type: &str, required: bool) -> ToolParam {
    ToolParam {
        name: name.to_string(),
        description: description.to_string(),
        param_type: param_type.to_string(),
        required,
    }
}

async fn call_json(server: &WireServer, name: &str, args: Value) -> Value {
    let result = server.call(name, args).await;
    assert!(
        !result.is_error.unwrap_or(false),
        "{name} should not return a tool error: {result:?}"
    );
    result.structured_content.expect("structured content")
}

fn assert_array_field(value: &Value, field: &str) {
    assert!(
        value.get(field).and_then(Value::as_array).is_some(),
        "{field} should be an array in {value}"
    );
}

fn assert_param_names(schema: &orbit_common::types::ToolSchema, expected: &[&str]) {
    let names: Vec<_> = schema
        .parameters
        .iter()
        .map(|param| param.name.as_str())
        .collect();
    assert_eq!(names, expected);
}

fn agent_workspace_context(path: &Path) -> ToolSessionContext {
    let mut context = ToolSessionContext::trusted_local(None, None, None);
    context.workspace = Some(path.display().to_string());
    context
}

fn with_workspace_params(base: &[&'static str]) -> Vec<&'static str> {
    base.iter()
        .copied()
        .chain(["workspace_path", "workspace"])
        .collect()
}

fn assert_workspace_params_optional_strings(schemas: &[orbit_common::types::ToolSchema]) {
    for schema in schemas {
        for param_name in ["workspace_path", "workspace"] {
            let param = schema
                .parameters
                .iter()
                .find(|param| param.name == param_name)
                .expect("workspace parameter exists");
            assert_eq!(param.param_type, "string");
            assert!(!param.required);
        }
    }
}

fn fixture_worktree() -> TempDir {
    let tempdir = TempDir::new().expect("temp worktree");
    run_git(tempdir.path(), ["init", "-b", "main"]);
    run_git(
        tempdir.path(),
        ["config", "user.email", "orbit@example.invalid"],
    );
    run_git(tempdir.path(), ["config", "user.name", "Orbit Test"]);

    fs::create_dir_all(tempdir.path().join("src")).expect("create src");
    fs::write(
        tempdir.path().join("src/lib.rs"),
        r#"
pub fn helper() -> i32 {
    1
}

pub fn entry() -> i32 {
    helper()
}

pub fn caller() -> i32 {
    entry()
}
"#,
    )
    .expect("write fixture");
    fs::write(
        tempdir.path().join("Cargo.toml"),
        "[package]\nname = \"graph_mcp_fixture\"\nversion = \"0.0.0\"\nedition = \"2024\"\n",
    )
    .expect("write manifest");

    run_git(tempdir.path(), ["add", "."]);
    run_git(tempdir.path(), ["commit", "-m", "fixture"]);
    tempdir
}

fn run_git<const N: usize>(worktree: &Path, args: [&str; N]) {
    let output = Command::new("git")
        .current_dir(worktree)
        .args(args)
        .output()
        .expect("run git");
    assert!(
        output.status.success(),
        "git failed: stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}
