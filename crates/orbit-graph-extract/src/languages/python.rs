//! Python tree-sitter extraction.

use std::path::Path;

use tree_sitter::{Node, Parser};

use crate::{ExtractedFile, Extractor, RawCommand, RawImport, RawRef, RawRelation, RawSymbol};

/// Extracts Python source files into raw graph rows.
pub struct PythonExtractor;

impl Extractor for PythonExtractor {
    fn lang(&self) -> &'static str {
        "python"
    }

    fn supports(&self, path: &Path) -> bool {
        path.extension().and_then(|ext| ext.to_str()) == Some("py")
    }

    fn extract(&self, path: &Path, bytes: &[u8]) -> ExtractedFile {
        let Ok(source) = std::str::from_utf8(bytes) else {
            return ExtractedFile::default();
        };

        let mut parser = Parser::new();
        if parser
            .set_language(&tree_sitter_python::LANGUAGE.into())
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
    commands: Vec<RawCommand>,
}

impl ExtractionState {
    fn new(path: &Path) -> Self {
        Self {
            file_path: normalize_path(path),
            symbols: Vec::new(),
            refs: Vec::new(),
            relations: Vec::new(),
            imports: Vec::new(),
            commands: Vec::new(),
        }
    }

    fn finish(mut self) -> ExtractedFile {
        dedup_symbols(&mut self.symbols);
        dedup_refs(&mut self.refs);
        dedup_relations(&mut self.relations);
        dedup_imports(&mut self.imports);
        dedup_commands(&mut self.commands);
        ExtractedFile {
            symbols: self.symbols,
            refs: self.refs,
            relations: self.relations,
            imports: self.imports,
            strings: Vec::new(),
            configs: Vec::new(),
            commands: self.commands,
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
        if is_ignored_name(&target_name) {
            return;
        }

        self.push_ref_span(
            node.start_byte(),
            node.end_byte(),
            target_name,
            target_qualified,
            kind,
            confidence,
        );
    }

    fn push_ref_span(
        &mut self,
        from_span_start: usize,
        from_span_end: usize,
        target_name: String,
        target_qualified: Option<String>,
        kind: &'static str,
        confidence: &'static str,
    ) {
        if target_name.is_empty() || is_ignored_name(&target_name) {
            return;
        }

        self.refs.push(RawRef {
            from_file: self.file_path.clone(),
            from_span_start,
            from_span_end,
            target_name,
            target_qualified,
            kind: kind.to_string(),
            confidence: confidence.to_string(),
        });
    }

    fn push_command(&mut self, name: String, span_start: usize, handler_symbol: Option<String>) {
        if name.is_empty() {
            return;
        }

        self.commands.push(RawCommand {
            name,
            file_path: self.file_path.clone(),
            span_start,
            handler_symbol,
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
            "function_definition" => extract_function(child, source, parent_symbol, state),
            "class_definition" => extract_class(child, source, parent_symbol, state),
            "decorated_definition" => extract_decorated(child, source, parent_symbol, state),
            "import_statement" => extract_import(child, source, state),
            "import_from_statement" | "future_import_statement" => {
                extract_from_import(child, source, state);
            }
            _ => collect_expression_refs(child, source, parent_symbol, state),
        }
    }
}

fn extract_decorated(
    node: Node,
    source: &str,
    parent_symbol: Option<&str>,
    state: &mut ExtractionState,
) {
    if let Some(definition) = node.child_by_field_name("definition") {
        match definition.kind() {
            "function_definition" => {
                extract_function(definition, source, parent_symbol, state);
                extract_click_commands(node, definition, source, parent_symbol, state);
            }
            "class_definition" => extract_class(definition, source, parent_symbol, state),
            _ => {}
        }
    }
}

fn extract_click_commands(
    decorated: Node,
    function: Node,
    source: &str,
    parent_symbol: Option<&str>,
    state: &mut ExtractionState,
) {
    let Some(function_name) = get_name(function, source) else {
        return;
    };
    let handler_symbol = qualify_name(parent_symbol, &function_name);

    let mut cursor = decorated.walk();
    for child in decorated.named_children(&mut cursor) {
        if child.start_byte() >= function.start_byte() {
            break;
        }
        if child.kind() != "decorator" {
            continue;
        }

        match click_command_from_decorator(child, source, &function_name) {
            Some(CommandName::Resolved(name)) => {
                state.push_command(name, child.start_byte(), Some(handler_symbol.clone()));
            }
            Some(CommandName::Unresolved {
                span_start,
                span_end,
                target_name,
            }) => state.push_ref_span(
                span_start,
                span_end,
                target_name,
                None,
                "command",
                "fuzzy_name",
            ),
            None => {}
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum CommandName {
    Resolved(String),
    Unresolved {
        span_start: usize,
        span_end: usize,
        target_name: String,
    },
}

fn click_command_from_decorator(
    node: Node,
    source: &str,
    fallback_function_name: &str,
) -> Option<CommandName> {
    let text = node_text(node, source);
    let at_index = text.find('@')?;
    let body_start = at_index + 1;
    let body = text.get(body_start..)?.trim_start();
    let body_leading_ws = text.get(body_start..)?.len() - body.len();
    let body_start = body_start + body_leading_ws;

    let Some(open_index_in_body) = body.find('(') else {
        return is_click_command_target(body)
            .then(|| CommandName::Resolved(click_default_command_name(fallback_function_name)));
    };

    let target = body.get(..open_index_in_body)?.trim();
    if !is_click_command_target(target) {
        return None;
    }

    let args_start = body_start + open_index_in_body + 1;
    let close_index = text.rfind(')').unwrap_or(text.len());
    let args = text.get(args_start..close_index).unwrap_or_default();
    command_name_from_click_args(args, node.start_byte() + args_start, fallback_function_name)
}

fn is_click_command_target(target: &str) -> bool {
    matches!(
        target.rsplit('.').next().map(str::trim),
        Some("command" | "group")
    )
}

fn command_name_from_click_args(
    args: &str,
    args_start: usize,
    fallback_function_name: &str,
) -> Option<CommandName> {
    for arg in split_top_level_args(args, args_start) {
        let trimmed = arg.text.trim();
        if trimmed.is_empty() {
            continue;
        }

        let trimmed_start = arg.start + leading_whitespace_len(arg.text);
        if let Some((key, value, value_start)) = split_keyword_arg(trimmed, trimmed_start) {
            if key == "name" {
                return command_name_from_value(value, value_start);
            }
            continue;
        }

        return command_name_from_value(trimmed, trimmed_start);
    }

    Some(CommandName::Resolved(click_default_command_name(
        fallback_function_name,
    )))
}

fn command_name_from_value(value: &str, value_start: usize) -> Option<CommandName> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return None;
    }
    let span_start = value_start + leading_whitespace_len(value);

    if let Some(literal) = python_string_literal_value(trimmed) {
        return Some(CommandName::Resolved(literal));
    }

    let target_name = unresolved_command_target_name(trimmed)?;
    Some(CommandName::Unresolved {
        span_start,
        span_end: span_start + trimmed.len(),
        target_name,
    })
}

#[derive(Debug, Clone, Copy)]
struct ArgSpan<'a> {
    text: &'a str,
    start: usize,
}

fn split_top_level_args(args: &str, args_start: usize) -> Vec<ArgSpan<'_>> {
    let mut spans = Vec::new();
    let mut start = 0usize;
    let mut depth = 0usize;
    let mut quote = None;
    let mut escaped = false;

    for (index, ch) in args.char_indices() {
        if let Some(quote_char) = quote {
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == quote_char {
                quote = None;
            }
            continue;
        }

        match ch {
            '\'' | '"' => quote = Some(ch),
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => depth = depth.saturating_sub(1),
            ',' if depth == 0 => {
                spans.push(ArgSpan {
                    text: &args[start..index],
                    start: args_start + start,
                });
                start = index + ch.len_utf8();
            }
            _ => {}
        }
    }

    spans.push(ArgSpan {
        text: &args[start..],
        start: args_start + start,
    });
    spans
}

fn split_keyword_arg(arg: &str, arg_start: usize) -> Option<(&str, &str, usize)> {
    let equals_index = arg.find('=')?;
    let key = arg.get(..equals_index)?.trim();
    if key.is_empty()
        || !key
            .chars()
            .all(|ch| ch == '_' || ch.is_ascii_alphanumeric())
    {
        return None;
    }
    let value_start = arg_start + equals_index + 1;
    let value = arg.get(equals_index + 1..)?;
    Some((key, value, value_start))
}

fn python_string_literal_value(value: &str) -> Option<String> {
    let mut literal = value.trim();
    loop {
        let first = literal.chars().next()?;
        if matches!(first, 'r' | 'R' | 'u' | 'U' | 'b' | 'B') {
            literal = literal.get(first.len_utf8()..)?;
            continue;
        }
        if matches!(first, 'f' | 'F') {
            return None;
        }
        break;
    }

    let quote = literal.chars().next()?;
    if !matches!(quote, '\'' | '"') {
        return None;
    }
    let rest = literal.get(quote.len_utf8()..)?;
    let end = rest.rfind(quote)?;
    Some(rest.get(..end)?.to_string())
}

fn unresolved_command_target_name(value: &str) -> Option<String> {
    let token_end = value
        .find(|ch: char| !(ch == '_' || ch == '.' || ch.is_ascii_alphanumeric()))
        .unwrap_or(value.len());
    value
        .get(..token_end)?
        .rsplit('.')
        .next()
        .map(str::to_string)
        .filter(|name| !name.is_empty())
}

fn click_default_command_name(function_name: &str) -> String {
    function_name.replace('_', "-")
}

fn leading_whitespace_len(value: &str) -> usize {
    value.len() - value.trim_start().len()
}

fn extract_function(
    node: Node,
    source: &str,
    parent_symbol: Option<&str>,
    state: &mut ExtractionState,
) {
    let Some(name) = get_name(node, source) else {
        return;
    };

    let qualified = qualify_name(parent_symbol, &name);
    let kind = if parent_symbol.is_some() {
        "method"
    } else {
        "function"
    };
    state.push_symbol(
        node,
        source,
        name,
        qualified,
        kind,
        parent_symbol.map(ToOwned::to_owned),
    );

    collect_function_type_refs(node, source, state);
    if let Some(body) = node.child_by_field_name("body") {
        collect_expression_refs(body, source, parent_symbol, state);
    }
}

fn extract_class(
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
        qualified.clone(),
        "class",
        parent_symbol.map(ToOwned::to_owned),
    );
    extract_superclasses(node, source, &qualified, state);

    if let Some(body) = node.child_by_field_name("body") {
        extract_children(body, source, Some(&qualified), state);
    }
}

fn extract_superclasses(
    node: Node,
    source: &str,
    class_qualified: &str,
    state: &mut ExtractionState,
) {
    let Some(superclasses) = node.child_by_field_name("superclasses") else {
        return;
    };

    let mut cursor = superclasses.walk();
    for superclass in superclasses.named_children(&mut cursor) {
        let Some(target) = type_reference_text(superclass, source) else {
            continue;
        };
        if target.is_empty() || is_ignored_name(&target) {
            continue;
        }

        state.relations.push(RawRelation {
            from_qualified: class_qualified.to_string(),
            to_qualified: target.clone(),
            kind: "extends".to_string(),
            def_file: state.file_path.clone(),
            def_span_start: superclass.start_byte(),
            def_span_end: superclass.end_byte(),
            confidence: confidence_for_name(&target).to_string(),
        });
        state.push_ref(
            superclass,
            source,
            Some(target),
            "type",
            confidence_for_name(&node_text(superclass, source)),
        );
    }
}

fn collect_function_type_refs(node: Node, source: &str, state: &mut ExtractionState) {
    if let Some(parameters) = node.child_by_field_name("parameters") {
        collect_parameter_type_refs(parameters, source, state);
    }
    if let Some(return_type) = node.child_by_field_name("return_type") {
        collect_type_refs(return_type, source, state);
    }
}

fn collect_parameter_type_refs(node: Node, source: &str, state: &mut ExtractionState) {
    if let Some(type_node) = node.child_by_field_name("type") {
        collect_type_refs(type_node, source, state);
    }

    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        collect_parameter_type_refs(child, source, state);
    }
}

fn collect_type_refs(node: Node, source: &str, state: &mut ExtractionState) {
    if let Some(target) = type_reference_text(node, source)
        && !target.is_empty()
        && !is_ignored_name(&target)
    {
        state.push_ref(
            type_span_node(node, source),
            source,
            Some(target.clone()),
            "type",
            confidence_for_name(&target),
        );
    }

    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        if child.kind() != "identifier" {
            collect_type_refs(child, source, state);
        }
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
            collect_call_ref(node, source, parent_symbol, state);
            if let Some(arguments) = node.child_by_field_name("arguments") {
                collect_expression_refs(arguments, source, parent_symbol, state);
            }
            return;
        }
        "function_definition"
        | "class_definition"
        | "decorated_definition"
        | "import_statement"
        | "import_from_statement"
        | "future_import_statement" => return,
        _ => {}
    }

    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        collect_expression_refs(child, source, parent_symbol, state);
    }
}

fn collect_call_ref(
    node: Node,
    source: &str,
    parent_symbol: Option<&str>,
    state: &mut ExtractionState,
) {
    let Some(function) = node.child_by_field_name("function") else {
        return;
    };

    match function.kind() {
        "identifier" => {
            let name = node_text(function, source);
            state.push_ref(
                function,
                source,
                Some(qualify_name(parent_symbol, &name)),
                "call",
                "fuzzy_name",
            );
        }
        "attribute" => {
            if let Some(attribute) = function.child_by_field_name("attribute") {
                state.push_ref(attribute, source, None, "call", "fuzzy_name");
            }
        }
        _ => collect_expression_refs(function, source, parent_symbol, state),
    }
}

fn extract_import(node: Node, source: &str, state: &mut ExtractionState) {
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        match child.kind() {
            "aliased_import" => {
                let Some(name_node) = child.child_by_field_name("name") else {
                    continue;
                };
                let Some(alias_node) = child.child_by_field_name("alias") else {
                    continue;
                };
                let target_path = normalized_import_path(&node_text(name_node, source));
                let alias = node_text(alias_node, source);
                state.imports.push(RawImport {
                    from_file: state.file_path.clone(),
                    target_path: target_path.clone(),
                    target_symbol: Some(alias),
                });
                state.push_ref(
                    alias_node,
                    source,
                    Some(target_path),
                    "use",
                    "import_resolved",
                );
            }
            "dotted_name" => {
                let target_path = normalized_import_path(&node_text(child, source));
                state.imports.push(RawImport {
                    from_file: state.file_path.clone(),
                    target_path: target_path.clone(),
                    target_symbol: None,
                });
                state.push_ref(child, source, Some(target_path), "use", "import_resolved");
            }
            _ => {}
        }
    }
}

fn extract_from_import(node: Node, source: &str, state: &mut ExtractionState) {
    let Some(module_node) = node.child_by_field_name("module_name") else {
        return;
    };
    let module_span = (module_node.start_byte(), module_node.end_byte());
    let target_path = normalized_import_path(&node_text(module_node, source));

    let mut saw_import = false;
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        if (child.start_byte(), child.end_byte()) == module_span {
            continue;
        }

        match child.kind() {
            "aliased_import" => {
                if let Some((span_node, imported_name, target_symbol)) =
                    import_symbol_parts(child, source)
                {
                    push_from_import_rows(span_node, source, &target_path, target_symbol, state);
                    state.push_ref(
                        span_node,
                        source,
                        Some(format!("{target_path}.{imported_name}")),
                        "use",
                        "import_resolved",
                    );
                    saw_import = true;
                }
            }
            "dotted_name" => {
                let imported_name = node_text(child, source);
                let target_symbol = import_leaf_name(&imported_name);
                push_from_import_rows(child, source, &target_path, Some(target_symbol), state);
                state.push_ref(
                    child,
                    source,
                    Some(format!("{target_path}.{imported_name}")),
                    "use",
                    "import_resolved",
                );
                saw_import = true;
            }
            "wildcard_import" => {
                push_from_import_rows(child, source, &target_path, None, state);
                saw_import = true;
            }
            _ => {}
        }
    }

    if !saw_import {
        state.imports.push(RawImport {
            from_file: state.file_path.clone(),
            target_path,
            target_symbol: None,
        });
    }
}

fn import_symbol_parts<'tree>(
    node: Node<'tree>,
    source: &str,
) -> Option<(Node<'tree>, String, Option<String>)> {
    let name_node = node.child_by_field_name("name")?;
    let imported_name = node_text(name_node, source);
    let alias_node = node.child_by_field_name("alias");
    let target_symbol = alias_node
        .map(|alias| node_text(alias, source))
        .or_else(|| Some(import_leaf_name(&imported_name)));
    Some((
        alias_node.unwrap_or(name_node),
        imported_name,
        target_symbol,
    ))
}

fn push_from_import_rows(
    span_node: Node,
    source: &str,
    target_path: &str,
    target_symbol: Option<String>,
    state: &mut ExtractionState,
) {
    state.imports.push(RawImport {
        from_file: state.file_path.clone(),
        target_path: target_path.to_string(),
        target_symbol,
    });
    if node_text(span_node, source) == "*" {
        state.push_ref(span_node, source, None, "use", "import_resolved");
    }
}

fn get_name(node: Node, source: &str) -> Option<String> {
    node.child_by_field_name("name")
        .map(|name| node_text(name, source))
        .filter(|name| !name.is_empty())
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
        Some(parent) => format!("{parent}.{name}"),
        None => name.to_string(),
    }
}

fn normalized_import_path(path: &str) -> String {
    path.split_whitespace().collect::<String>()
}

fn normalize_qualified_name(name: &str) -> String {
    name.split('.')
        .map(str::trim)
        .filter(|segment| !segment.is_empty())
        .collect::<Vec<_>>()
        .join(".")
}

fn import_leaf_name(name: &str) -> String {
    normalize_qualified_name(name)
        .rsplit('.')
        .next()
        .unwrap_or("")
        .to_string()
}

fn type_reference_text(node: Node, source: &str) -> Option<String> {
    match node.kind() {
        "identifier" | "dotted_name" | "attribute" => {
            Some(normalize_qualified_name(&node_text(node, source)))
        }
        "type" | "generic_type" | "member_type" | "subscript" | "call" => {
            first_type_reference_child(node, source)
        }
        _ => first_type_reference_child(node, source),
    }
}

fn first_type_reference_child(node: Node, source: &str) -> Option<String> {
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        if let Some(text) = type_reference_text(child, source)
            && !text.is_empty()
        {
            return Some(text);
        }
    }
    None
}

fn type_span_node<'tree>(node: Node<'tree>, source: &str) -> Node<'tree> {
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        if matches!(child.kind(), "identifier" | "dotted_name" | "attribute") {
            return child;
        }
        let nested = type_span_node(child, source);
        if !node_text(nested, source).is_empty() {
            return nested;
        }
    }
    node
}

fn target_name(node: Node, source: &str) -> Option<String> {
    let text = normalize_qualified_name(&node_text(node, source));
    text.rsplit('.')
        .next()
        .map(str::to_string)
        .filter(|name| !name.is_empty() && name != "*")
}

fn confidence_for_name(name: &str) -> &'static str {
    if name.contains('.') {
        "import_resolved"
    } else {
        "fuzzy_name"
    }
}

fn is_ignored_name(name: &str) -> bool {
    matches!(
        name,
        "self"
            | "cls"
            | "None"
            | "True"
            | "False"
            | "object"
            | "str"
            | "int"
            | "float"
            | "bool"
            | "bytes"
            | "list"
            | "dict"
            | "tuple"
            | "set"
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

fn dedup_commands(commands: &mut Vec<RawCommand>) {
    commands.sort_by(|left, right| {
        left.span_start
            .cmp(&right.span_start)
            .then_with(|| left.name.cmp(&right.name))
            .then_with(|| left.handler_symbol.cmp(&right.handler_symbol))
    });
    commands.dedup_by(|left, right| {
        left.file_path == right.file_path
            && left.name == right.name
            && left.span_start == right.span_start
            && left.handler_symbol == right.handler_symbol
    });
}

#[cfg(test)]
#[path = "tests/python.rs"]
mod tests;
