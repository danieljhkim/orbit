use std::fs;
use std::path::Path;

use orbit_knowledge::graph::object_store::RefName;
use orbit_knowledge::graph::{
    BaseNodeFields, CodebaseGraphV1, DirNode, FileNode, GraphObjectStore, LeafKind, LeafNode,
};
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

fn execute_graph_tool(repo_root: &Path, tool_name: &str, input: Value) -> Value {
    let mut registry = ToolRegistry::new();
    registry.register_builtins();

    let mut input = input;
    let object = input.as_object_mut().expect("tool input must be an object");
    object.insert(
        "knowledge_dir".to_string(),
        Value::String(repo_root.join(".orbit/knowledge").display().to_string()),
    );
    object.insert("ref".to_string(), Value::String(GRAPH_REF.to_string()));

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
        imports: imports.into_iter().map(str::to_string).collect(),
        exports: Vec::new(),
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
