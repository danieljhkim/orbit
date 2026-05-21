#![allow(missing_docs)]

use crate::graph::nodes::{BaseNodeFields, CodebaseGraphV1, DirNode, FileNode, LeafKind, LeafNode};

use super::*;

struct LanguageCase {
    path: &'static str,
    language: &'static str,
    name: &'static str,
    kind: LeafKind,
    source: &'static str,
    pattern: &'static str,
    start_line: u32,
}

#[test]
fn source_regex_matches_leaf_source_for_indexed_languages() {
    let cases = vec![
        LanguageCase {
            path: "src/lib.rs",
            language: "rust",
            name: "build",
            kind: LeafKind::Function,
            source: "pub const fn build() {}\n",
            pattern: r"^\s*pub\s+const\s+fn\s+build",
            start_line: 7,
        },
        LanguageCase {
            path: "go/main.go",
            language: "go",
            name: "Build",
            kind: LeafKind::Function,
            source: "func Build() {}\n",
            pattern: r"^\s*func\s+Build",
            start_line: 11,
        },
        LanguageCase {
            path: "java/Sink.java",
            language: "java",
            name: "Sink",
            kind: LeafKind::Class,
            source: "class Sink implements ISink {}\n",
            pattern: r"class\s+Sink\s+implements\s+ISink",
            start_line: 13,
        },
        LanguageCase {
            path: "web/sink.js",
            language: "javascript",
            name: "Sink",
            kind: LeafKind::Class,
            source: "class Sink extends BaseSink {}\n",
            pattern: r"class\s+Sink\s+extends\s+BaseSink",
            start_line: 17,
        },
        LanguageCase {
            path: "py/sink.py",
            language: "python",
            name: "Sink",
            kind: LeafKind::Class,
            source: "class Sink(ISink):\n    pass\n",
            pattern: r"^\s*class\s+Sink\(ISink\):",
            start_line: 19,
        },
    ];

    for case in cases {
        let graph = graph_for_case(&case);
        let service = GraphContextService::new(&graph);
        let regex = Regex::new(case.pattern).unwrap();
        let (total, hits) = service.search_hits_with_total(
            "",
            Some(&["symbol"]),
            Some(case.path),
            None,
            Some(&regex),
            10,
        );

        assert_eq!(total, 1, "expected one match for {}", case.language);
        assert_eq!(
            hits.len(),
            1,
            "expected one returned hit for {}",
            case.language
        );
        assert_eq!(hits[0].node.base().language, case.language);
        assert_eq!(
            hits[0].matched_lines[0].line_number,
            case.start_line as usize
        );
    }
}

fn graph_for_case(case: &LanguageCase) -> CodebaseGraphV1 {
    let root_id = "dir:.".to_string();
    let file_id = format!("file:{}", case.path);
    let kind_name = case.kind.to_string();
    let leaf_id = format!("symbol:{}#{}:{kind_name}", case.path, case.name);

    CodebaseGraphV1 {
        root_dir_id: root_id.clone(),
        dirs: vec![DirNode {
            base: base_node(&root_id, ".", ".", "", None),
            dir_children: Vec::new(),
            file_children: vec![file_id.clone()],
        }],
        files: vec![FileNode {
            base: base_node(&file_id, case.path, case.path, case.language, Some("dir:.")),
            extension: std::path::Path::new(case.path)
                .extension()
                .and_then(|ext| ext.to_str())
                .map(str::to_string),
            source_blob_hash: None,
            source: String::new(),
            imports: Vec::new(),
            exports: Vec::new(),
            re_exports: Vec::new(),
            leaf_children: vec![leaf_id.clone()],
        }],
        leaves: vec![LeafNode {
            base: base_node(
                &leaf_id,
                case.name,
                &format!("{}#{}", case.path, case.name),
                case.language,
                Some(&file_id),
            ),
            kind: case.kind.clone(),
            source: case.source.to_string(),
            source_blob_hash: None,
            source_hash: None,
            file_hash_at_capture: None,
            history: Vec::new(),
            input_signature: Vec::new(),
            output_signature: Vec::new(),
            start_line: Some(case.start_line),
            end_line: Some(case.start_line + case.source.lines().count() as u32 - 1),
            children: Vec::new(),
        }],
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
    }
}
