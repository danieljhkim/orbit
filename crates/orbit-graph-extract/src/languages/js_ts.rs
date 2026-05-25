use std::path::Path;

use tree_sitter::Node;

use crate::{ExtractedFile, RawImport, RawRef, RawRelation, RawSymbol};

#[derive(Debug, Clone, Copy)]
pub(super) struct JsTsOptions {
    pub(super) type_syntax: bool,
}

pub(super) fn extract_file(
    path: &Path,
    source: &str,
    root: Node,
    options: JsTsOptions,
) -> ExtractedFile {
    let mut state = ExtractionState::new(path);
    extract_statements(root, source, options, &mut state);
    state.finish()
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
        let text = reference_text(node, source);
        let Some(target_name) = target_name_from_text(&text) else {
            return;
        };
        self.push_ref_span(
            node.start_byte(),
            node.end_byte(),
            target_name,
            target_qualified,
            kind,
            confidence,
        );
    }

    fn push_type_ref(&mut self, node: Node, source: &str) {
        let text = reference_text(node, source);
        let normalized = normalize_reference_name(&text);
        if normalized.is_empty() || is_ignored_type_name(&normalized) {
            return;
        }
        let Some(target_name) = target_name_from_text(&normalized) else {
            return;
        };
        let confidence = confidence_for_reference(&normalized);
        self.push_ref_span(
            node.start_byte(),
            node.end_byte(),
            target_name,
            Some(normalized),
            "type",
            confidence,
        );
    }

    fn push_ref_span(
        &mut self,
        span_start: usize,
        span_end: usize,
        target_name: String,
        target_qualified: Option<String>,
        kind: &'static str,
        confidence: &'static str,
    ) {
        if target_name.is_empty() || is_ignored_type_name(&target_name) {
            return;
        }
        self.refs.push(RawRef {
            from_file: self.file_path.clone(),
            from_span_start: span_start,
            from_span_end: span_end,
            target_name,
            target_qualified,
            kind: kind.to_string(),
            confidence: confidence.to_string(),
        });
    }

    fn push_relation_span(
        &mut self,
        span_start: usize,
        span_end: usize,
        from_qualified: &str,
        to_qualified: String,
        kind: &'static str,
    ) {
        if to_qualified.is_empty() {
            return;
        }
        self.relations.push(RawRelation {
            from_qualified: from_qualified.to_string(),
            to_qualified,
            kind: kind.to_string(),
            def_file: self.file_path.clone(),
            def_span_start: span_start,
            def_span_end: span_end,
            confidence: "exact".to_string(),
        });
    }

    fn push_import(&mut self, target_path: &str, target_symbol: Option<String>) {
        self.imports.push(RawImport {
            from_file: self.file_path.clone(),
            target_path: target_path.to_string(),
            target_symbol,
        });
    }
}

fn extract_statements(node: Node, source: &str, options: JsTsOptions, state: &mut ExtractionState) {
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        extract_statement(child, source, options, state);
    }
}

fn extract_statement(node: Node, source: &str, options: JsTsOptions, state: &mut ExtractionState) {
    match node.kind() {
        "program" => extract_statements(node, source, options, state),
        "import_statement" => extract_import_statement(node, source, state),
        "export_statement" => extract_export_statement(node, source, options, state),
        "ambient_declaration" => {
            if let Some(child) = first_named_child(node) {
                extract_statement(child, source, options, state);
            }
        }
        "function_declaration" | "generator_function_declaration" | "function_signature" => {
            extract_function(node, source, options, state)
        }
        "class_declaration" | "abstract_class_declaration" => {
            extract_class_declaration(node, node, source, options, state)
        }
        "interface_declaration" if options.type_syntax => {
            extract_interface(node, source, options, state);
        }
        "type_alias_declaration" if options.type_syntax => {
            extract_named_declaration(node, source, options, "type_alias", state);
        }
        "enum_declaration" if options.type_syntax => {
            extract_named_declaration(node, source, options, "enum", state);
        }
        "lexical_declaration" | "variable_declaration" => {
            extract_binding_declaration(node, source, options, state);
        }
        _ => collect_expression_refs(node, source, options, state),
    }
}

fn extract_export_statement(
    node: Node,
    source: &str,
    options: JsTsOptions,
    state: &mut ExtractionState,
) {
    if let Some(source_node) = node.child_by_field_name("source") {
        let target_path = string_literal_value(source_node, source);
        let (clause, clause_start) = clause_before_source(node, source_node, source, "export");
        let symbols = import_symbols_from_clause(clause, clause_start);
        push_import_rows(&target_path, symbols, state);
        return;
    }

    if let Some(declaration) = node.child_by_field_name("declaration") {
        extract_statement(declaration, source, options, state);
    }
}

fn extract_import_statement(node: Node, source: &str, state: &mut ExtractionState) {
    let Some(source_node) = node.child_by_field_name("source") else {
        return;
    };

    let target_path = string_literal_value(source_node, source);
    let (clause, clause_start) = clause_before_source(node, source_node, source, "import");
    let symbols = import_symbols_from_clause(clause, clause_start);
    push_import_rows(&target_path, symbols, state);
}

fn push_import_rows(target_path: &str, symbols: Vec<ImportSymbol>, state: &mut ExtractionState) {
    if symbols.is_empty() {
        state.push_import(target_path, None);
        return;
    }

    for symbol in symbols {
        state.push_import(target_path, Some(symbol.name.clone()));
        state.push_ref_span(
            symbol.span_start,
            symbol.span_end,
            symbol.name.clone(),
            Some(import_qualified_name(target_path, &symbol.name)),
            "use",
            "import_resolved",
        );
    }
}

fn extract_function(node: Node, source: &str, options: JsTsOptions, state: &mut ExtractionState) {
    let Some(name) = get_name(node, source) else {
        return;
    };

    state.push_symbol(node, source, name.clone(), name, "function", None);
    collect_signature_type_refs(node, source, options, state);
    if let Some(body) = node.child_by_field_name("body") {
        collect_expression_refs(body, source, options, state);
    }
}

fn extract_named_declaration(
    node: Node,
    source: &str,
    options: JsTsOptions,
    kind: &'static str,
    state: &mut ExtractionState,
) {
    let Some(name) = get_name(node, source) else {
        return;
    };
    state.push_symbol(node, source, name.clone(), name, kind, None);
    collect_signature_type_refs(node, source, options, state);
}

fn extract_interface(node: Node, source: &str, options: JsTsOptions, state: &mut ExtractionState) {
    let Some(name) = get_name(node, source) else {
        return;
    };
    state.push_symbol(node, source, name.clone(), name.clone(), "interface", None);
    extract_header_relations(node, source, &name, "extends", "extends", state);
    collect_signature_type_refs(node, source, options, state);
}

fn extract_class_declaration(
    class_node: Node,
    symbol_node: Node,
    source: &str,
    options: JsTsOptions,
    state: &mut ExtractionState,
) {
    let Some(name) = get_name(symbol_node, source) else {
        return;
    };
    let qualified = name.clone();

    extract_class_methods(class_node, source, options, &qualified, state);
    state.push_symbol(symbol_node, source, name, qualified.clone(), "class", None);
    extract_header_relations(class_node, source, &qualified, "extends", "extends", state);
    extract_header_relations(
        class_node,
        source,
        &qualified,
        "implements",
        "implements",
        state,
    );
    collect_signature_type_refs(class_node, source, options, state);
}

fn extract_class_methods(
    class_node: Node,
    source: &str,
    options: JsTsOptions,
    parent: &str,
    state: &mut ExtractionState,
) {
    let Some(body) = class_node.child_by_field_name("body") else {
        return;
    };

    let mut cursor = body.walk();
    for child in body.named_children(&mut cursor) {
        match child.kind() {
            "method_definition" | "method_signature" | "abstract_method_signature" => {
                extract_method(child, source, options, parent, state);
            }
            "public_field_definition" | "field_definition" => {
                extract_field_function(child, source, options, parent, state);
            }
            _ => {}
        }
    }
}

fn extract_method(
    node: Node,
    source: &str,
    options: JsTsOptions,
    parent: &str,
    state: &mut ExtractionState,
) {
    let Some(name) = get_name(node, source) else {
        return;
    };
    let qualified = format!("{parent}::{name}");
    state.push_symbol(
        node,
        source,
        name,
        qualified.clone(),
        "method",
        Some(parent.to_string()),
    );
    collect_signature_type_refs(node, source, options, state);
    if let Some(body) = node.child_by_field_name("body") {
        collect_expression_refs(body, source, options, state);
    }
}

fn extract_field_function(
    node: Node,
    source: &str,
    options: JsTsOptions,
    parent: &str,
    state: &mut ExtractionState,
) {
    let Some(name) = get_name(node, source) else {
        return;
    };
    let Some(value) = node.child_by_field_name("value") else {
        return;
    };
    if !matches!(value.kind(), "arrow_function" | "function_expression") {
        return;
    }

    let qualified = format!("{parent}::{name}");
    state.push_symbol(
        node,
        source,
        name,
        qualified.clone(),
        "method",
        Some(parent.to_string()),
    );
    collect_signature_type_refs(node, source, options, state);
    if let Some(body) = value.child_by_field_name("body") {
        collect_expression_refs(body, source, options, state);
    }
}

fn extract_binding_declaration(
    node: Node,
    source: &str,
    options: JsTsOptions,
    state: &mut ExtractionState,
) {
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        if child.kind() != "variable_declarator" {
            continue;
        }
        let Some(name) = get_simple_binding_name(child, source) else {
            continue;
        };
        let Some(value) = child.child_by_field_name("value") else {
            continue;
        };

        match value.kind() {
            "arrow_function" | "function_expression" | "generator_function" => {
                state.push_symbol(child, source, name.clone(), name, "function", None);
                collect_signature_type_refs(child, source, options, state);
                if let Some(body) = value.child_by_field_name("body") {
                    collect_expression_refs(body, source, options, state);
                }
            }
            "class" => extract_class_declaration(value, child, source, options, state),
            _ => collect_expression_refs(value, source, options, state),
        }
    }
}

fn collect_signature_type_refs(
    node: Node,
    source: &str,
    options: JsTsOptions,
    state: &mut ExtractionState,
) {
    if !options.type_syntax {
        return;
    }
    collect_type_refs(node, source, state);
}

fn collect_type_refs(node: Node, source: &str, state: &mut ExtractionState) {
    match node.kind() {
        "predefined_type" | "statement_block" | "class_body" | "import_statement"
        | "export_statement" | "extends_clause" | "implements_clause" | "class_heritage" => return,
        "required_parameter" | "optional_parameter" | "type_parameter" => {
            collect_type_refs_after_parameter_name(node, source, state);
            return;
        }
        "identifier"
        | "type_identifier"
        | "qualified_type_identifier"
        | "nested_type_identifier" => {
            state.push_type_ref(node, source);
            return;
        }
        "generic_type" => {
            if let Some(type_node) = node.child_by_field_name("type") {
                state.push_type_ref(type_node, source);
            } else if let Some(symbol) =
                first_reference_symbol(&node_text(node, source), node.start_byte())
            {
                // L-0046: tree-sitter-typescript may expose generic bases only in node text.
                state.push_ref_span(
                    symbol.span_start,
                    symbol.span_end,
                    symbol.name.clone(),
                    Some(symbol.name),
                    "type",
                    "fuzzy_name",
                );
            }
        }
        _ => {}
    }

    let name_node = node.child_by_field_name("name");
    let body_node = node.child_by_field_name("body");
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        if same_node(Some(child), name_node) || same_node(Some(child), body_node) {
            continue;
        }
        collect_type_refs(child, source, state);
    }
}

fn collect_type_refs_after_parameter_name(node: Node, source: &str, state: &mut ExtractionState) {
    let mut skipped_name = false;
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        if !skipped_name && matches!(child.kind(), "identifier" | "type_identifier") {
            skipped_name = true;
            continue;
        }
        collect_type_refs(child, source, state);
    }
}

fn collect_expression_refs(
    node: Node,
    source: &str,
    options: JsTsOptions,
    state: &mut ExtractionState,
) {
    match node.kind() {
        "call_expression" => {
            collect_call_ref(node, source, options, state);
            if let Some(arguments) = node.child_by_field_name("arguments") {
                collect_expression_refs(arguments, source, options, state);
            }
            return;
        }
        "new_expression" => {
            collect_new_ref(node, source, state);
        }
        "function_declaration"
        | "generator_function_declaration"
        | "class_declaration"
        | "abstract_class_declaration"
        | "interface_declaration"
        | "type_alias_declaration"
        | "enum_declaration"
        | "import_statement"
        | "export_statement" => return,
        _ => {}
    }

    if options.type_syntax && is_type_context_node(node) {
        collect_type_refs(node, source, state);
        return;
    }

    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        collect_expression_refs(child, source, options, state);
    }
}

fn collect_call_ref(node: Node, source: &str, options: JsTsOptions, state: &mut ExtractionState) {
    let Some(function) = node.child_by_field_name("function") else {
        return;
    };
    collect_callable_ref(function, source, options, state);
}

fn collect_new_ref(node: Node, source: &str, state: &mut ExtractionState) {
    let constructor = node
        .child_by_field_name("constructor")
        .or_else(|| node.child_by_field_name("function"))
        .or_else(|| first_named_child(node));
    let Some(constructor) = constructor else {
        return;
    };
    if matches!(
        constructor.kind(),
        "identifier" | "member_expression" | "new_expression"
    ) {
        state.push_ref(
            constructor,
            source,
            Some(normalize_reference_name(&reference_text(
                constructor,
                source,
            ))),
            "call",
            confidence_for_reference(&reference_text(constructor, source)),
        );
    }
}

fn collect_callable_ref(
    function: Node,
    source: &str,
    options: JsTsOptions,
    state: &mut ExtractionState,
) {
    match function.kind() {
        "identifier" => {
            let name = node_text(function, source);
            state.push_ref(function, source, Some(name), "call", "fuzzy_name");
        }
        "member_expression" | "optional_chain" | "subscript_expression" => {
            if let Some(property) = member_property_node(function) {
                state.push_ref(property, source, None, "call", "fuzzy_name");
            }
            if let Some(object) = function.child_by_field_name("object") {
                collect_expression_refs(object, source, options, state);
            }
        }
        "call_expression" => collect_call_ref(function, source, options, state),
        "parenthesized_expression" => {
            if let Some(child) = first_named_child(function) {
                collect_callable_ref(child, source, options, state);
            }
        }
        _ => collect_expression_refs(function, source, options, state),
    }
}

fn extract_header_relations(
    node: Node,
    source: &str,
    from_qualified: &str,
    keyword: &str,
    relation_kind: &'static str,
    state: &mut ExtractionState,
) {
    let header_end = node
        .child_by_field_name("body")
        .map_or_else(|| node.end_byte(), |body| body.start_byte());
    let Some(header) = source.get(node.start_byte()..header_end) else {
        return;
    };
    let Some(clause) = keyword_clause(header, node.start_byte(), keyword) else {
        return;
    };
    for target in relation_targets(clause.text, clause.start) {
        state.push_relation_span(
            target.span_start,
            target.span_end,
            from_qualified,
            target.name,
            relation_kind,
        );
    }
}

fn get_name(node: Node, source: &str) -> Option<String> {
    node.child_by_field_name("name")
        .map(|name| node_text(name, source))
        .filter(|name| !name.is_empty())
}

fn get_simple_binding_name(node: Node, source: &str) -> Option<String> {
    let name_node = node.child_by_field_name("name")?;
    if name_node.kind() != "identifier" {
        return None;
    }
    let name = node_text(name_node, source);
    (!name.is_empty()).then_some(name)
}

fn member_property_node(node: Node) -> Option<Node> {
    node.child_by_field_name("property")
        .or_else(|| node.child_by_field_name("field"))
        .or_else(|| node.child_by_field_name("name"))
        .or_else(|| last_named_child(node))
}

fn first_named_child(node: Node) -> Option<Node> {
    let mut cursor = node.walk();
    node.named_children(&mut cursor).next()
}

fn last_named_child(node: Node) -> Option<Node> {
    let mut cursor = node.walk();
    node.named_children(&mut cursor).last()
}

fn same_node(left: Option<Node>, right: Option<Node>) -> bool {
    match (left, right) {
        (Some(left), Some(right)) => {
            left.start_byte() == right.start_byte()
                && left.end_byte() == right.end_byte()
                && left.kind() == right.kind()
        }
        _ => false,
    }
}

fn is_type_context_node(node: Node) -> bool {
    matches!(
        node.kind(),
        "type_annotation"
            | "return_type"
            | "type_parameters"
            | "type_arguments"
            | "required_parameter"
            | "optional_parameter"
    )
}

fn node_text(node: Node, source: &str) -> String {
    node.utf8_text(source.as_bytes())
        .unwrap_or("")
        .trim()
        .to_string()
}

fn reference_text(node: Node, source: &str) -> String {
    match node.kind() {
        "generic_type" => node.child_by_field_name("type").map_or_else(
            || node_text(node, source),
            |type_node| node_text(type_node, source),
        ),
        "member_expression" => member_property_node(node).map_or_else(
            || node_text(node, source),
            |property| node_text(property, source),
        ),
        _ => node_text(node, source),
    }
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
    signature
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .trim_end_matches('=')
        .trim()
        .to_string()
}

fn normalize_path(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

fn string_literal_value(node: Node, source: &str) -> String {
    node_text(node, source)
        .trim_matches('"')
        .trim_matches('\'')
        .to_string()
}

fn normalize_reference_name(name: &str) -> String {
    let without_generics = name.split('<').next().unwrap_or(name);
    without_generics
        .trim()
        .trim_start_matches("this.")
        .trim_start_matches("super.")
        .replace("?.", ".")
}

fn target_name_from_text(text: &str) -> Option<String> {
    let normalized = normalize_reference_name(text);
    normalized
        .rsplit(['.', ':', '/'])
        .next()
        .map(str::trim)
        .map(str::to_string)
        .filter(|name| !name.is_empty() && name != "*")
}

fn confidence_for_reference(name: &str) -> &'static str {
    if name.contains('.') || name.contains('/') {
        "import_resolved"
    } else {
        "fuzzy_name"
    }
}

fn import_qualified_name(target_path: &str, target_symbol: &str) -> String {
    format!("{target_path}::{target_symbol}")
}

fn is_ignored_type_name(name: &str) -> bool {
    matches!(
        name,
        "any"
            | "bigint"
            | "boolean"
            | "false"
            | "never"
            | "null"
            | "number"
            | "object"
            | "string"
            | "symbol"
            | "true"
            | "undefined"
            | "unknown"
            | "void"
    )
}

#[derive(Debug, Clone, Copy)]
struct Clause<'a> {
    text: &'a str,
    start: usize,
}

fn clause_before_source<'a>(
    node: Node,
    source_node: Node,
    source: &'a str,
    leading_keyword: &str,
) -> (&'a str, usize) {
    let before_start = node.start_byte();
    let before_end = source_node.start_byte();
    let Some(before) = source.get(before_start..before_end) else {
        return ("", before_start);
    };

    let start =
        find_keyword(before, leading_keyword, 0).map_or(0, |index| index + leading_keyword.len());
    let end = find_keyword(before, "from", start).unwrap_or(before.len());
    trim_ascii_range(before, before_start, start, end)
}

fn keyword_clause<'a>(header: &'a str, header_start: usize, keyword: &str) -> Option<Clause<'a>> {
    let keyword_index = find_keyword(header, keyword, 0)?;
    let start = keyword_index + keyword.len();
    let mut end = header.len();
    for next_keyword in ["extends", "implements"] {
        if next_keyword == keyword {
            continue;
        }
        if let Some(next_index) = find_keyword(header, next_keyword, start) {
            end = end.min(next_index);
        }
    }
    let (text, absolute_start) = trim_ascii_range(header, header_start, start, end);
    Some(Clause {
        text,
        start: absolute_start,
    })
}

fn find_keyword(text: &str, keyword: &str, start_at: usize) -> Option<usize> {
    let mut search_from = start_at;
    while search_from < text.len() {
        let relative = text.get(search_from..)?.find(keyword)?;
        let index = search_from + relative;
        let before = index
            .checked_sub(1)
            .and_then(|pos| text.as_bytes().get(pos))
            .is_none_or(|byte| !is_identifier_byte(*byte));
        let after_index = index + keyword.len();
        let after = text
            .as_bytes()
            .get(after_index)
            .is_none_or(|byte| !is_identifier_byte(*byte));
        if before && after {
            return Some(index);
        }
        search_from = after_index;
    }
    None
}

fn trim_ascii_range(text: &str, base: usize, mut start: usize, mut end: usize) -> (&str, usize) {
    let bytes = text.as_bytes();
    while start < end && bytes[start].is_ascii_whitespace() {
        start += 1;
    }
    while end > start && (bytes[end - 1].is_ascii_whitespace() || bytes[end - 1] == b';') {
        end -= 1;
    }
    (&text[start..end], base + start)
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ImportSymbol {
    name: String,
    span_start: usize,
    span_end: usize,
}

fn import_symbols_from_clause(clause: &str, clause_start: usize) -> Vec<ImportSymbol> {
    let (clause, clause_start) = strip_leading_type(clause, clause_start);
    let mut symbols = Vec::new();

    if let Some((open, close)) = find_top_level_braces(clause) {
        symbols.extend(import_symbols_from_plain_clause(
            &clause[..open],
            clause_start,
        ));
        symbols.extend(import_symbols_from_named_list(
            &clause[open + 1..close],
            clause_start + open + 1,
        ));
        symbols.extend(import_symbols_from_plain_clause(
            &clause[close + 1..],
            clause_start + close + 1,
        ));
    } else {
        symbols.extend(import_symbols_from_plain_clause(clause, clause_start));
    }

    symbols
}

fn strip_leading_type(clause: &str, clause_start: usize) -> (&str, usize) {
    let trimmed = clause.trim_start();
    let trim_delta = clause.len().saturating_sub(trimmed.len());
    let mut start = clause_start + trim_delta;
    let mut text = trimmed;
    if text.starts_with("type ") {
        text = &text["type ".len()..];
        start += "type ".len();
    }
    (text, start)
}

fn find_top_level_braces(text: &str) -> Option<(usize, usize)> {
    let mut depth = 0usize;
    let mut open = None;
    for (index, ch) in text.char_indices() {
        match ch {
            '{' if depth == 0 => {
                open = Some(index);
                depth = 1;
            }
            '{' => depth += 1,
            '}' if depth == 1 => return open.map(|open| (open, index)),
            '}' if depth > 1 => depth -= 1,
            _ => {}
        }
    }
    None
}

fn import_symbols_from_named_list(list: &str, list_start: usize) -> Vec<ImportSymbol> {
    split_top_level(list, list_start)
        .into_iter()
        .filter_map(|part| identifier_symbol(part.text, part.start))
        .collect()
}

fn import_symbols_from_plain_clause(clause: &str, clause_start: usize) -> Vec<ImportSymbol> {
    split_top_level(clause, clause_start)
        .into_iter()
        .filter_map(|part| {
            let trimmed = part.text.trim();
            if trimmed.is_empty() || trimmed == "*" {
                None
            } else {
                identifier_symbol(part.text, part.start)
            }
        })
        .collect()
}

#[derive(Debug, Clone, Copy)]
struct Part<'a> {
    text: &'a str,
    start: usize,
}

fn split_top_level(text: &str, base: usize) -> Vec<Part<'_>> {
    let mut parts = Vec::new();
    let mut depth = 0usize;
    let mut start = 0usize;
    for (index, ch) in text.char_indices() {
        match ch {
            '<' | '(' | '[' | '{' => depth += 1,
            '>' | ')' | ']' | '}' => depth = depth.saturating_sub(1),
            ',' if depth == 0 => {
                push_part(text, base, start, index, &mut parts);
                start = index + 1;
            }
            _ => {}
        }
    }
    push_part(text, base, start, text.len(), &mut parts);
    parts
}

fn push_part<'a>(
    text: &'a str,
    base: usize,
    mut start: usize,
    mut end: usize,
    parts: &mut Vec<Part<'a>>,
) {
    let bytes = text.as_bytes();
    while start < end && bytes[start].is_ascii_whitespace() {
        start += 1;
    }
    while end > start && bytes[end - 1].is_ascii_whitespace() {
        end -= 1;
    }
    if start < end {
        parts.push(Part {
            text: &text[start..end],
            start: base + start,
        });
    }
}

fn identifier_symbol(text: &str, base: usize) -> Option<ImportSymbol> {
    let mut selected = None;
    for (relative_start, relative_end) in identifier_spans(text) {
        let name = &text[relative_start..relative_end];
        if matches!(name, "as" | "from" | "import" | "export" | "type") {
            continue;
        }
        selected = Some(ImportSymbol {
            name: name.to_string(),
            span_start: base + relative_start,
            span_end: base + relative_end,
        });
    }
    selected
}

fn relation_targets(text: &str, base: usize) -> Vec<ImportSymbol> {
    split_top_level(text, base)
        .into_iter()
        .filter_map(|part| first_reference_symbol(part.text, part.start))
        .collect()
}

fn first_reference_symbol(text: &str, base: usize) -> Option<ImportSymbol> {
    for (relative_start, relative_end) in identifier_spans(text) {
        let name = &text[relative_start..relative_end];
        if matches!(name, "extends" | "implements" | "type" | "as") {
            continue;
        }
        let mut end = relative_end;
        let mut cursor = relative_end;
        while text.get(cursor..).is_some_and(|rest| rest.starts_with('.')) {
            let next_start = cursor + 1;
            let Some((next_relative_start, next_relative_end)) =
                identifier_spans(&text[next_start..]).next()
            else {
                break;
            };
            if next_relative_start != 0 {
                break;
            }
            cursor = next_start + next_relative_end;
            end = cursor;
        }
        return Some(ImportSymbol {
            name: text[relative_start..end].to_string(),
            span_start: base + relative_start,
            span_end: base + end,
        });
    }
    None
}

fn identifier_spans(text: &str) -> impl Iterator<Item = (usize, usize)> + '_ {
    let mut index = 0usize;
    std::iter::from_fn(move || {
        while index < text.len() {
            let ch = text[index..].chars().next()?;
            if is_identifier_start(ch) {
                let start = index;
                index += ch.len_utf8();
                while index < text.len() {
                    let ch = text[index..].chars().next()?;
                    if !is_identifier_continue(ch) {
                        break;
                    }
                    index += ch.len_utf8();
                }
                return Some((start, index));
            }
            index += ch.len_utf8();
        }
        None
    })
}

fn is_identifier_start(ch: char) -> bool {
    ch == '_' || ch == '$' || ch.is_ascii_alphabetic()
}

fn is_identifier_continue(ch: char) -> bool {
    is_identifier_start(ch) || ch.is_ascii_digit()
}

fn is_identifier_byte(byte: u8) -> bool {
    byte == b'_' || byte == b'$' || byte.is_ascii_alphanumeric()
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
