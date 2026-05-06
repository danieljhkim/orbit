use std::collections::BTreeMap;
use std::path::Path;

use orbit_knowledge::extract::{FileKind, Language, extract_file};
use orbit_knowledge::graph::GraphNodeRef;
use orbit_knowledge::graph::object_store::RefName;
use orbit_knowledge::pipeline::context::BuildConfig;
use orbit_knowledge::service::GraphContextService;
use orbit_knowledge::{KnowledgeStore, Selector};
use tempfile::tempdir;

const TS_SOURCE: &str = "export function buildThing(value: string): string {\n\
    return value;\n\
}\n\
\n\
export const makeThing = (name: string) => ({ name });\n\
export let runThing = function runThing(value: number) {\n\
    return value;\n\
};\n\
\n\
export class Worker {\n\
    start(): void {\n\
    }\n\
\n\
    async stop(): Promise<void> {\n\
    }\n\
}\n\
\n\
export interface WorkerConfig {\n\
    enabled: boolean;\n\
}\n\
\n\
export type WorkerState = \"idle\" | \"running\";\n\
export enum WorkerMode {\n\
    Fast,\n\
    Safe,\n\
}\n";

const TSX_SOURCE: &str = "export const Card = ({ title }: { title: string }) => (\n\
    <section>{title}</section>\n\
);\n\
\n\
export function Shell() {\n\
    return <Card title=\"ok\" />;\n\
}\n";

#[test]
fn file_kind_classifies_typescript_and_tsx_extensions() {
    assert_eq!(
        FileKind::from_extension("ts"),
        FileKind::Code(Language::TypeScript)
    );
    assert_eq!(
        FileKind::from_extension("mts"),
        FileKind::Code(Language::TypeScript)
    );
    assert_eq!(
        FileKind::from_extension("cts"),
        FileKind::Code(Language::TypeScript)
    );
    assert_eq!(
        FileKind::from_extension("tsx"),
        FileKind::Code(Language::Tsx)
    );
    assert_eq!(FileKind::from_extension("ts").as_str(), "typescript");
    assert_eq!(FileKind::from_extension("tsx").as_str(), "tsx");
    assert_eq!(
        FileKind::from_extension("js"),
        FileKind::Code(Language::JavaScript)
    );
    assert_eq!(FileKind::from_extension("js").as_str(), "javascript");
}

#[test]
fn extractor_emits_typescript_and_tsx_symbols() {
    let result = extract_file(TS_SOURCE, Language::TypeScript);
    let leaves = leaves_by_qualified_name(&result.leaves);

    assert_leaf(&leaves, "buildThing", "buildThing", "function", 1, 3);
    assert_leaf(&leaves, "makeThing", "makeThing", "function", 5, 5);
    assert_leaf(&leaves, "runThing", "runThing", "function", 6, 8);
    assert_leaf(&leaves, "Worker::start", "start", "method", 11, 12);
    assert_leaf(&leaves, "Worker::stop", "stop", "method", 14, 15);
    assert_leaf(&leaves, "Worker", "Worker", "class", 10, 16);
    assert_leaf(&leaves, "WorkerConfig", "WorkerConfig", "interface", 18, 20);
    assert_leaf(&leaves, "WorkerState", "WorkerState", "type_alias", 22, 22);
    assert_leaf(&leaves, "WorkerMode", "WorkerMode", "enum", 23, 26);

    let result = extract_file(TSX_SOURCE, Language::Tsx);
    let leaves = leaves_by_qualified_name(&result.leaves);
    assert_leaf(&leaves, "Card", "Card", "function", 1, 3);
    assert_leaf(&leaves, "Shell", "Shell", "function", 5, 7);
}

#[test]
fn pipeline_search_and_pack_return_typescript_fixture_symbols() {
    let repo_dir = tempdir().expect("repo tempdir");
    let repo = repo_dir.path();
    write_file(repo, "src/types.ts", TS_SOURCE);
    write_file(repo, "src/view.tsx", TSX_SOURCE);
    write_file(
        repo,
        "src/contracts.d.ts",
        "export declare function declaredCall(input: string): void;\n\
export interface DeclaredShape {\n\
    id: string;\n\
}\n",
    );

    let knowledge_root = tempdir().expect("knowledge tempdir");
    let output_dir = knowledge_root.path().join("knowledge");
    let ctx = run_build(repo, &output_dir);

    let types_file = ctx
        .graph
        .files
        .iter()
        .find(|file| file.base.location == "src/types.ts")
        .expect("types.ts file node");
    assert_eq!(types_file.base.language, "typescript");
    let tsx_file = ctx
        .graph
        .files
        .iter()
        .find(|file| file.base.location == "src/view.tsx")
        .expect("view.tsx file node");
    assert_eq!(tsx_file.base.language, "tsx");
    let declaration_file = ctx
        .graph
        .files
        .iter()
        .find(|file| file.base.location == "src/contracts.d.ts")
        .expect("contracts.d.ts file node");
    assert_eq!(declaration_file.base.language, "typescript");

    let service = GraphContextService::new(&ctx.graph);
    let results = service.search_structured(
        "WorkerState",
        Some(&["symbol"]),
        Some("src/types.ts"),
        None,
        10,
    );
    assert_eq!(results.len(), 1);
    assert_eq!(
        results[0].selector,
        "symbol:src/types.ts#WorkerState:type_alias"
    );
    assert_eq!(results[0].kind, "type_alias");

    let expected: [(&str, &str, u32, u32); 13] = [
        ("symbol:src/types.ts#buildThing:function", "function", 1, 3),
        ("symbol:src/types.ts#makeThing:function", "function", 5, 5),
        ("symbol:src/types.ts#runThing:function", "function", 6, 8),
        ("symbol:src/types.ts#Worker::start:method", "method", 11, 12),
        ("symbol:src/types.ts#Worker::stop:method", "method", 14, 15),
        ("symbol:src/types.ts#Worker:class", "class", 10, 16),
        (
            "symbol:src/types.ts#WorkerConfig:interface",
            "interface",
            18,
            20,
        ),
        (
            "symbol:src/types.ts#WorkerState:type_alias",
            "type_alias",
            22,
            22,
        ),
        ("symbol:src/types.ts#WorkerMode:enum", "enum", 23, 26),
        ("symbol:src/view.tsx#Card:function", "function", 1, 3),
        ("symbol:src/view.tsx#Shell:function", "function", 5, 7),
        (
            "symbol:src/contracts.d.ts#declaredCall:function",
            "function",
            1,
            1,
        ),
        (
            "symbol:src/contracts.d.ts#DeclaredShape:interface",
            "interface",
            2,
            4,
        ),
    ];

    for (selector, kind, start_line, end_line) in expected {
        let selector: Selector = selector.parse().expect("selector parses");
        let node = service
            .resolve_selector(&selector)
            .expect("selector resolves");
        let GraphNodeRef::Leaf(leaf) = node else {
            panic!("selector {selector} did not resolve to a leaf");
        };
        assert_eq!(leaf.kind.to_string(), kind);
        assert_eq!(leaf.start_line, Some(start_line));
        assert_eq!(leaf.end_line, Some(end_line));
        assert!(!leaf.source.is_empty());
    }

    let store = KnowledgeStore::open(
        &output_dir,
        &RefName::new("main").expect("valid ref"),
        None,
        None,
    )
    .expect("knowledge store opens");
    let pack = store
        .pack(&["symbol:src/types.ts#WorkerState:type_alias"
            .parse()
            .expect("selector parses")])
        .expect("pack succeeds");
    assert_eq!(pack.total_nodes, 1);
    assert_eq!(pack.entries[0].name.as_deref(), Some("WorkerState"));
    assert_eq!(pack.entries[0].language.as_deref(), Some("typescript"));
    assert_eq!(pack.entries[0].start_line, Some(22));
    assert!(
        pack.entries[0]
            .source
            .as_deref()
            .expect("leaf source is included")
            .contains("type WorkerState")
    );
}

fn run_build(
    repo: &Path,
    output_dir: &Path,
) -> orbit_knowledge::pipeline::context::PipelineContext {
    let config = BuildConfig {
        repo_path: repo.to_path_buf(),
        output_dir: output_dir.to_path_buf(),
        incremental: false,
        ref_name: Some(RefName::new("main").expect("valid ref")),
    };
    orbit_knowledge::pipeline::run_build(config).expect("pipeline runs")
}

fn write_file(repo: &Path, rel: &str, content: &str) {
    let path = repo.join(rel);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).expect("create parent dir");
    }
    std::fs::write(path, content).expect("write fixture file");
}

fn leaves_by_qualified_name<'a>(
    leaves: &'a [orbit_knowledge::extract::ExtractedLeaf],
) -> BTreeMap<&'a str, &'a orbit_knowledge::extract::ExtractedLeaf> {
    leaves
        .iter()
        .map(|leaf| (leaf.qualified_name.as_str(), leaf))
        .collect()
}

fn assert_leaf(
    leaves: &BTreeMap<&str, &orbit_knowledge::extract::ExtractedLeaf>,
    qualified_name: &str,
    name: &str,
    kind: &str,
    start_line: usize,
    end_line: usize,
) {
    let leaf = leaves
        .get(qualified_name)
        .unwrap_or_else(|| panic!("missing leaf {qualified_name}"));
    assert_eq!(leaf.name, name);
    assert_eq!(leaf.kind, kind);
    assert_eq!(leaf.start_line, start_line);
    assert_eq!(leaf.end_line, end_line);
    assert!(!leaf.source.is_empty());
}
