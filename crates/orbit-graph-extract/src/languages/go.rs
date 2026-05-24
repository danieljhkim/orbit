//! Go tree-sitter extraction.

use std::collections::HashMap;
use std::path::Path;

use tree_sitter::{Node, Parser};

use crate::{ExtractedFile, Extractor, RawImport, RawRef, RawRelation, RawSymbol};

/// Extracts Go source files into raw graph rows.
pub struct GoExtractor;

impl Extractor for GoExtractor {
    fn lang(&self) -> &'static str {
        "go"
    }

    fn supports(&self, path: &Path) -> bool {
        path.extension().and_then(|ext| ext.to_str()) == Some("go")
    }

    fn extract(&self, path: &Path, bytes: &[u8]) -> ExtractedFile {
        let Ok(source) = std::str::from_utf8(bytes) else {
            return ExtractedFile::default();
        };

        let mut parser = Parser::new();
        if parser
            .set_language(&tree_sitter_go::LANGUAGE.into())
            .is_err()
        {
            return ExtractedFile::default();
        }

        let Some(tree) = parser.parse(source, None) else {
            return ExtractedFile::default();
        };

        let mut state = ExtractionState::new(path);
        extract_top_level(tree.root_node(), source, &mut state);
        state.finish()
    }
}

struct ExtractionState {
    file_path: String,
    symbols: Vec<RawSymbol>,
    refs: Vec<RawRef>,
    relations: Vec<RawRelation>,
    imports: Vec<RawImport>,
    import_aliases: HashMap<String, String>,
}

impl ExtractionState {
    fn new(path: &Path) -> Self {
        Self {
            file_path: normalize_path(path),
            symbols: Vec::new(),
            refs: Vec::new(),
            relations: Vec::new(),
            imports: Vec::new(),
            import_aliases: HashMap::new(),
        }
    }

    fn finish(mut self) -> ExtractedFile {
        dedup_symbols(&mut self.symbols);
        dedup_refs(&mut self.refs);
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
        if is_ignored_type_name(&target_name) {
            return;
        }

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

fn extract_top_level(node: Node, source: &str, state: &mut ExtractionState) {
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        match child.kind() {
            "import_declaration" => extract_import(child, source, state),
            "function_declaration" => extract_function(child, source, state),
            "method_declaration" => extract_method(child, source, state),
            "type_declaration" => extract_type_declaration(child, source, state),
            "const_declaration" => extract_binding_declaration(child, source, "const_spec", state),
            "var_declaration" => extract_binding_declaration(child, source, "var_spec", state),
            _ => collect_expression_refs(child, source, state),
        }
    }
}

fn extract_function(node: Node, source: &str, state: &mut ExtractionState) {
    let Some(name) = get_name(node, source) else {
        return;
    };
    state.push_symbol(node, source, name.clone(), name, "function", None);
    collect_signature_refs(node, source, state);
    if let Some(body) = node.child_by_field_name("body") {
        collect_expression_refs(body, source, state);
    }
}

fn extract_method(node: Node, source: &str, state: &mut ExtractionState) {
    let Some(receiver) = receiver_type_name(node, source) else {
        return;
    };
    let Some(name) = get_name(node, source) else {
        return;
    };
    let qualified = format!("{receiver}::{name}");
    state.push_symbol(node, source, name, qualified, "method", Some(receiver));
    collect_signature_refs(node, source, state);
    if let Some(body) = node.child_by_field_name("body") {
        collect_expression_refs(body, source, state);
    }
}

fn extract_type_declaration(node: Node, source: &str, state: &mut ExtractionState) {
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        match child.kind() {
            "type_spec" | "type_alias" => extract_type_spec(child, source, state),
            _ => {}
        }
    }
}

fn extract_type_spec(node: Node, source: &str, state: &mut ExtractionState) {
    let Some(name) = get_name(node, source) else {
        return;
    };
    let Some(type_node) = node.child_by_field_name("type") else {
        return;
    };

    let kind = match type_node.kind() {
        "interface_type" => "interface",
        "struct_type" => "struct",
        _ if node.kind() == "type_alias" => "type_alias",
        _ => "type_alias",
    };
    state.push_symbol(node, source, name.clone(), name.clone(), kind, None);

    if type_node.kind() == "interface_type" {
        extract_interface_methods(type_node, &name, source, state);
    }
    collect_type_refs(type_node, source, state);
}

fn extract_interface_methods(
    interface_type: Node,
    parent: &str,
    source: &str,
    state: &mut ExtractionState,
) {
    let mut cursor = interface_type.walk();
    for child in interface_type.named_children(&mut cursor) {
        if child.kind() != "method_elem" {
            collect_type_refs(child, source, state);
            continue;
        }
        let Some(name) = get_name(child, source) else {
            continue;
        };
        state.push_symbol(
            child,
            source,
            name.clone(),
            format!("{parent}::{name}"),
            "method",
            Some(parent.to_string()),
        );
        collect_signature_refs(child, source, state);
    }
}

fn extract_binding_declaration(
    node: Node,
    source: &str,
    spec_kind: &str,
    state: &mut ExtractionState,
) {
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        if child.kind() != spec_kind {
            continue;
        }
        let Some(name) = get_name(child, source) else {
            continue;
        };
        state.push_symbol(child, source, name.clone(), name, "field", None);
        if let Some(type_node) = child.child_by_field_name("type") {
            collect_type_refs(type_node, source, state);
        }
        if let Some(value) = child.child_by_field_name("value") {
            collect_expression_refs(value, source, state);
        }
    }
}

fn collect_signature_refs(node: Node, source: &str, state: &mut ExtractionState) {
    if let Some(receiver) = node.child_by_field_name("receiver") {
        collect_type_refs(receiver, source, state);
    }
    if let Some(parameters) = node.child_by_field_name("parameters") {
        collect_type_refs(parameters, source, state);
    }
    if let Some(result) = node.child_by_field_name("result") {
        collect_type_refs(result, source, state);
    }
    if let Some(type_parameters) = node.child_by_field_name("type_parameters") {
        collect_type_refs(type_parameters, source, state);
    }
}

fn collect_expression_refs(node: Node, source: &str, state: &mut ExtractionState) {
    match node.kind() {
        "call_expression" => {
            collect_call_ref(node, source, state);
            if let Some(type_arguments) = node.child_by_field_name("type_arguments") {
                collect_type_refs(type_arguments, source, state);
            }
            if let Some(arguments) = node.child_by_field_name("arguments") {
                collect_expression_refs(arguments, source, state);
            }
            return;
        }
        "function_declaration"
        | "method_declaration"
        | "type_declaration"
        | "import_declaration"
        | "const_declaration"
        | "var_declaration" => return,
        _ if is_type_reference_node(node) => {
            collect_type_refs(node, source, state);
            return;
        }
        _ => {}
    }

    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        collect_expression_refs(child, source, state);
    }
}

fn collect_call_ref(node: Node, source: &str, state: &mut ExtractionState) {
    let Some(function) = node.child_by_field_name("function") else {
        return;
    };

    match function.kind() {
        "identifier" => state.push_ref(
            function,
            source,
            Some(node_text(function, source)),
            "call",
            "same_module",
        ),
        "selector_expression" => collect_selector_call_ref(function, source, state),
        "type_instantiation_expression" | "generic_type" => {
            collect_expression_refs(function, source, state);
        }
        _ => collect_expression_refs(function, source, state),
    }
}

fn collect_selector_call_ref(node: Node, source: &str, state: &mut ExtractionState) {
    let Some(field) = node.child_by_field_name("field") else {
        return;
    };
    let Some(operand) = node.child_by_field_name("operand") else {
        state.push_ref(field, source, None, "call", "fuzzy_name");
        return;
    };

    let operand_name = node_text(operand, source);
    if let Some(target_path) = state.import_aliases.get(&operand_name) {
        state.push_ref(
            field,
            source,
            Some(format!("{target_path}.{}", node_text(field, source))),
            "call",
            "import_resolved",
        );
    } else {
        state.push_ref(field, source, None, "call", "fuzzy_name");
    }
}

fn collect_type_refs(node: Node, source: &str, state: &mut ExtractionState) {
    match node.kind() {
        "type_identifier" => {
            let target = node_text(node, source);
            state.push_ref(node, source, Some(target), "type", "same_module");
            return;
        }
        "qualified_type" => {
            collect_qualified_type_ref(node, source, state);
            return;
        }
        "generic_type" => {
            if let Some(type_node) = node.child_by_field_name("type") {
                collect_type_refs(type_node, source, state);
            }
            if let Some(type_arguments) = node.child_by_field_name("type_arguments") {
                collect_type_refs(type_arguments, source, state);
            }
            return;
        }
        "identifier" | "field_identifier" | "package_identifier" | "blank_identifier" => return,
        _ => {}
    }

    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        collect_type_refs(child, source, state);
    }
}

fn collect_qualified_type_ref(node: Node, source: &str, state: &mut ExtractionState) {
    let Some(name_node) = node.child_by_field_name("name") else {
        return;
    };
    let Some(package_node) = node.child_by_field_name("package") else {
        return;
    };

    let package = node_text(package_node, source);
    let name = node_text(name_node, source);
    let target_qualified = state.import_aliases.get(&package).map_or_else(
        || format!("{package}.{name}"),
        |path| format!("{path}.{name}"),
    );
    let confidence = if state.import_aliases.contains_key(&package) {
        "import_resolved"
    } else {
        "same_module"
    };
    state.push_ref(
        name_node,
        source,
        Some(target_qualified),
        "type",
        confidence,
    );
}

fn extract_import(node: Node, source: &str, state: &mut ExtractionState) {
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        match child.kind() {
            "import_spec" => extract_import_spec(child, source, state),
            "import_spec_list" => extract_import(child, source, state),
            _ => {}
        }
    }
}

fn extract_import_spec(node: Node, source: &str, state: &mut ExtractionState) {
    let Some(path_node) = node.child_by_field_name("path") else {
        return;
    };
    let target_path = unquote_literal(&node_text(path_node, source));
    if target_path.is_empty() {
        return;
    }

    let explicit_name = node
        .child_by_field_name("name")
        .map(|name| node_text(name, source));
    let alias = explicit_name
        .as_deref()
        .filter(|name| !matches!(*name, "." | "_"))
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| default_import_name(&target_path));
    state
        .import_aliases
        .insert(alias.clone(), target_path.clone());

    state.imports.push(RawImport {
        from_file: state.file_path.clone(),
        target_path: target_path.clone(),
        target_symbol: explicit_name.filter(|name| !matches!(name.as_str(), "." | "_")),
    });
    state.push_ref(
        path_node,
        source,
        Some(target_path),
        "use",
        "import_resolved",
    );
}

fn get_name(node: Node, source: &str) -> Option<String> {
    node.child_by_field_name("name")
        .map(|name| node_text(name, source))
        .filter(|name| !name.is_empty())
}

fn receiver_type_name(node: Node, source: &str) -> Option<String> {
    let receiver = node.child_by_field_name("receiver")?;
    let mut cursor = receiver.walk();
    let declaration = receiver
        .named_children(&mut cursor)
        .find(|child| child.kind() == "parameter_declaration")?;
    let receiver_type = declaration.child_by_field_name("type")?;
    let raw = node_text(receiver_type, source);
    Some(raw.trim_start_matches('*').trim().to_string())
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
        .or_else(|| node.child_by_field_name("value"))
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

fn unquote_literal(raw: &str) -> String {
    raw.trim_matches('"')
        .trim_matches('`')
        .trim_matches('\'')
        .to_string()
}

fn default_import_name(path: &str) -> String {
    path.rsplit('/').next().unwrap_or(path).to_string()
}

fn target_name(node: Node, source: &str) -> Option<String> {
    let text = node_text(node, source);
    text.rsplit(['.', '/'])
        .next()
        .map(str::to_string)
        .filter(|name| !name.is_empty())
}

fn is_type_reference_node(node: Node) -> bool {
    matches!(
        node.kind(),
        "type_identifier"
            | "qualified_type"
            | "generic_type"
            | "array_type"
            | "slice_type"
            | "map_type"
            | "channel_type"
            | "function_type"
            | "pointer_type"
            | "struct_type"
            | "interface_type"
            | "parameter_list"
            | "parameter_declaration"
    )
}

fn is_ignored_type_name(name: &str) -> bool {
    matches!(
        name,
        "any"
            | "bool"
            | "byte"
            | "comparable"
            | "complex64"
            | "complex128"
            | "error"
            | "float32"
            | "float64"
            | "int"
            | "int8"
            | "int16"
            | "int32"
            | "int64"
            | "rune"
            | "string"
            | "uint"
            | "uint8"
            | "uint16"
            | "uint32"
            | "uint64"
            | "uintptr"
    )
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
#[path = "tests/go.rs"]
mod tests;
