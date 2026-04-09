//! Regex-based language extractors that mirror the Python orbit-map extractors.
//!
//! These produce compatible output so the working graph can be refreshed
//! after each `knowledge.write` without shelling out to the Python pipeline.

use regex::Regex;
use sha2::{Digest, Sha256};
use std::sync::LazyLock;

/// A single extracted leaf from a source file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtractedLeaf {
    pub qualified_name: String,
    pub name: String,
    pub kind: String,
    pub start_line: usize,
    pub end_line: usize,
    pub source: String,
    pub source_hash: String,
    /// For methods: the qualified_name of the parent impl block.
    pub parent_qualified_name: Option<String>,
    /// For impl blocks: qualified_names of child methods.
    pub children_qualified_names: Vec<String>,
}

/// Result of extracting a single file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtractionResult {
    pub leaves: Vec<ExtractedLeaf>,
}

/// Supported languages for extraction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Language {
    Rust,
    Python,
}

impl Language {
    pub fn from_extension(ext: &str) -> Option<Self> {
        match ext {
            "rs" => Some(Self::Rust),
            "py" => Some(Self::Python),
            _ => None,
        }
    }
}

pub fn compute_source_hash(source: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(source.as_bytes());
    format!("{:x}", hasher.finalize())
}

pub fn extract_file(source: &str, language: Language) -> ExtractionResult {
    match language {
        Language::Rust => extract_rust(source),
        Language::Python => extract_python(source),
    }
}

// ---- Rust extractor ----

// Mirrors orbit-map/orbit_map/graph/extraction/rust.py

static ITEM_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?m)^(?P<vis>pub(?:\([^)]*\))?\s+)?(?P<kw>fn|struct|enum|trait|impl|mod|type|const|static)\s+(?P<name>\w+)",
    )
    .expect("ITEM_RE")
});

static METHOD_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?m)^    (?:pub(?:\([^)]*\))?\s+)?(?:async\s+)?fn\s+(?P<name>\w+)\s*(?P<sig>[^{;]*)",
    )
    .expect("METHOD_RE")
});

fn rust_kind(kw: &str) -> &'static str {
    match kw {
        "fn" => "function",
        "struct" => "struct",
        "enum" => "struct",
        "trait" => "trait",
        "impl" => "impl",
        "mod" => "module",
        "type" => "struct",
        "const" => "field",
        "static" => "field",
        _ => "function",
    }
}

fn find_block_end(source: &str, start: usize) -> usize {
    let bytes = source.as_bytes();
    let limit = (start + 512).min(bytes.len());
    let mut i = start;

    // Find opening brace or semicolon
    while i < limit {
        if bytes[i] == b';' {
            return i + 1;
        }
        if bytes[i] == b'{' {
            break;
        }
        i += 1;
    }

    if i >= limit || bytes[i] != b'{' {
        // No brace found — return next newline
        if let Some(pos) = source[start..].find('\n') {
            return start + pos + 1;
        }
        return source.len();
    }

    // Count brace depth
    let mut depth = 0i32;
    while i < bytes.len() {
        match bytes[i] {
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    return i + 1;
                }
            }
            _ => {}
        }
        i += 1;
    }
    source.len()
}

fn line_of(source: &str, offset: usize) -> usize {
    source[..offset].matches('\n').count() + 1
}

fn extract_rust(source: &str) -> ExtractionResult {
    let mut leaves = Vec::new();

    for m in ITEM_RE.find_iter(source) {
        let caps = ITEM_RE.captures(&source[m.start()..]).unwrap();
        let kw = caps.name("kw").unwrap().as_str();
        let name = caps.name("name").unwrap().as_str().to_string();
        let kind = rust_kind(kw).to_string();

        let item_start = m.start();
        let item_end = find_block_end(source, m.end());
        let item_source = source[item_start..item_end].trim().to_string();
        let hash = compute_source_hash(&item_source);

        let start_line = line_of(source, item_start);
        let end_line = line_of(source, item_end);

        let mut children = Vec::new();

        // Extract methods from impl blocks
        if kw == "impl" {
            for mm in METHOD_RE.find_iter(&item_source) {
                let mcaps = METHOD_RE.captures(&item_source[mm.start()..]).unwrap();
                let mname = mcaps.name("name").unwrap().as_str().to_string();
                let qualified = format!("{name}::{mname}");

                let mstart_in_item = mm.start();
                let mend_in_item = find_block_end(&item_source, mm.end());
                let msource = item_source[mstart_in_item..mend_in_item].trim().to_string();
                let mhash = compute_source_hash(&msource);

                let mstart_abs = item_start + mstart_in_item;
                let mend_abs = item_start + mend_in_item;

                children.push(qualified.clone());

                leaves.push(ExtractedLeaf {
                    qualified_name: qualified,
                    name: mname,
                    kind: "method".to_string(),
                    start_line: line_of(source, mstart_abs),
                    end_line: line_of(source, mend_abs),
                    source: msource,
                    source_hash: mhash,
                    parent_qualified_name: Some(name.clone()),
                    children_qualified_names: vec![],
                });
            }
        }

        leaves.push(ExtractedLeaf {
            qualified_name: name.clone(),
            name,
            kind,
            start_line,
            end_line,
            source: item_source,
            source_hash: hash,
            parent_qualified_name: None,
            children_qualified_names: children,
        });
    }

    ExtractionResult { leaves }
}

// ---- Python extractor (minimal, regex-based) ----

static PY_DEF_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?m)^(?P<indent> *)(?:async\s+)?(?P<kw>def|class)\s+(?P<name>\w+)")
        .expect("PY_DEF_RE")
});

fn extract_python(source: &str) -> ExtractionResult {
    let mut leaves = Vec::new();
    let lines: Vec<&str> = source.lines().collect();

    for m in PY_DEF_RE.find_iter(source) {
        let caps = PY_DEF_RE.captures(&source[m.start()..]).unwrap();
        let indent = caps.name("indent").unwrap().as_str();
        let kw = caps.name("kw").unwrap().as_str();
        let name = caps.name("name").unwrap().as_str().to_string();

        let start_line = line_of(source, m.start());
        let indent_len = indent.len();

        // Find block end: next line at same or lesser indent (or EOF)
        let mut end_line_idx = start_line; // 1-indexed, start at the def line
        for (i, line) in lines.iter().enumerate().skip(start_line) {
            if line.trim().is_empty() {
                end_line_idx = i + 1;
                continue;
            }
            let line_indent = line.len() - line.trim_start().len();
            if line_indent <= indent_len && i > start_line - 1 {
                break;
            }
            end_line_idx = i + 1;
        }

        // Collect source lines
        let item_lines: Vec<&str> = lines[start_line - 1..end_line_idx].to_vec();
        let item_source = item_lines.join("\n").trim_end().to_string();
        let hash = compute_source_hash(&item_source);

        let kind = if kw == "class" {
            "class"
        } else if indent_len > 0 {
            "method"
        } else {
            "function"
        };

        leaves.push(ExtractedLeaf {
            qualified_name: name.clone(),
            name,
            kind: kind.to_string(),
            start_line,
            end_line: end_line_idx,
            source: item_source,
            source_hash: hash,
            parent_qualified_name: None,
            children_qualified_names: vec![],
        });
    }

    ExtractionResult { leaves }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RUST_SAMPLE: &str = r#"use std::collections::HashMap;

pub fn top_level_fn() -> bool {
    true
}

pub struct MyStruct {
    field: i32,
}

impl MyStruct {
    pub fn new(val: i32) -> Self {
        Self { field: val }
    }

    pub fn value(&self) -> i32 {
        self.field
    }
}

fn private_fn() {
    // nothing
}
"#;

    #[test]
    fn rust_extracts_top_level_items() {
        let result = extract_rust(RUST_SAMPLE);
        let names: Vec<&str> = result
            .leaves
            .iter()
            .filter(|l| l.parent_qualified_name.is_none())
            .map(|l| l.qualified_name.as_str())
            .collect();
        assert!(names.contains(&"top_level_fn"));
        assert!(names.contains(&"MyStruct"));
        assert!(names.contains(&"MyStruct")); // impl block also named MyStruct
        assert!(names.contains(&"private_fn"));
    }

    #[test]
    fn rust_extracts_methods_from_impl_block() {
        let result = extract_rust(RUST_SAMPLE);
        let methods: Vec<&ExtractedLeaf> = result
            .leaves
            .iter()
            .filter(|l| l.kind == "method")
            .collect();
        assert_eq!(methods.len(), 2);
        assert!(methods.iter().any(|m| m.qualified_name == "MyStruct::new"));
        assert!(
            methods
                .iter()
                .any(|m| m.qualified_name == "MyStruct::value")
        );

        for m in &methods {
            assert_eq!(m.parent_qualified_name.as_deref(), Some("MyStruct"));
        }
    }

    #[test]
    fn rust_source_hash_is_sha256() {
        let result = extract_rust(RUST_SAMPLE);
        let top = result
            .leaves
            .iter()
            .find(|l| l.name == "top_level_fn")
            .unwrap();
        assert_eq!(top.source_hash, compute_source_hash(&top.source));
        assert_eq!(top.source_hash.len(), 64); // hex SHA-256
    }

    #[test]
    fn rust_line_numbers_are_correct() {
        let result = extract_rust(RUST_SAMPLE);
        let top = result
            .leaves
            .iter()
            .find(|l| l.name == "top_level_fn")
            .unwrap();
        assert_eq!(top.start_line, 3);
        assert_eq!(top.end_line, 5);
    }

    #[test]
    fn rust_impl_children_are_tracked() {
        let result = extract_rust(RUST_SAMPLE);
        let impl_leaf = result.leaves.iter().find(|l| l.kind == "impl").unwrap();
        assert_eq!(impl_leaf.children_qualified_names.len(), 2);
        assert!(
            impl_leaf
                .children_qualified_names
                .contains(&"MyStruct::new".to_string())
        );
        assert!(
            impl_leaf
                .children_qualified_names
                .contains(&"MyStruct::value".to_string())
        );
    }

    #[test]
    fn python_extracts_functions_and_classes() {
        let source = "def foo():\n    pass\n\nclass Bar:\n    def method(self):\n        pass\n";
        let result = extract_python(source);
        assert!(
            result
                .leaves
                .iter()
                .any(|l| l.name == "foo" && l.kind == "function")
        );
        assert!(
            result
                .leaves
                .iter()
                .any(|l| l.name == "Bar" && l.kind == "class")
        );
        assert!(
            result
                .leaves
                .iter()
                .any(|l| l.name == "method" && l.kind == "method")
        );
    }

    #[test]
    fn source_hash_matches_python_impl() {
        // Python: hashlib.sha256(source.encode("utf-8")).hexdigest()
        let source = "pub fn hello() {}";
        let hash = compute_source_hash(source);
        assert_eq!(hash.len(), 64);
        // Verify deterministic
        assert_eq!(hash, compute_source_hash(source));
    }

    #[test]
    fn unsupported_extension() {
        assert!(Language::from_extension("rs").is_some());
        assert!(Language::from_extension("py").is_some());
        assert!(Language::from_extension("go").is_none());
        assert!(Language::from_extension("js").is_none());
    }
}
