//! Ruby tree-sitter extraction.

use std::path::Path;

use tree_sitter::{Node, Parser};

use crate::{ExtractedFile, Extractor, RawImport, RawRef, RawRelation, RawSymbol};

/// Extracts Ruby source files into raw graph rows.
pub struct RubyExtractor;

impl Extractor for RubyExtractor {
    fn lang(&self) -> &'static str {
        "ruby"
    }

    fn supports(&self, path: &Path) -> bool {
        path.extension().and_then(|ext| ext.to_str()) == Some("rb")
    }

    fn extract(&self, path: &Path, bytes: &[u8]) -> ExtractedFile {
        let Ok(source) = std::str::from_utf8(bytes) else {
            return ExtractedFile::default();
        };

        let mut parser = Parser::new();
        if parser
            .set_language(&tree_sitter_ruby::LANGUAGE.into())
            .is_err()
        {
            return ExtractedFile::default();
        }

        let Some(tree) = parser.parse(source, None) else {
            return ExtractedFile::default();
        };

        let mut state = ExtractionState::new(path);
        extract_children(tree.root_node(), source, None, &mut state);
        state.finish()
    }
}

struct ExtractionState {
    file_path: String,
    symbols: Vec<RawSymbol>,
    refs: Vec<RawRef>,
    relations: Vec<RawRelation>,
    imports: Vec<RawImport>,
}

impl ExtractionState {
    fn new(path: &Path) -> Self {
        Self {
            file_path: normalize_path(path),
            symbols: Vec::new(),
            refs: Vec::new(),
            relations: Vec::new(),
            imports: Vec::new(),
        }
    }

    fn finish(mut self) -> ExtractedFile {
        dedup_symbols(&mut self.symbols);
        dedup_refs(&mut self.refs);
        dedup_relations(&mut self.relations);
        dedup_imports(&mut self.imports);
        ExtractedFile {
            symbols: self.symbols,
            refs: self.refs,
            relations: self.relations,
            imports: self.imports,
            strings: Vec::new(),
            configs: Vec::new(),
            commands: Vec::new(),
        }
    }

    fn push_symbol(
        &mut self,
        node: Node,
        source: &str,
        name: String,
        qualified: String,
        kind: &'static str,
        parent_symbol: Option<String>,
    ) {
        self.symbols.push(RawSymbol {
            file_path: self.file_path.clone(),
            name,
            qualified,
            kind: kind.to_string(),
            span_start: node.start_byte(),
            span_end: node.end_byte(),
            signature: signature_for(node, source),
            parent_symbol,
        });
    }

    fn push_ref(
        &mut self,
        node: Node,
        source: &str,
        target_qualified: Option<String>,
        kind: &'static str,
        confidence: &'static str,
    ) {
        let Some(target_name) = target_name(node, source) else {
            return;
        };

        self.refs.push(RawRef {
            from_file: self.file_path.clone(),
            from_span_start: node.start_byte(),
            from_span_end: node.end_byte(),
            target_name,
            target_qualified,
            kind: kind.to_string(),
            confidence: confidence.to_string(),
        });
    }
}

fn extract_children(
    node: Node,
    source: &str,
    parent_symbol: Option<&str>,
    state: &mut ExtractionState,
) {
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        match child.kind() {
            "module" => extract_scope(child, source, parent_symbol, "module", state),
            "class" => extract_scope(child, source, parent_symbol, "class", state),
            "singleton_class" => extract_singleton_class(child, source, parent_symbol, state),
            "method" => extract_method(child, source, parent_symbol, state),
            "singleton_method" => extract_singleton_method(child, source, parent_symbol, state),
            "assignment" if parent_symbol.is_none() => {
                extract_top_level_constant(child, source, state);
            }
            "call" => handle_call(child, source, parent_symbol, state),
            "body_statement" | "program" => {
                extract_children(child, source, parent_symbol, state);
            }
            _ => collect_expression_refs(child, source, parent_symbol, state),
        }
    }
}

fn extract_scope(
    node: Node,
    source: &str,
    parent_symbol: Option<&str>,
    kind: &'static str,
    state: &mut ExtractionState,
) {
    let Some(name) = get_name(node, source) else {
        return;
    };
    let qualified = qualify_name(parent_symbol, &name);
    if kind == "class" {
        extract_superclass_relation(node, source, &qualified, state);
    }
    if let Some(body) = node.child_by_field_name("body") {
        extract_children(body, source, Some(&qualified), state);
    }
    state.push_symbol(
        node,
        source,
        name,
        qualified,
        kind,
        parent_symbol.map(ToOwned::to_owned),
    );
}

fn extract_superclass_relation(
    node: Node,
    source: &str,
    class_qualified: &str,
    state: &mut ExtractionState,
) {
    let Some(superclass) = node.child_by_field_name("superclass") else {
        return;
    };
    let Some(target_node) = first_named_child(superclass) else {
        return;
    };
    let target = normalize_qualified_name(&node_text(target_node, source));
    if target.is_empty() {
        return;
    }
    push_extends_relation(target_node, source, class_qualified, &target, state);
}

fn extract_singleton_class(
    node: Node,
    source: &str,
    parent_symbol: Option<&str>,
    state: &mut ExtractionState,
) {
    let Some(value) = node.child_by_field_name("value") else {
        return;
    };
    let name = node_text(value, source);
    if name.is_empty() {
        return;
    }

    let qualified = qualify_name(parent_symbol, &name);
    if let Some(body) = node.child_by_field_name("body") {
        extract_children(body, source, Some(&qualified), state);
    }
    state.push_symbol(
        node,
        source,
        name,
        qualified,
        "singleton_class",
        parent_symbol.map(ToOwned::to_owned),
    );
}

fn extract_method(
    node: Node,
    source: &str,
    parent_symbol: Option<&str>,
    state: &mut ExtractionState,
) {
    let Some(name) = get_name(node, source) else {
        return;
    };
    let qualified = qualify_name(parent_symbol, &name);
    state.push_symbol(
        node,
        source,
        name,
        qualified,
        "method",
        parent_symbol.map(ToOwned::to_owned),
    );
    if let Some(body) = node.child_by_field_name("body") {
        collect_expression_refs(body, source, parent_symbol, state);
    }
}

fn extract_singleton_method(
    node: Node,
    source: &str,
    parent_symbol: Option<&str>,
    state: &mut ExtractionState,
) {
    let Some(object) = node.child_by_field_name("object") else {
        return;
    };
    let Some(method_name) = get_name(node, source) else {
        return;
    };
    let object_name = node_text(object, source);
    if object_name.is_empty() {
        return;
    }

    let name = format!("{object_name}.{method_name}");
    let qualified = match parent_symbol {
        Some(parent) if object_name == "self" => format!("{parent}.{method_name}"),
        Some(parent) => format!("{parent}::{name}"),
        None => name.clone(),
    };
    state.push_symbol(
        node,
        source,
        name,
        qualified,
        "singleton_method",
        parent_symbol.map(ToOwned::to_owned),
    );
    if let Some(body) = node.child_by_field_name("body") {
        collect_expression_refs(body, source, parent_symbol, state);
    }
}

fn extract_top_level_constant(node: Node, source: &str, state: &mut ExtractionState) {
    let Some(left) = node.child_by_field_name("left") else {
        return;
    };
    if left.kind() != "constant" {
        collect_expression_refs(node, source, None, state);
        return;
    }

    let name = node_text(left, source);
    if name.is_empty() {
        return;
    }
    state.push_symbol(node, source, name.clone(), name, "constant", None);
    if let Some(right) = node.child_by_field_name("right") {
        collect_expression_refs(right, source, None, state);
    }
}

fn collect_expression_refs(
    node: Node,
    source: &str,
    parent_symbol: Option<&str>,
    state: &mut ExtractionState,
) {
    match node.kind() {
        "call" => {
            handle_call(node, source, parent_symbol, state);
            return;
        }
        "module" | "class" | "singleton_class" | "method" | "singleton_method" => return,
        "constant" | "scope_resolution" => {
            let target = normalize_qualified_name(&node_text(node, source));
            if !target.is_empty() {
                state.push_ref(
                    node,
                    source,
                    Some(target.clone()),
                    "type",
                    confidence_for_name(&target),
                );
            }
            return;
        }
        _ => {}
    }

    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        collect_expression_refs(child, source, parent_symbol, state);
    }
}

fn handle_call(node: Node, source: &str, parent_symbol: Option<&str>, state: &mut ExtractionState) {
    extract_require_import(node, source, state);
    extract_include_or_extend_relation(node, source, parent_symbol, state);
    collect_call_ref(node, source, parent_symbol, state);

    if let Some(arguments) = node.child_by_field_name("arguments") {
        collect_expression_refs(arguments, source, parent_symbol, state);
    }
    if let Some(block) = node.child_by_field_name("block") {
        collect_expression_refs(block, source, parent_symbol, state);
    }
}

fn collect_call_ref(
    node: Node,
    source: &str,
    parent_symbol: Option<&str>,
    state: &mut ExtractionState,
) {
    let Some(method) = node.child_by_field_name("method") else {
        return;
    };
    let method_name = node_text(method, source);
    if matches!(method_name.as_str(), "require" | "include" | "extend") {
        return;
    }

    if node.child_by_field_name("receiver").is_some() {
        state.push_ref(method, source, None, "call", "fuzzy_name");
    } else {
        state.push_ref(
            method,
            source,
            Some(qualify_name(parent_symbol, &method_name)),
            "call",
            "same_module",
        );
    }
}

fn extract_require_import(node: Node, source: &str, state: &mut ExtractionState) {
    if node.child_by_field_name("receiver").is_some() {
        return;
    }
    let Some(method) = node.child_by_field_name("method") else {
        return;
    };
    if node_text(method, source) != "require" {
        return;
    }
    let Some(arguments) = node.child_by_field_name("arguments") else {
        return;
    };

    let mut cursor = arguments.walk();
    for child in arguments.named_children(&mut cursor) {
        if child.kind() != "string" {
            continue;
        }
        let target_path = string_literal_value(child, source);
        if target_path.is_empty() {
            continue;
        }
        state.imports.push(RawImport {
            from_file: state.file_path.clone(),
            target_path,
            target_symbol: None,
        });
    }
}

fn extract_include_or_extend_relation(
    node: Node,
    source: &str,
    parent_symbol: Option<&str>,
    state: &mut ExtractionState,
) {
    if node.child_by_field_name("receiver").is_some() {
        return;
    }
    let Some(parent) = parent_symbol else {
        return;
    };
    let Some(method) = node.child_by_field_name("method") else {
        return;
    };
    if !matches!(node_text(method, source).as_str(), "include" | "extend") {
        return;
    }
    let Some(arguments) = node.child_by_field_name("arguments") else {
        return;
    };

    let mut cursor = arguments.walk();
    for child in arguments.named_children(&mut cursor) {
        if !matches!(child.kind(), "constant" | "scope_resolution") {
            continue;
        }
        let target = normalize_qualified_name(&node_text(child, source));
        if target.is_empty() {
            continue;
        }
        push_extends_relation(child, source, parent, &target, state);
    }
}

fn push_extends_relation(
    node: Node,
    source: &str,
    from_qualified: &str,
    to_qualified: &str,
    state: &mut ExtractionState,
) {
    state.relations.push(RawRelation {
        from_qualified: from_qualified.to_string(),
        to_qualified: to_qualified.to_string(),
        kind: "extends".to_string(),
        def_file: state.file_path.clone(),
        def_span_start: node.start_byte(),
        def_span_end: node.end_byte(),
        confidence: confidence_for_name(to_qualified).to_string(),
    });
    state.push_ref(
        node,
        source,
        Some(to_qualified.to_string()),
        "type",
        confidence_for_name(to_qualified),
    );
}

fn get_name(node: Node, source: &str) -> Option<String> {
    node.child_by_field_name("name")
        .map(|name| node_text(name, source))
        .filter(|name| !name.is_empty())
}

fn first_named_child(node: Node) -> Option<Node> {
    let mut cursor = node.walk();
    node.named_children(&mut cursor).next()
}

fn node_text(node: Node, source: &str) -> String {
    node.utf8_text(source.as_bytes())
        .unwrap_or("")
        .trim()
        .to_string()
}

fn signature_for(node: Node, source: &str) -> Option<String> {
    let end = node
        .child_by_field_name("body")
        .map_or_else(|| node.end_byte(), |body| body.start_byte());
    source
        .get(node.start_byte()..end)
        .map(normalize_signature)
        .filter(|signature| !signature.is_empty())
}

fn normalize_signature(signature: &str) -> String {
    signature.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn normalize_path(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

fn qualify_name(parent: Option<&str>, name: &str) -> String {
    match parent {
        Some(parent) if !name.contains("::") => format!("{parent}::{name}"),
        _ => name.to_string(),
    }
}

fn normalize_qualified_name(name: &str) -> String {
    name.split("::")
        .map(str::trim)
        .filter(|segment| !segment.is_empty())
        .collect::<Vec<_>>()
        .join("::")
}

fn string_literal_value(node: Node, source: &str) -> String {
    let mut value = String::new();
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        if child.kind() == "string_content" {
            value.push_str(&node_text(child, source));
        }
    }
    if value.is_empty() {
        node_text(node, source)
            .trim_matches('"')
            .trim_matches('\'')
            .to_string()
    } else {
        value
    }
}

fn target_name(node: Node, source: &str) -> Option<String> {
    let text = normalize_qualified_name(&node_text(node, source));
    text.rsplit("::")
        .next()
        .map(str::to_string)
        .filter(|name| !name.is_empty())
}

fn confidence_for_name(name: &str) -> &'static str {
    if name.contains("::") {
        "import_resolved"
    } else {
        "same_module"
    }
}

fn dedup_symbols(symbols: &mut Vec<RawSymbol>) {
    symbols.sort_by(|left, right| {
        left.span_start
            .cmp(&right.span_start)
            .then_with(|| left.span_end.cmp(&right.span_end))
            .then_with(|| left.kind.cmp(&right.kind))
            .then_with(|| left.qualified.cmp(&right.qualified))
    });
    symbols.dedup_by(|left, right| {
        left.file_path == right.file_path
            && left.qualified == right.qualified
            && left.kind == right.kind
            && left.span_start == right.span_start
            && left.span_end == right.span_end
    });
}

fn dedup_refs(refs: &mut Vec<RawRef>) {
    refs.sort_by(|left, right| {
        left.from_span_start
            .cmp(&right.from_span_start)
            .then_with(|| left.from_span_end.cmp(&right.from_span_end))
            .then_with(|| left.kind.cmp(&right.kind))
            .then_with(|| left.target_name.cmp(&right.target_name))
    });
    refs.dedup_by(|left, right| {
        left.from_file == right.from_file
            && left.from_span_start == right.from_span_start
            && left.from_span_end == right.from_span_end
            && left.target_name == right.target_name
            && left.target_qualified == right.target_qualified
            && left.kind == right.kind
            && left.confidence == right.confidence
    });
}

fn dedup_relations(relations: &mut Vec<RawRelation>) {
    relations.sort_by(|left, right| {
        left.def_span_start
            .cmp(&right.def_span_start)
            .then_with(|| left.from_qualified.cmp(&right.from_qualified))
            .then_with(|| left.to_qualified.cmp(&right.to_qualified))
    });
    relations.dedup_by(|left, right| {
        left.from_qualified == right.from_qualified
            && left.to_qualified == right.to_qualified
            && left.kind == right.kind
            && left.def_file == right.def_file
            && left.def_span_start == right.def_span_start
            && left.def_span_end == right.def_span_end
    });
}

fn dedup_imports(imports: &mut Vec<RawImport>) {
    imports.sort_by(|left, right| {
        left.target_path
            .cmp(&right.target_path)
            .then_with(|| left.target_symbol.cmp(&right.target_symbol))
    });
    imports.dedup_by(|left, right| {
        left.from_file == right.from_file
            && left.target_path == right.target_path
            && left.target_symbol == right.target_symbol
    });
}

#[cfg(test)]
#[path = "tests/ruby.rs"]
mod tests;
