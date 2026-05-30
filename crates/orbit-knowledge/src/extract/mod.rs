//! File content extractors for the knowledge graph.
//!
//! `orbit-graph-extract` owns the extractor implementations. This module keeps
//! the orbit-knowledge compatibility contract by translating those raw graph
//! rows back into the legacy `ExtractionResult` shape consumed by the existing
//! pipeline and working-graph code.

mod c;
mod common;
mod config;
mod csharp;
mod go;
mod java;
mod javascript;
mod kotlin;
mod language;
mod markdown;
mod python;
mod ruby;
mod rust;
mod table;
mod typescript;

pub use common::{
    ExtractedExport, ExtractedLeaf, ExtractionResult, compute_source_hash,
    finalize_unique_qualified_names, identity_key, leaf_location, node_id,
};
pub use language::{ConfigFormat, DocFormat, FileKind, Language, TableFormat};

use c::CExtractor;
use csharp::CSharpExtractor;
use go::GoExtractor;
use java::JavaExtractor;
use javascript::JavaScriptExtractor;
use kotlin::KotlinExtractor;
use markdown::MarkdownExtractor;
use orbit_graph_extract::{
    ExtractedFile as GraphExtractedFile, Extractor as GraphExtractor, RawSymbol,
};
use python::PythonExtractor;
use ruby::RubyExtractor;
use rust::RustExtractor;
use std::collections::{HashMap, HashSet};
use std::path::Path;
use table::TableExtractor;
use tree_sitter::{Node, Parser};
use typescript::TypeScriptExtractor;

/// Trait for file-content extractors.
///
/// Implementors declare the `FileKind` they handle and emit a flat list of
/// `ExtractedLeaf` anchors for that file's content. For code, anchors are
/// symbols; for markdown, anchors are sections. Config and table extractors
/// stay registered to preserve file-level source capture but emit no leaves.
pub trait FileExtractor: Send + Sync {
    fn file_kind(&self) -> FileKind;
    fn extract(&self, source: &str) -> ExtractionResult;
}

struct GraphConfigExtractor {
    format: ConfigFormat,
}

impl GraphConfigExtractor {
    fn new(format: ConfigFormat) -> Self {
        Self { format }
    }
}

impl FileExtractor for GraphConfigExtractor {
    fn file_kind(&self) -> FileKind {
        FileKind::Config(self.format)
    }

    fn extract(&self, source: &str) -> ExtractionResult {
        let path = match self.format {
            ConfigFormat::Yaml => "memory.yaml",
            ConfigFormat::Json => "memory.json",
            ConfigFormat::Toml => "memory.toml",
        };
        graph_to_knowledge_result(
            self.file_kind(),
            source,
            config::ConfigExtractor.extract(Path::new(path), source.as_bytes()),
        )
    }
}

struct GraphTypeScriptExtractor {
    language: Language,
}

impl GraphTypeScriptExtractor {
    fn new(language: Language) -> Self {
        debug_assert!(matches!(language, Language::TypeScript | Language::Tsx));
        Self { language }
    }
}

impl FileExtractor for GraphTypeScriptExtractor {
    fn file_kind(&self) -> FileKind {
        FileKind::Code(self.language)
    }

    fn extract(&self, source: &str) -> ExtractionResult {
        let path = match self.language {
            Language::Tsx => "memory.tsx",
            _ => "memory.ts",
        };
        graph_to_knowledge_result(
            self.file_kind(),
            source,
            TypeScriptExtractor.extract(Path::new(path), source.as_bytes()),
        )
    }
}

macro_rules! impl_graph_file_extractor {
    ($ty:path, $kind:expr, $path:literal) => {
        impl FileExtractor for $ty {
            fn file_kind(&self) -> FileKind {
                $kind
            }

            fn extract(&self, source: &str) -> ExtractionResult {
                graph_to_knowledge_result(
                    self.file_kind(),
                    source,
                    GraphExtractor::extract(self, Path::new($path), source.as_bytes()),
                )
            }
        }
    };
}

impl_graph_file_extractor!(CExtractor, FileKind::Code(Language::C), "memory.c");
impl_graph_file_extractor!(
    CSharpExtractor,
    FileKind::Code(Language::CSharp),
    "memory.cs"
);
impl_graph_file_extractor!(RustExtractor, FileKind::Code(Language::Rust), "memory.rs");
impl_graph_file_extractor!(
    PythonExtractor,
    FileKind::Code(Language::Python),
    "memory.py"
);
impl_graph_file_extractor!(RubyExtractor, FileKind::Code(Language::Ruby), "memory.rb");
impl_graph_file_extractor!(GoExtractor, FileKind::Code(Language::Go), "memory.go");
impl_graph_file_extractor!(JavaExtractor, FileKind::Code(Language::Java), "memory.java");
impl_graph_file_extractor!(
    JavaScriptExtractor,
    FileKind::Code(Language::JavaScript),
    "memory.js"
);
impl_graph_file_extractor!(
    KotlinExtractor,
    FileKind::Code(Language::Kotlin),
    "memory.kt"
);
impl_graph_file_extractor!(
    MarkdownExtractor,
    FileKind::Doc(DocFormat::Markdown),
    "memory.md"
);

/// Registry of available extractors.
pub struct ExtractorRegistry {
    extractors: Vec<Box<dyn FileExtractor>>,
}

impl ExtractorRegistry {
    pub fn new() -> Self {
        Self {
            extractors: vec![
                Box::new(CExtractor),
                Box::new(CSharpExtractor),
                Box::new(RustExtractor),
                Box::new(PythonExtractor),
                Box::new(RubyExtractor),
                Box::new(GoExtractor),
                Box::new(JavaExtractor),
                Box::new(JavaScriptExtractor),
                Box::new(KotlinExtractor),
                Box::new(GraphTypeScriptExtractor::new(Language::TypeScript)),
                Box::new(GraphTypeScriptExtractor::new(Language::Tsx)),
                Box::new(MarkdownExtractor),
                Box::new(GraphConfigExtractor::new(ConfigFormat::Yaml)),
                Box::new(GraphConfigExtractor::new(ConfigFormat::Json)),
                Box::new(GraphConfigExtractor::new(ConfigFormat::Toml)),
                Box::new(TableExtractor::new(TableFormat::Csv)),
                Box::new(TableExtractor::new(TableFormat::Tsv)),
            ],
        }
    }

    pub fn get(&self, kind: FileKind) -> Option<&dyn FileExtractor> {
        self.extractors
            .iter()
            .find(|e| e.file_kind() == kind)
            .map(|e| e.as_ref())
    }
}

impl Default for ExtractorRegistry {
    fn default() -> Self {
        Self::new()
    }
}

/// Extract leaves from source using the code extractor for `language`.
///
/// Thin compatibility shim for working-graph callers that dispatch on a
/// `Language` directly (not on a `FileKind`). Non-code files do not reach
/// this path — the pipeline layer dispatches by `FileKind`.
pub fn extract_file(source: &str, language: Language) -> ExtractionResult {
    let registry = ExtractorRegistry::new();
    registry
        .get(FileKind::Code(language))
        .map(|e| e.extract(source))
        .unwrap_or_default()
}

// ORB-00306: coexistence scaffolding. orbit-knowledge still consumes its legacy
// ExtractionResult while orbit-graph-extract owns parsing; if orbit-knowledge is
// ever phased out per GRAPH_SPEC.md §16 Step 4, this translator is the bridge to
// re-evaluate rather than a blanket deletion target.
fn graph_to_knowledge_result(
    file_kind: FileKind,
    source: &str,
    extracted: GraphExtractedFile,
) -> ExtractionResult {
    let mut converted = Vec::new();
    let mut translated: Vec<TranslatedSymbol> = extracted
        .symbols
        .iter()
        .filter_map(|symbol| translate_symbol(file_kind, source, symbol))
        .collect();
    translated.extend(legacy_synthesized_symbols(file_kind, source, &translated));

    let translated_by_qualified: HashMap<String, usize> = translated
        .iter()
        .enumerate()
        .map(|(index, symbol)| (symbol.original_qualified.clone(), index))
        .collect();
    let qualified_by_original: HashMap<String, String> = translated
        .iter()
        .map(|symbol| {
            (
                symbol.original_qualified.clone(),
                symbol.qualified_name.clone(),
            )
        })
        .collect();
    let child_names_by_parent = child_names_by_parent(&translated, &translated_by_qualified);
    let start_line_by_qualified: HashMap<String, usize> = translated
        .iter()
        .map(|symbol| (symbol.qualified_name.clone(), symbol.start_line))
        .collect();
    let parent_originals: HashSet<String> = child_names_by_parent.keys().cloned().collect();

    for symbol in translated {
        let parent_qualified_name = symbol
            .original_parent
            .as_deref()
            .and_then(|parent| qualified_by_original.get(parent).cloned())
            .or(symbol.original_parent);
        let children_qualified_names = child_names_by_parent
            .get(symbol.original_qualified.as_str())
            .cloned()
            .unwrap_or_default();

        let is_parent = parent_originals.contains(&symbol.original_qualified);
        converted.push(OrderedLeaf {
            order_line: if is_parent {
                children_qualified_names
                    .iter()
                    .filter_map(|child| start_line_by_qualified.get(child).copied())
                    .max()
                    .unwrap_or(symbol.start_line)
            } else {
                symbol.start_line
            },
            parent_rank: usize::from(is_parent),
            leaf: ExtractedLeaf {
                qualified_name: symbol.qualified_name,
                name: symbol.name,
                kind: symbol.kind,
                start_line: symbol.start_line,
                end_line: symbol.end_line,
                source_hash: compute_source_hash(&symbol.source),
                source: symbol.source,
                parent_qualified_name,
                children_qualified_names,
                depth: symbol.depth,
            },
        });
    }

    converted.sort_by(|left, right| {
        left.order_line
            .cmp(&right.order_line)
            .then_with(|| left.parent_rank.cmp(&right.parent_rank))
            .then_with(|| left.leaf.start_line.cmp(&right.leaf.start_line))
            .then_with(|| left.leaf.end_line.cmp(&right.leaf.end_line))
            .then_with(|| left.leaf.qualified_name.cmp(&right.leaf.qualified_name))
            .then_with(|| left.leaf.kind.cmp(&right.leaf.kind))
    });
    let mut leaves: Vec<ExtractedLeaf> = converted.into_iter().map(|entry| entry.leaf).collect();
    if file_kind == FileKind::Doc(DocFormat::Markdown) {
        adjust_markdown_sections(&mut leaves, source);
    }
    finalize_unique_qualified_names(&mut leaves);

    ExtractionResult {
        leaves,
        exports: if file_kind == FileKind::Code(Language::Rust) {
            rust_exports(source)
        } else {
            Vec::new()
        },
    }
}

fn child_names_by_parent(
    translated: &[TranslatedSymbol],
    translated_by_qualified: &HashMap<String, usize>,
) -> HashMap<String, Vec<String>> {
    let mut children: HashMap<String, Vec<String>> = HashMap::new();
    for symbol in translated {
        let Some(parent) = symbol.original_parent.as_deref() else {
            continue;
        };
        if translated_by_qualified.contains_key(parent) {
            children
                .entry(parent.to_string())
                .or_default()
                .push(symbol.qualified_name.clone());
        }
    }
    children
}

struct OrderedLeaf {
    order_line: usize,
    parent_rank: usize,
    leaf: ExtractedLeaf,
}

struct TranslatedSymbol {
    original_qualified: String,
    original_parent: Option<String>,
    qualified_name: String,
    name: String,
    kind: String,
    start_line: usize,
    end_line: usize,
    source: String,
    depth: Option<u8>,
}

fn translate_symbol(
    file_kind: FileKind,
    source: &str,
    symbol: &RawSymbol,
) -> Option<TranslatedSymbol> {
    if is_legacy_filtered_symbol(file_kind, symbol) {
        return None;
    }

    let mut kind = legacy_kind(file_kind, &symbol.kind)?.to_string();
    if file_kind == FileKind::Code(Language::Kotlin)
        && symbol.kind == "object"
        && symbol.name == "Companion"
        && symbol.parent_symbol.is_some()
    {
        kind = "companion_object".to_string();
    }
    let symbol_source = legacy_symbol_source(file_kind, source, symbol);
    let qualified_name = legacy_qualified_name(file_kind, symbol, &symbol_source);
    Some(TranslatedSymbol {
        original_qualified: symbol.qualified.clone(),
        original_parent: symbol.parent_symbol.clone(),
        qualified_name,
        name: symbol.name.clone(),
        kind,
        start_line: line_for_byte(source, symbol.span_start),
        end_line: line_for_byte(source, symbol.span_end.saturating_sub(1)),
        depth: heading_depth(file_kind, &symbol_source),
        source: symbol_source,
    })
}

fn is_legacy_filtered_symbol(file_kind: FileKind, symbol: &RawSymbol) -> bool {
    matches!(file_kind, FileKind::Code(Language::Rust))
        && symbol.parent_symbol.is_some()
        && symbol.kind != "method"
}

fn legacy_kind(file_kind: FileKind, kind: &str) -> Option<&str> {
    match (file_kind, kind) {
        (FileKind::Code(Language::Rust), "enum" | "type_alias") => Some("struct"),
        (FileKind::Code(Language::Rust), "const" | "static") => Some("field"),
        (FileKind::Code(Language::Rust), "test") => Some("function"),
        (FileKind::Code(Language::Java), "enum" | "record") => Some("class"),
        (FileKind::Code(Language::Go), "type_alias") => Some("struct"),
        (FileKind::Doc(DocFormat::Markdown), "heading") => Some("section"),
        (_, "package" | "namespace" | "delegate" | "record" | "event" | "property")
        | (_, "macro" | "global" | "function_declaration" | "union" | "constant")
        | (_, "companion_object" | "singleton_class" | "singleton_method")
        | (_, "object")
        | (_, "function" | "method" | "struct" | "class" | "interface" | "enum")
        | (_, "trait" | "impl" | "module" | "field" | "type_alias") => Some(kind),
        _ => None,
    }
}

fn legacy_synthesized_symbols(
    file_kind: FileKind,
    source: &str,
    existing: &[TranslatedSymbol],
) -> Vec<TranslatedSymbol> {
    // L-0047: keep legacy orbit-knowledge leaves until its query surface migrates intentionally.
    match file_kind {
        FileKind::Code(Language::C) => synthesize_c_macros(source),
        FileKind::Code(Language::Kotlin) => synthesize_kotlin_package(source),
        FileKind::Code(Language::Ruby) => synthesize_ruby_accessors(source, existing),
        _ => Vec::new(),
    }
}

fn synthesize_c_macros(source: &str) -> Vec<TranslatedSymbol> {
    source
        .lines()
        .enumerate()
        .filter_map(|(index, line)| {
            let trimmed = line.trim_start();
            let rest = trimmed.strip_prefix("#define ")?;
            let name = rest
                .split(|ch: char| ch.is_whitespace() || ch == '(')
                .next()
                .unwrap_or_default()
                .to_string();
            if name.is_empty() {
                return None;
            }
            let line_number = index + 1;
            Some(synthetic_symbol(
                name.clone(),
                name,
                "macro",
                line_number,
                line_number,
                trimmed.trim_end().to_string(),
                None,
            ))
        })
        .collect()
}

fn synthesize_kotlin_package(source: &str) -> Vec<TranslatedSymbol> {
    source
        .lines()
        .enumerate()
        .find_map(|(index, line)| {
            let trimmed = line.trim();
            let name = trimmed.strip_prefix("package ")?.trim().to_string();
            if name.is_empty() {
                return None;
            }
            let line_number = index + 1;
            Some(vec![synthetic_symbol(
                name.clone(),
                name,
                "package",
                line_number,
                line_number,
                trimmed.to_string(),
                None,
            )])
        })
        .unwrap_or_default()
}

fn synthesize_ruby_accessors(source: &str, existing: &[TranslatedSymbol]) -> Vec<TranslatedSymbol> {
    let mut out = Vec::new();
    for (index, line) in source.lines().enumerate() {
        let line_number = index + 1;
        let trimmed = line.trim();
        let Some((mode, rest)) = ruby_accessor_call(trimmed) else {
            continue;
        };
        let Some(parent) = containing_ruby_scope(existing, line_number) else {
            continue;
        };
        for attr in ruby_attr_names(rest) {
            if matches!(mode, RubyAccessorMode::Reader | RubyAccessorMode::Accessor) {
                out.push(synthetic_symbol(
                    format!("{parent}::{attr}"),
                    attr.clone(),
                    "method",
                    line_number,
                    line_number,
                    trimmed.to_string(),
                    Some(parent.clone()),
                ));
            }
            if matches!(mode, RubyAccessorMode::Writer | RubyAccessorMode::Accessor) {
                let writer = format!("{attr}=");
                out.push(synthetic_symbol(
                    format!("{parent}::{writer}"),
                    writer.clone(),
                    "method",
                    line_number,
                    line_number,
                    trimmed.to_string(),
                    Some(parent.clone()),
                ));
            }
        }
    }
    out
}

#[derive(Clone, Copy)]
enum RubyAccessorMode {
    Accessor,
    Reader,
    Writer,
}

fn ruby_accessor_call(trimmed: &str) -> Option<(RubyAccessorMode, &str)> {
    if let Some(rest) = trimmed.strip_prefix("attr_accessor") {
        Some((RubyAccessorMode::Accessor, rest))
    } else if let Some(rest) = trimmed.strip_prefix("attr_reader") {
        Some((RubyAccessorMode::Reader, rest))
    } else if let Some(rest) = trimmed.strip_prefix("attr_writer") {
        Some((RubyAccessorMode::Writer, rest))
    } else {
        None
    }
}

fn ruby_attr_names(rest: &str) -> Vec<String> {
    rest.split(',')
        .filter_map(|part| {
            let name = part
                .trim()
                .trim_start_matches(':')
                .trim_matches('"')
                .trim_matches('\'');
            (!name.is_empty()).then(|| name.to_string())
        })
        .collect()
}

fn containing_ruby_scope(existing: &[TranslatedSymbol], line: usize) -> Option<String> {
    existing
        .iter()
        .filter(|symbol| matches!(symbol.kind.as_str(), "class" | "module" | "singleton_class"))
        .filter(|symbol| symbol.start_line <= line && line <= symbol.end_line)
        .min_by_key(|symbol| {
            (
                symbol.end_line.saturating_sub(symbol.start_line),
                std::cmp::Reverse(symbol.start_line),
            )
        })
        .map(|symbol| symbol.qualified_name.clone())
}

fn synthetic_symbol(
    qualified_name: String,
    name: String,
    kind: &str,
    start_line: usize,
    end_line: usize,
    source: String,
    parent: Option<String>,
) -> TranslatedSymbol {
    TranslatedSymbol {
        original_qualified: qualified_name.clone(),
        original_parent: parent,
        qualified_name,
        name,
        kind: kind.to_string(),
        start_line,
        end_line,
        source,
        depth: None,
    }
}

fn adjust_markdown_sections(leaves: &mut [ExtractedLeaf], source: &str) {
    let lines: Vec<&str> = source.lines().collect();
    let section_lines: Vec<(usize, u8)> = leaves
        .iter()
        .map(|leaf| (leaf.start_line, leaf.depth.unwrap_or(1)))
        .collect();

    for (index, leaf) in leaves.iter_mut().enumerate() {
        let depth = leaf.depth.unwrap_or(1);
        let end_line = section_lines
            .iter()
            .skip(index + 1)
            .find_map(|(line, next_depth)| (*next_depth <= depth).then(|| line.saturating_sub(1)))
            .unwrap_or(lines.len());
        leaf.end_line = end_line;
        leaf.source = slice_lines(&lines, leaf.start_line, end_line);
        leaf.source_hash = compute_source_hash(&leaf.source);
    }
}

fn slice_lines(lines: &[&str], start_line: usize, end_line: usize) -> String {
    if start_line == 0 || end_line < start_line {
        return String::new();
    }
    let lo = start_line - 1;
    let hi = end_line.min(lines.len());
    lines[lo..hi].join("\n")
}

fn legacy_qualified_name(file_kind: FileKind, symbol: &RawSymbol, symbol_source: &str) -> String {
    if matches!(
        file_kind,
        FileKind::Code(Language::TypeScript | Language::Tsx)
    ) && symbol.kind == "method"
        && !symbol.qualified.contains('#')
    {
        return format!("{}#{}", symbol.qualified, parameter_arity(symbol_source));
    }
    symbol.qualified.clone()
}

fn legacy_symbol_source(file_kind: FileKind, source: &str, symbol: &RawSymbol) -> String {
    let fragment = source
        .get(symbol.span_start..symbol.span_end)
        .unwrap_or_default();
    match file_kind {
        FileKind::Code(Language::Rust) => fragment.trim().to_string(),
        FileKind::Doc(DocFormat::Markdown) => markdown_section_source(fragment),
        _ => fragment.trim_end().to_string(),
    }
}

fn markdown_section_source(fragment: &str) -> String {
    fragment.lines().collect::<Vec<_>>().join("\n")
}

fn heading_depth(file_kind: FileKind, symbol_source: &str) -> Option<u8> {
    if !matches!(file_kind, FileKind::Doc(DocFormat::Markdown)) {
        return None;
    }
    let heading = symbol_source.lines().next()?.trim_start();
    let depth = heading.bytes().take_while(|byte| *byte == b'#').count();
    u8::try_from(depth)
        .ok()
        .filter(|depth| (1..=6).contains(depth))
}

fn line_for_byte(source: &str, byte: usize) -> usize {
    let end = byte.min(source.len());
    source.as_bytes()[..end]
        .iter()
        .filter(|byte| **byte == b'\n')
        .count()
        + 1
}

fn parameter_arity(source: &str) -> usize {
    let Some(start) = source.find('(') else {
        return 0;
    };
    let Some(end) = matching_paren(source, start) else {
        return 0;
    };
    let params = source[start + 1..end].trim();
    if params.is_empty() {
        return 0;
    }

    let mut depth = 0usize;
    let mut count = 1usize;
    for ch in params.chars() {
        match ch {
            '(' | '[' | '{' | '<' => depth += 1,
            ')' | ']' | '}' | '>' => depth = depth.saturating_sub(1),
            ',' if depth == 0 => count += 1,
            _ => {}
        }
    }
    count
}

fn matching_paren(source: &str, start: usize) -> Option<usize> {
    let mut depth = 0usize;
    for (offset, ch) in source[start..].char_indices() {
        match ch {
            '(' => depth += 1,
            ')' => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    return Some(start + offset);
                }
            }
            _ => {}
        }
    }
    None
}

fn rust_exports(source: &str) -> Vec<ExtractedExport> {
    let mut parser = Parser::new();
    if parser
        .set_language(&tree_sitter_rust::LANGUAGE.into())
        .is_err()
    {
        return Vec::new();
    }
    let Some(tree) = parser.parse(source, None) else {
        return Vec::new();
    };

    let mut exports = Vec::new();
    let mut cursor = tree.root_node().walk();
    for child in tree.root_node().children(&mut cursor) {
        match child.kind() {
            "function_item" | "struct_item" | "enum_item" | "trait_item" | "mod_item"
            | "type_item" | "const_item" | "static_item" => {
                extract_named_public_export(child, source, &mut exports)
            }
            "use_declaration" => extract_pub_use_exports(child, source, &mut exports),
            _ => {}
        }
    }

    exports.sort_by(|left, right| {
        left.name
            .cmp(&right.name)
            .then_with(|| left.source_path.cmp(&right.source_path))
    });
    exports
        .dedup_by(|left, right| left.name == right.name && left.source_path == right.source_path);
    exports
}

fn extract_named_public_export(node: Node, source: &str, exports: &mut Vec<ExtractedExport>) {
    if !is_unrestricted_public(node, source) {
        return;
    }
    if let Some(name) = node_name(node, source) {
        exports.push(ExtractedExport {
            name,
            source_path: None,
        });
    }
}

fn extract_pub_use_exports(node: Node, source: &str, exports: &mut Vec<ExtractedExport>) {
    if !is_unrestricted_public(node, source) {
        return;
    }
    let Some(argument) = node.child_by_field_name("argument") else {
        return;
    };
    collect_use_exports(argument, source, &[], exports);
}

fn is_unrestricted_public(node: Node, source: &str) -> bool {
    let mut cursor = node.walk();
    node.children(&mut cursor).any(|child| {
        child.kind() == "visibility_modifier"
            && source
                .get(child.start_byte()..child.end_byte())
                .unwrap_or_default()
                .trim()
                == "pub"
    })
}

fn collect_use_exports(
    node: Node,
    source: &str,
    prefix: &[String],
    exports: &mut Vec<ExtractedExport>,
) {
    match node.kind() {
        "scoped_use_list" => {
            let mut next_prefix = prefix.to_vec();
            if let Some(path) = node.child_by_field_name("path") {
                next_prefix.extend(path_segments(path, source));
            }
            if let Some(list) = node.child_by_field_name("list") {
                collect_use_exports(list, source, &next_prefix, exports);
            }
        }
        "use_list" => {
            let mut cursor = node.walk();
            for child in node.named_children(&mut cursor) {
                collect_use_exports(child, source, prefix, exports);
            }
        }
        "use_as_clause" => {
            let Some(alias) = node.child_by_field_name("alias") else {
                return;
            };
            let Some(path) = node.child_by_field_name("path") else {
                return;
            };
            let mut source_segments = prefix.to_vec();
            source_segments.extend(path_segments(path, source));
            let Some(source_path) = join_segments(&source_segments) else {
                return;
            };
            exports.push(ExtractedExport {
                name: node_text(alias, source),
                source_path: Some(source_path),
            });
        }
        "use_wildcard" => {
            let mut source_segments = prefix.to_vec();
            let mut cursor = node.walk();
            for child in node.named_children(&mut cursor) {
                source_segments.extend(path_segments(child, source));
            }
            let Some(path) = join_segments(&source_segments) else {
                return;
            };
            let source_path = format!("{path}::*");
            exports.push(ExtractedExport {
                name: source_path.clone(),
                source_path: Some(source_path),
            });
        }
        "identifier" | "crate" | "self" | "super" | "scoped_identifier" => {
            let mut source_segments = prefix.to_vec();
            source_segments.extend(path_segments(node, source));
            let Some(source_path) = join_segments(&source_segments) else {
                return;
            };
            exports.push(ExtractedExport {
                name: export_name(&source_segments),
                source_path: Some(source_path),
            });
        }
        _ => {
            let mut cursor = node.walk();
            for child in node.named_children(&mut cursor) {
                collect_use_exports(child, source, prefix, exports);
            }
        }
    }
}

fn node_name(node: Node, source: &str) -> Option<String> {
    node.child_by_field_name("name")
        .map(|name| node_text(name, source))
        .filter(|name| !name.is_empty())
}

fn node_text(node: Node, source: &str) -> String {
    source
        .get(node.start_byte()..node.end_byte())
        .unwrap_or_default()
        .trim()
        .to_string()
}

fn path_segments(node: Node, source: &str) -> Vec<String> {
    node_text(node, source)
        .split("::")
        .map(str::trim)
        .filter(|segment| !segment.is_empty())
        .map(ToOwned::to_owned)
        .collect()
}

fn join_segments(segments: &[String]) -> Option<String> {
    (!segments.is_empty()).then(|| segments.join("::"))
}

fn export_name(segments: &[String]) -> String {
    match segments.last().map(String::as_str) {
        Some("self") if segments.len() > 1 => segments[segments.len() - 2].clone(),
        Some(name) => name.to_string(),
        None => String::new(),
    }
}

#[cfg(test)]
mod tests;
