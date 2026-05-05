use std::fs;
use std::path::Path;
use std::process::Command;

use orbit_common::types::OrbitError;
use orbit_knowledge::graph::object_store::RefName;
use orbit_knowledge::graph::{
    BaseNodeFields, CodebaseGraphV1, DirNode, FileNode, GraphObjectStore, LeafKind, LeafNode,
};
use orbit_knowledge::pipeline::context::BuildConfig;
use orbit_tools::{ToolContext, ToolRegistry};
use serde_json::{Value, json};
use tempfile::TempDir;

const GRAPH_REF: &str = "main";

#[test]
fn search_prefers_code_symbols_and_hides_non_code_by_default() {
    let runtime_file = file_node("src/runtime.rs", "rust", Some("rs"), vec![]);
    let config_file = file_node(
        "benchmarks/locate-agentruntime.yaml",
        "yaml",
        Some("yaml"),
        vec![],
    );
    let runtime_trait = leaf_node(
        "src/runtime.rs",
        "AgentRuntime",
        LeafKind::Trait,
        "pub trait AgentRuntime {}",
    );
    let config_key = leaf_node(
        "benchmarks/locate-agentruntime.yaml",
        "AgentRuntime",
        LeafKind::ConfigKey,
        "",
    );
    let fixture = write_graph_fixture(graph_with_root(
        vec![
            attach_leaf(runtime_file, &runtime_trait),
            attach_leaf(config_file, &config_key),
        ],
        vec![runtime_trait, config_key],
    ));

    let response = execute_graph_tool(
        fixture.path(),
        "orbit.graph.search",
        json!({"query":"AgentRuntime","limit":5}),
    );

    assert_eq!(response["total"], 1);
    assert_eq!(response["results"].as_array().unwrap().len(), 1);
    assert_eq!(
        response["results"][0]["selector"],
        "symbol:src/runtime.rs#AgentRuntime:trait"
    );

    let response = execute_graph_tool(
        fixture.path(),
        "orbit.graph.search",
        json!({"query":"AgentRuntime","limit":5,"include_non_code":true}),
    );

    let selectors: Vec<&str> = response["results"]
        .as_array()
        .unwrap()
        .iter()
        .map(|entry| entry["selector"].as_str().unwrap())
        .collect();
    assert_eq!(selectors[0], "symbol:src/runtime.rs#AgentRuntime:trait");
    assert!(
        selectors.contains(&"symbol:benchmarks/locate-agentruntime.yaml#AgentRuntime:config_key")
    );
}

#[test]
fn search_source_regex_filters_file_source_and_adds_matched_lines() {
    let mut re_export_file = file_node("src/types/mod.rs", "rust", Some("rs"), vec![]);
    re_export_file.source =
        "mod error;\npub use error::OrbitError;\npub use ids::TaskId;\n".to_string();
    let mut definition_file = file_node("src/types/error.rs", "rust", Some("rs"), vec![]);
    definition_file.source = "pub enum OrbitError {\n    Message(String),\n}\n".to_string();
    let fixture = write_graph_fixture(graph_with_root(
        vec![re_export_file, definition_file],
        Vec::new(),
    ));

    let response = execute_graph_tool(
        fixture.path(),
        "orbit.graph.search",
        json!({
            "query": "",
            "type": "file",
            "prefix": "src/types/",
            "source_regex": "^\\s*pub\\s+use\\s+.*OrbitError",
            "limit": 50
        }),
    );

    assert_eq!(response["total"], 1);
    assert_eq!(response["results"][0]["selector"], "file:src/types/mod.rs");
    assert_eq!(response["results"][0]["matched_lines"][0]["line_number"], 2);
    assert_eq!(
        response["results"][0]["matched_lines"][0]["snippet"],
        "pub use error::OrbitError;"
    );

    let query_keeps_name_location_semantics = execute_graph_tool(
        fixture.path(),
        "orbit.graph.search",
        json!({
            "query": "OrbitError",
            "type": "file",
            "prefix": "src/types/",
            "source_regex": "^\\s*pub\\s+use\\s+.*OrbitError",
            "limit": 50
        }),
    );
    assert_eq!(query_keeps_name_location_semantics["total"], 0);
}

#[test]
fn search_source_regex_selector_format_stays_plain_selectors() {
    let mut file = file_node("src/lib.rs", "rust", Some("rs"), vec![]);
    file.source = "pub const MAX_RETRIES: usize = 3;\n".to_string();
    let fixture = write_graph_fixture(graph_with_root(vec![file], Vec::new()));

    let response = execute_graph_tool(
        fixture.path(),
        "orbit.graph.search",
        json!({
            "query": "",
            "type": "file",
            "prefix": "src/",
            "source_regex": "^\\s*pub\\s+const\\s+",
            "format": "selectors"
        }),
    );

    assert_eq!(response, json!(["file:src/lib.rs"]));
}

#[test]
fn search_source_regex_rejects_invalid_and_unbounded_inputs() {
    let mut file = file_node("src/lib.rs", "rust", Some("rs"), vec![]);
    file.source = "pub const MAX_RETRIES: usize = 3;\n".to_string();
    let fixture = write_graph_fixture(graph_with_root(vec![file], Vec::new()));

    let invalid_regex = execute_graph_tool_result(
        fixture.path(),
        "orbit.graph.search",
        json!({"query": "lib", "source_regex": "["}),
    )
    .unwrap_err()
    .to_string();
    assert!(invalid_regex.contains("invalid `source_regex`"));

    let omitted_limit = execute_graph_tool_result(
        fixture.path(),
        "orbit.graph.search",
        json!({"query": "", "source_regex": "pub const"}),
    )
    .unwrap_err()
    .to_string();
    assert!(omitted_limit.contains("requires explicit `limit` <= 200"));

    let too_large_limit = execute_graph_tool_result(
        fixture.path(),
        "orbit.graph.search",
        json!({"query": "", "source_regex": "pub const", "limit": 201}),
    )
    .unwrap_err()
    .to_string();
    assert!(too_large_limit.contains("requires explicit `limit` <= 200"));
}

#[test]
fn search_task_id_filter_plumbs_json_input() {
    let mut touched = leaf_node(
        "src/runtime.rs",
        "AgentRuntime",
        LeafKind::Trait,
        "pub trait AgentRuntime {}",
    );
    touched.base.task_ids = vec!["T20260421-0528".to_string()];
    let mut untouched = leaf_node(
        "src/engine.rs",
        "EngineRuntime",
        LeafKind::Struct,
        "pub struct EngineRuntime;",
    );
    untouched.base.task_ids = vec!["T20260421-0001".to_string()];
    let fixture = write_graph_fixture(graph_with_root(
        vec![
            attach_leaf(
                file_node("src/runtime.rs", "rust", Some("rs"), vec![]),
                &touched,
            ),
            attach_leaf(
                file_node("src/engine.rs", "rust", Some("rs"), vec![]),
                &untouched,
            ),
        ],
        vec![touched, untouched],
    ));

    let response = execute_graph_tool(
        fixture.path(),
        "orbit.graph.search",
        json!({
            "query": "Runtime",
            "type": "symbol",
            "task_id": "T20260421-0528",
            "limit": 10
        }),
    );

    assert_eq!(response["total"], 1);
    assert_eq!(response["results"].as_array().unwrap().len(), 1);
    assert_eq!(
        response["results"][0]["selector"],
        "symbol:src/runtime.rs#AgentRuntime:trait"
    );
}

#[test]
fn search_task_id_filter_accepts_current_short_suffix() {
    let mut touched = leaf_node(
        "src/runtime.rs",
        "AgentRuntime",
        LeafKind::Trait,
        "pub trait AgentRuntime {}",
    );
    touched.base.task_ids = vec!["T20260428-1".to_string()];
    let fixture = write_graph_fixture(graph_with_root(
        vec![attach_leaf(
            file_node("src/runtime.rs", "rust", Some("rs"), vec![]),
            &touched,
        )],
        vec![touched],
    ));

    let response = execute_graph_tool(
        fixture.path(),
        "orbit.graph.search",
        json!({
            "query": "Runtime",
            "type": "symbol",
            "task_id": "T20260428-1",
            "limit": 10
        }),
    );

    assert_eq!(response["total"], 1);
    assert_eq!(
        response["results"][0]["selector"],
        "symbol:src/runtime.rs#AgentRuntime:trait"
    );
}

#[test]
fn search_task_id_filter_rejects_malformed_input() {
    let fixture = write_graph_fixture(graph_with_root(Vec::new(), Vec::new()));

    let error = execute_graph_tool_result(
        fixture.path(),
        "orbit.graph.search",
        json!({"task_id": "T20260421"}),
    )
    .unwrap_err()
    .to_string();

    assert!(error.contains("`task_id` must match T\\d{8}-\\d+(?:-\\d+)*"));
}

#[test]
fn search_and_pack_surface_reads_typescript_build_output() {
    let repo = tempfile::tempdir().unwrap();
    write_repo_file(
        repo.path(),
        "src/types.ts",
        "export type WidgetState = \"idle\" | \"ready\";\n\
export interface WidgetConfig {\n\
    state: WidgetState;\n\
}\n",
    );
    write_repo_file(
        repo.path(),
        "src/view.tsx",
        "export const WidgetView = ({ state }: { state: string }) => (\n\
    <span>{state}</span>\n\
);\n",
    );
    run_knowledge_build(repo.path());

    let search_response = execute_graph_tool(
        repo.path(),
        "orbit.graph.search",
        json!({"query":"WidgetState","type":"symbol","limit":5}),
    );
    assert_eq!(search_response["total"], 1);
    assert_eq!(
        search_response["results"][0]["selector"],
        "symbol:src/types.ts#WidgetState:type_alias"
    );
    assert_eq!(search_response["results"][0]["kind"], "type_alias");

    let tsx_response = execute_graph_tool(
        repo.path(),
        "orbit.graph.search",
        json!({"query":"WidgetView","type":"symbol","limit":5}),
    );
    assert_eq!(tsx_response["total"], 1);
    assert_eq!(
        tsx_response["results"][0]["selector"],
        "symbol:src/view.tsx#WidgetView:function"
    );

    let pack_response = execute_graph_tool(
        repo.path(),
        "orbit.graph.pack",
        json!({
            "selectors": "symbol:src/types.ts#WidgetState:type_alias",
            "summary": false
        }),
    );
    assert_eq!(pack_response["entries"][0]["name"], "WidgetState");
    assert_eq!(pack_response["entries"][0]["language"], "typescript");
    assert!(
        pack_response["entries"][0]["source"]
            .as_str()
            .unwrap()
            .contains("type WidgetState")
    );
}

#[test]
fn overview_defaults_to_summary_for_broad_scope() {
    let mut files = Vec::new();
    let mut leaves = Vec::new();

    for index in 0..25 {
        let path = format!("src/file_{index}.rs");
        let name = format!("item_{index}");
        let leaf = leaf_node(
            &path,
            &name,
            LeafKind::Function,
            &format!("fn {name}() {{}}"),
        );
        files.push(attach_leaf(
            file_node(&path, "rust", Some("rs"), vec![]),
            &leaf,
        ));
        leaves.push(leaf);
    }

    let fixture = write_graph_fixture(graph_with_root(files, leaves));
    let response = execute_graph_tool(fixture.path(), "orbit.graph.overview", json!({}));

    assert_eq!(response["mode"], "summary");
    assert_eq!(response["requested_format"], "auto");
    assert!(response.get("files").is_none());

    let full_response = execute_graph_tool(
        fixture.path(),
        "orbit.graph.overview",
        json!({"format":"full"}),
    );
    assert_eq!(full_response["mode"], "full");
    assert_eq!(full_response["files"].as_array().unwrap().len(), 25);
}

#[test]
fn refs_partition_code_doc_and_config_hits() {
    let definition = leaf_node(
        "src/runtime.rs",
        "AgentRuntime",
        LeafKind::Trait,
        "pub trait AgentRuntime {}",
    );
    let code_refs = vec![
        leaf_node(
            "src/claude.rs",
            "ClaudeRuntime",
            LeafKind::Struct,
            "struct ClaudeRuntime;\nimpl AgentRuntime for ClaudeRuntime {}",
        ),
        leaf_node(
            "src/codex.rs",
            "CodexRuntime",
            LeafKind::Struct,
            "struct CodexRuntime;\nimpl AgentRuntime for CodexRuntime {}",
        ),
        leaf_node(
            "src/gemini.rs",
            "GeminiRuntime",
            LeafKind::Struct,
            "struct GeminiRuntime;\nimpl AgentRuntime for GeminiRuntime {}",
        ),
    ];
    let doc_files = vec![
        file_node("README.md", "markdown", Some("md"), vec!["AgentRuntime"]),
        file_node(
            "docs/runtime.md",
            "markdown",
            Some("md"),
            vec!["AgentRuntime"],
        ),
    ];
    let config_file = file_node("runtime.yaml", "yaml", Some("yaml"), vec!["AgentRuntime"]);

    let mut files = vec![attach_leaf(
        file_node("src/runtime.rs", "rust", Some("rs"), vec![]),
        &definition,
    )];
    for leaf in &code_refs {
        files.push(attach_leaf(
            file_node(
                leaf.base.location.split_once('#').unwrap().0,
                "rust",
                Some("rs"),
                vec![],
            ),
            leaf,
        ));
    }
    files.extend(doc_files);
    files.push(config_file);

    let mut leaves = vec![definition];
    leaves.extend(code_refs);
    let fixture = write_graph_fixture(graph_with_root(files, leaves));

    let default_response = execute_graph_tool(
        fixture.path(),
        "orbit.graph.refs",
        json!({"selector":"symbol:src/runtime.rs#AgentRuntime:trait"}),
    );
    assert_eq!(default_response["code_refs"].as_array().unwrap().len(), 3);
    assert!(default_response["doc_refs"].as_array().unwrap().is_empty());
    assert!(
        default_response["config_refs"]
            .as_array()
            .unwrap()
            .is_empty()
    );

    let all_response = execute_graph_tool(
        fixture.path(),
        "orbit.graph.refs",
        json!({"selector":"symbol:src/runtime.rs#AgentRuntime:trait","include":["all"]}),
    );
    assert_eq!(all_response["code_refs"].as_array().unwrap().len(), 3);
    assert_eq!(all_response["doc_refs"].as_array().unwrap().len(), 2);
    assert_eq!(all_response["config_refs"].as_array().unwrap().len(), 1);

    let scalar_all_response = execute_graph_tool(
        fixture.path(),
        "orbit.graph.refs",
        json!({"selector":"symbol:src/runtime.rs#AgentRuntime:trait","include":"all"}),
    );
    assert_eq!(scalar_all_response, all_response);

    let code_only_response = execute_graph_tool(
        fixture.path(),
        "orbit.graph.refs",
        json!({"selector":"symbol:src/runtime.rs#AgentRuntime:trait","include":["code"]}),
    );
    assert_eq!(code_only_response["code_refs"].as_array().unwrap().len(), 3);
    assert!(
        code_only_response["doc_refs"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert!(
        code_only_response["config_refs"]
            .as_array()
            .unwrap()
            .is_empty()
    );

    let doc_only_response = execute_graph_tool(
        fixture.path(),
        "orbit.graph.refs",
        json!({"selector":"symbol:src/runtime.rs#AgentRuntime:trait","include":["doc"]}),
    );
    assert!(
        doc_only_response["code_refs"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert_eq!(doc_only_response["doc_refs"].as_array().unwrap().len(), 2);
    assert!(
        doc_only_response["config_refs"]
            .as_array()
            .unwrap()
            .is_empty()
    );

    let config_only_response = execute_graph_tool(
        fixture.path(),
        "orbit.graph.refs",
        json!({"selector":"symbol:src/runtime.rs#AgentRuntime:trait","include":["config"]}),
    );
    assert!(
        config_only_response["code_refs"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert!(
        config_only_response["doc_refs"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        config_only_response["config_refs"]
            .as_array()
            .unwrap()
            .len(),
        1
    );

    let mixed_response = execute_graph_tool(
        fixture.path(),
        "orbit.graph.refs",
        json!({"selector":"symbol:src/runtime.rs#AgentRuntime:trait","include":["code","config"]}),
    );
    assert_eq!(mixed_response["code_refs"].as_array().unwrap().len(), 3);
    assert!(mixed_response["doc_refs"].as_array().unwrap().is_empty());
    assert_eq!(mixed_response["config_refs"].as_array().unwrap().len(), 1);
}

#[test]
fn refs_rejects_invalid_include_shapes() {
    let definition = leaf_node(
        "src/runtime.rs",
        "AgentRuntime",
        LeafKind::Trait,
        "pub trait AgentRuntime {}",
    );
    let fixture = write_graph_fixture(graph_with_root(
        vec![attach_leaf(
            file_node("src/runtime.rs", "rust", Some("rs"), vec![]),
            &definition,
        )],
        vec![definition],
    ));

    let bogus = execute_graph_tool_result(
        fixture.path(),
        "orbit.graph.refs",
        json!({"selector":"symbol:src/runtime.rs#AgentRuntime:trait","include":"bogus"}),
    )
    .unwrap_err()
    .to_string();
    assert!(bogus.contains("`code`, `doc`, `config`, or `all`"));

    let object_shape = execute_graph_tool_result(
        fixture.path(),
        "orbit.graph.refs",
        json!({"selector":"symbol:src/runtime.rs#AgentRuntime:trait","include":{"kind":"code"}}),
    )
    .unwrap_err()
    .to_string();
    assert!(object_shape.contains("`include` must be a string or array of strings"));
}

#[test]
fn pack_defaults_to_summary_without_leaf_bodies() {
    let impl_leaf = leaf_node(
        "src/runtime.rs",
        "AgentRuntimeImpl",
        LeafKind::Impl,
        "impl AgentRuntime for CodexRuntime {\n    fn run(&self) {}\n}\n",
    );
    let fixture = write_graph_fixture(graph_with_root(
        vec![attach_leaf(
            file_node("src/runtime.rs", "rust", Some("rs"), vec![]),
            &impl_leaf,
        )],
        vec![impl_leaf],
    ));

    let summary_response = execute_graph_tool(
        fixture.path(),
        "orbit.graph.pack",
        json!({"selectors":["symbol:src/runtime.rs#AgentRuntimeImpl:impl"]}),
    );
    let scalar_summary_response = execute_graph_tool(
        fixture.path(),
        "orbit.graph.pack",
        json!({"selectors":"symbol:src/runtime.rs#AgentRuntimeImpl:impl"}),
    );
    assert_eq!(scalar_summary_response, summary_response);

    let file_alias_response = execute_graph_tool(
        fixture.path(),
        "orbit.graph.pack",
        json!({"file":"src/runtime.rs"}),
    );
    assert_eq!(
        file_alias_response["entries"][0]["selector"],
        "file:src/runtime.rs"
    );

    let summary_entry = &summary_response["entries"][0];
    assert_eq!(summary_entry["file"], "src/runtime.rs");
    assert!(summary_entry.get("source").is_none());
    assert!(
        !summary_response
            .to_string()
            .contains("impl AgentRuntime for CodexRuntime")
    );

    let full_response = execute_graph_tool(
        fixture.path(),
        "orbit.graph.pack",
        json!({"selectors":["symbol:src/runtime.rs#AgentRuntimeImpl:impl"],"summary":false}),
    );
    assert_eq!(
        full_response["entries"][0]["source"],
        "impl AgentRuntime for CodexRuntime {\n    fn run(&self) {}\n}\n"
    );
}

#[test]
fn pack_timeout_returns_unresolved_entries_for_unprocessed_selectors() {
    let impl_leaf = leaf_node(
        "src/runtime.rs",
        "AgentRuntimeImpl",
        LeafKind::Impl,
        "impl AgentRuntime for CodexRuntime {\n    fn run(&self) {}\n}\n",
    );
    let fixture = write_graph_fixture(graph_with_root(
        vec![attach_leaf(
            file_node("src/runtime.rs", "rust", Some("rs"), vec![]),
            &impl_leaf,
        )],
        vec![impl_leaf],
    ));

    let response = execute_graph_tool(
        fixture.path(),
        "orbit.graph.pack",
        json!({
            "selectors": [
                "symbol:src/runtime.rs#AgentRuntimeImpl:impl",
                "file:src/runtime.rs"
            ],
            "timeout_ms": 0
        }),
    );

    assert_eq!(response["timeout"]["timeout_ms"], 0);
    assert_eq!(response["timeout"]["processed_selectors"], 0);
    assert_eq!(response["timeout"]["total_selectors"], 2);
    assert_eq!(response["total_nodes"], 0);
    assert_eq!(
        response["unresolved_selectors"].as_array().unwrap().len(),
        2
    );
    assert_eq!(response["entries"].as_array().unwrap().len(), 2);
    assert_eq!(response["entries"][0]["kind"], "unresolved");
    assert!(
        response["entries"][0]["hint"]
            .as_str()
            .unwrap()
            .contains("timed out")
    );
}

#[test]
fn pack_skips_inline_refresh_by_default_with_diagnostic() {
    let impl_leaf = leaf_node(
        "src/runtime.rs",
        "AgentRuntimeImpl",
        LeafKind::Impl,
        "impl AgentRuntime for CodexRuntime {\n    fn run(&self) {}\n}\n",
    );
    let fixture = write_graph_fixture(graph_with_root(
        vec![attach_leaf(
            file_node("src/runtime.rs", "rust", Some("rs"), vec![]),
            &impl_leaf,
        )],
        vec![impl_leaf],
    ));
    init_git_repo(fixture.path());

    let response = execute_graph_tool_unpinned(
        fixture.path(),
        "orbit.graph.pack",
        json!({"selectors":["symbol:src/runtime.rs#AgentRuntimeImpl:impl"]}),
    );

    assert_eq!(response["diagnostics"]["auto_refresh"]["status"], "skipped");
    assert!(
        response["diagnostics"]["auto_refresh"]["remediation"]
            .as_str()
            .unwrap()
            .contains("refresh: true")
    );
    assert_eq!(response["entries"][0]["kind"], "leaf");
}

#[test]
fn pack_rejects_invalid_selector_shapes() {
    let impl_leaf = leaf_node(
        "src/runtime.rs",
        "AgentRuntimeImpl",
        LeafKind::Impl,
        "impl AgentRuntime for CodexRuntime {\n    fn run(&self) {}\n}\n",
    );
    let fixture = write_graph_fixture(graph_with_root(
        vec![attach_leaf(
            file_node("src/runtime.rs", "rust", Some("rs"), vec![]),
            &impl_leaf,
        )],
        vec![impl_leaf],
    ));

    let empty =
        execute_graph_tool_result(fixture.path(), "orbit.graph.pack", json!({"selectors":[]}))
            .unwrap_err()
            .to_string();
    assert!(empty.contains("`selectors` must contain at least one selector"));

    let object_shape = execute_graph_tool_result(
        fixture.path(),
        "orbit.graph.pack",
        json!({"selectors":{"selector":"symbol:src/runtime.rs#AgentRuntimeImpl:impl"}}),
    )
    .unwrap_err()
    .to_string();
    assert!(object_shape.contains("`selectors` must be a string or array of strings"));

    let timeout_shape = execute_graph_tool_result(
        fixture.path(),
        "orbit.graph.pack",
        json!({"selectors":["symbol:src/runtime.rs#AgentRuntimeImpl:impl"],"timeout_ms":"15s"}),
    )
    .unwrap_err()
    .to_string();
    assert!(timeout_shape.contains("`timeout_ms` must be a non-negative integer"));

    let refresh_shape = execute_graph_tool_result(
        fixture.path(),
        "orbit.graph.pack",
        json!({"selectors":["symbol:src/runtime.rs#AgentRuntimeImpl:impl"],"refresh":"yes"}),
    )
    .unwrap_err()
    .to_string();
    assert!(refresh_shape.contains("`refresh` must be a boolean"));
}

#[test]
fn pack_schema_exposes_timeout_and_refresh_controls() {
    let mut registry = ToolRegistry::new();
    registry.register_builtins();

    let schema = registry
        .get_schema("orbit.graph.pack")
        .expect("pack schema registered");
    let timeout = schema
        .parameters
        .iter()
        .find(|param| param.name == "timeout_ms")
        .expect("timeout_ms parameter present");
    assert!(!timeout.required);
    assert_eq!(timeout.param_type, "number");

    let refresh = schema
        .parameters
        .iter()
        .find(|param| param.name == "refresh")
        .expect("refresh parameter present");
    assert!(!refresh.required);
    assert_eq!(refresh.param_type, "boolean");
}

fn execute_graph_tool_unpinned(repo_root: &Path, tool_name: &str, input: Value) -> Value {
    let mut registry = ToolRegistry::new();
    registry.register_builtins();

    registry
        .execute(
            tool_name,
            &ToolContext {
                workspace_root: Some(repo_root.to_path_buf()),
                ..ToolContext::default()
            },
            input,
        )
        .unwrap_or_else(|error| panic!("tool `{tool_name}` failed: {error}"))
}

fn execute_graph_tool(repo_root: &Path, tool_name: &str, input: Value) -> Value {
    execute_graph_tool_result(repo_root, tool_name, input)
        .unwrap_or_else(|error| panic!("tool `{tool_name}` failed: {error}"))
}

fn execute_graph_tool_result(
    repo_root: &Path,
    tool_name: &str,
    input: Value,
) -> Result<Value, OrbitError> {
    let mut registry = ToolRegistry::new();
    registry.register_builtins();

    let mut input = input;
    let object = input.as_object_mut().expect("tool input must be an object");
    object.insert(
        "knowledge_dir".to_string(),
        Value::String(repo_root.join(".orbit/knowledge").display().to_string()),
    );
    object.insert("ref".to_string(), Value::String(GRAPH_REF.to_string()));

    registry.execute(
        tool_name,
        &ToolContext {
            workspace_root: Some(repo_root.to_path_buf()),
            ..ToolContext::default()
        },
        input,
    )
}

fn run_knowledge_build(repo_root: &Path) {
    let config = BuildConfig {
        repo_path: repo_root.to_path_buf(),
        output_dir: repo_root.join(".orbit/knowledge"),
        incremental: false,
        ref_name: Some(RefName::new(GRAPH_REF).unwrap()),
        task_id_pattern: None,
    };
    orbit_knowledge::pipeline::run_build(config).unwrap();
}

fn write_repo_file(repo_root: &Path, rel: &str, content: &str) {
    let path = repo_root.join(rel);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(path, content).unwrap();
}

fn init_git_repo(repo_root: &Path) {
    let output = Command::new("git")
        .args(["init", "-b", GRAPH_REF])
        .current_dir(repo_root)
        .output()
        .expect("run git init");
    assert!(
        output.status.success(),
        "git init failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn write_graph_fixture(graph: CodebaseGraphV1) -> TempDir {
    let repo_root = tempfile::tempdir().unwrap();
    let knowledge_dir = repo_root.path().join(".orbit/knowledge");
    fs::create_dir_all(&knowledge_dir).unwrap();
    fs::write(
        knowledge_dir.join("manifest.json"),
        "{\n  \"generated_at\": \"2026-04-23T00:00:00Z\"\n}\n",
    )
    .unwrap();

    let store = GraphObjectStore::new(knowledge_dir.join("graph"));
    let current_ref = store.write_graph(&graph).unwrap();
    store
        .write_ref_atomic(&RefName::new(GRAPH_REF).unwrap(), &current_ref)
        .unwrap();

    repo_root
}

fn graph_with_root(files: Vec<FileNode>, leaves: Vec<LeafNode>) -> CodebaseGraphV1 {
    let root_id = "dir:.".to_string();
    CodebaseGraphV1 {
        root_dir_id: root_id.clone(),
        dirs: vec![DirNode {
            base: base_node(&root_id, ".", ".", "", None),
            dir_children: Vec::new(),
            file_children: files.iter().map(|file| file.base.id.clone()).collect(),
        }],
        files,
        leaves,
    }
}

fn attach_leaf(mut file: FileNode, leaf: &LeafNode) -> FileNode {
    file.leaf_children.push(leaf.base.id.clone());
    file
}

fn file_node(path: &str, language: &str, extension: Option<&str>, imports: Vec<&str>) -> FileNode {
    let id = format!("file:{path}");
    FileNode {
        base: base_node(
            &id,
            Path::new(path)
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or(path),
            path,
            language,
            Some("dir:."),
        ),
        extension: extension.map(str::to_string),
        source_blob_hash: None,
        source: String::new(),
        imports: imports.into_iter().map(str::to_string).collect(),
        exports: Vec::new(),
        re_exports: Vec::new(),
        leaf_children: Vec::new(),
    }
}

fn leaf_node(path: &str, name: &str, kind: LeafKind, source: &str) -> LeafNode {
    let kind_name = kind.to_string();
    let id = format!("symbol:{path}#{name}:{kind_name}");
    LeafNode {
        base: base_node(
            &id,
            name,
            &format!("{path}#{name}"),
            file_language(path),
            Some(&format!("file:{path}")),
        ),
        kind,
        source: source.to_string(),
        source_blob_hash: None,
        source_hash: None,
        file_hash_at_capture: None,
        history: Vec::new(),
        input_signature: Vec::new(),
        output_signature: Vec::new(),
        start_line: Some(1),
        end_line: Some(3),
        children: Vec::new(),
    }
}

fn file_language(path: &str) -> &'static str {
    match Path::new(path).extension().and_then(|ext| ext.to_str()) {
        Some("md") => "markdown",
        Some("yaml") | Some("yml") => "yaml",
        _ => "rust",
    }
}

fn base_node(
    id: &str,
    name: &str,
    location: &str,
    language: &str,
    parent_id: Option<&str>,
) -> BaseNodeFields {
    BaseNodeFields {
        id: id.to_string(),
        identity_key: id.to_string(),
        object_hash: None,
        name: name.to_string(),
        location: location.to_string(),
        language: language.to_string(),
        description: String::new(),
        parent_id: parent_id.map(str::to_string),
        is_locked: false,
        lineage_locked: false,
        lock_owner: None,
        lock_reason: String::new(),
        task_ids: Vec::new(),
        structural_conflict: false,
    }
}
