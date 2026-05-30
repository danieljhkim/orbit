//! C# tree-sitter extraction.

use std::path::Path;

use tree_sitter::{Node, Parser};

use crate::{ExtractedFile, Extractor, RawImport, RawRef, RawRelation, RawSymbol};

/// Extracts C# source files into raw graph rows.
pub struct CSharpExtractor;

#[cfg(test)]
#[path = "tests/csharp.rs"]
mod tests;

impl Extractor for CSharpExtractor {
    fn lang(&self) -> &'static str {
        "csharp"
    }

    fn supports(&self, path: &Path) -> bool {
        path.extension().and_then(|ext| ext.to_str()) == Some("cs")
    }

    fn extract(&self, path: &Path, bytes: &[u8]) -> ExtractedFile {
        let Ok(source) = std::str::from_utf8(bytes) else {
            return ExtractedFile::default();
        };

        let mut parser = Parser::new();
        if parser
            .set_language(&tree_sitter_c_sharp::LANGUAGE.into())
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

    fn finish(self) -> ExtractedFile {
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

    fn push_relation(
        &mut self,
        node: Node,
        from_qualified: &str,
        to_qualified: String,
        kind: &'static str,
    ) {
        self.relations.push(RawRelation {
            from_qualified: from_qualified.to_string(),
            to_qualified,
            kind: kind.to_string(),
            def_file: self.file_path.clone(),
            def_span_start: node.start_byte(),
            def_span_end: node.end_byte(),
            confidence: "exact".to_string(),
        });
    }

    fn push_call_ref(&mut self, name: String, start: usize, end: usize) {
        if name.is_empty() {
            return;
        }

        self.refs.push(RawRef {
            from_file: self.file_path.clone(),
            from_span_start: start,
            from_span_end: end,
            target_name: name,
            target_qualified: None,
            kind: "call".to_string(),
            confidence: "fuzzy_name".to_string(),
        });
    }
}

fn extract_children(node: Node, source: &str, parent: Option<&str>, state: &mut ExtractionState) {
    let mut scoped_parent = parent.map(str::to_string);
    let mut cursor = node.walk();

    for child in node.named_children(&mut cursor) {
        if child.kind() == "file_scoped_namespace_declaration" {
            if let Some(qualified) = extract_file_scoped_namespace(child, source, parent, state) {
                scoped_parent = Some(qualified);
            }
            continue;
        }

        extract_node(child, source, scoped_parent.as_deref().or(parent), state);
    }
}

fn extract_node(node: Node, source: &str, parent: Option<&str>, state: &mut ExtractionState) {
    match node.kind() {
        "using_directive" => extract_using(node, source, state),
        "namespace_declaration" => extract_namespace(node, source, parent, state),
        "class_declaration" => extract_type(node, source, parent, "class", state),
        "struct_declaration" => extract_type(node, source, parent, "struct", state),
        "record_declaration" => extract_type(node, source, parent, "record", state),
        "interface_declaration" => extract_type(node, source, parent, "interface", state),
        "enum_declaration" => extract_type(node, source, parent, "enum", state),
        "method_declaration" | "constructor_declaration" => {
            extract_named_member(node, source, parent, "method", state);
        }
        "property_declaration" => extract_named_member(node, source, parent, "property", state),
        "field_declaration" => extract_variable_members(node, source, parent, "field", state),
        "event_declaration" => extract_named_member(node, source, parent, "event", state),
        "event_field_declaration" => {
            extract_variable_members(node, source, parent, "event", state);
        }
        "delegate_declaration" => extract_named_member(node, source, parent, "delegate", state),
        "compilation_unit" | "declaration_list" | "declaration" | "type_declaration"
        | "preproc_if" | "preproc_elif" | "preproc_else" => {
            extract_children(node, source, parent, state);
        }
        _ => {}
    }
}

fn extract_namespace(node: Node, source: &str, parent: Option<&str>, state: &mut ExtractionState) {
    let Some(name) = get_name(node, source) else {
        return;
    };
    let qualified = qualify_name(parent, &name);
    state.push_symbol(
        node,
        source,
        name,
        qualified.clone(),
        "namespace",
        parent.map(ToOwned::to_owned),
    );

    if let Some(body) = node.child_by_field_name("body") {
        extract_children(body, source, Some(&qualified), state);
    }
}

fn extract_file_scoped_namespace(
    node: Node,
    source: &str,
    parent: Option<&str>,
    state: &mut ExtractionState,
) -> Option<String> {
    let name = get_name(node, source)?;
    let qualified = qualify_name(parent, &name);
    state.push_symbol(
        node,
        source,
        name,
        qualified.clone(),
        "namespace",
        parent.map(ToOwned::to_owned),
    );
    Some(qualified)
}

fn extract_type(
    node: Node,
    source: &str,
    parent: Option<&str>,
    kind: &'static str,
    state: &mut ExtractionState,
) {
    let Some(name) = get_name(node, source) else {
        return;
    };
    let qualified = qualify_name(parent, &name);

    state.push_symbol(
        node,
        source,
        name,
        qualified.clone(),
        kind,
        parent.map(ToOwned::to_owned),
    );
    extract_supertype_relations(node, source, &qualified, state);

    if let Some(body) = node.child_by_field_name("body") {
        extract_children(body, source, Some(&qualified), state);
    }
}

fn extract_named_member(
    node: Node,
    source: &str,
    parent: Option<&str>,
    kind: &'static str,
    state: &mut ExtractionState,
) {
    let Some(name) = get_name(node, source) else {
        return;
    };
    let qualified = qualify_name(parent, &name);
    state.push_symbol(
        node,
        source,
        name,
        qualified,
        kind,
        parent.map(ToOwned::to_owned),
    );

    if kind == "method" {
        let body_start = node
            .child_by_field_name("body")
            .map_or_else(|| node.start_byte(), |body| body.start_byte());
        collect_dot_call_refs(source, body_start, node.end_byte(), state);
    }
}

fn extract_variable_members(
    node: Node,
    source: &str,
    parent: Option<&str>,
    kind: &'static str,
    state: &mut ExtractionState,
) {
    for name in variable_names(node, source) {
        state.push_symbol(
            node,
            source,
            name.clone(),
            qualify_name(parent, &name),
            kind,
            parent.map(ToOwned::to_owned),
        );
    }
}

fn extract_using(node: Node, source: &str, state: &mut ExtractionState) {
    let text = node_text(node, source);
    let mut target = text
        .trim()
        .strip_prefix("using")
        .unwrap_or(text.trim())
        .trim()
        .strip_prefix("static")
        .unwrap_or_else(|| {
            text.trim()
                .strip_prefix("using")
                .unwrap_or(text.trim())
                .trim()
        })
        .trim()
        .trim_end_matches(';')
        .trim();
    let mut target_symbol = import_symbol(target);

    if let Some((alias, path)) = target.split_once('=') {
        target = path.trim();
        target_symbol = Some(alias.trim().to_string());
    }
    if target.is_empty() {
        return;
    }

    state.imports.push(RawImport {
        from_file: state.file_path.clone(),
        target_path: target.to_string(),
        target_symbol,
    });
}

fn extract_supertype_relations(
    node: Node,
    source: &str,
    from_qualified: &str,
    state: &mut ExtractionState,
) {
    let Some(name_node) = node.child_by_field_name("name") else {
        return;
    };
    let header_end = body_start(node).unwrap_or_else(|| node.end_byte());
    let Some(header) = source.get(name_node.end_byte()..header_end) else {
        return;
    };
    let header =
        find_keyword_outside_groups(header, "where").map_or(header, |where_at| &header[..where_at]);
    let Some(colon_at) = find_char_outside_groups(header, ':') else {
        return;
    };

    for (index, target) in parse_type_list(&header[colon_at + 1..])
        .into_iter()
        .enumerate()
    {
        let kind = if index == 0 { "extends" } else { "implements" };
        state.push_relation(node, from_qualified, target, kind);
    }
}

fn collect_dot_call_refs(
    source: &str,
    range_start: usize,
    range_end: usize,
    state: &mut ExtractionState,
) {
    let bytes = source.as_bytes();
    let mut index = range_start;
    while index < range_end {
        if bytes.get(index) != Some(&b'.') {
            index += 1;
            continue;
        }

        let mut name_start = index + 1;
        while name_start < range_end && bytes[name_start].is_ascii_whitespace() {
            name_start += 1;
        }
        if name_start >= range_end || !is_ident_start(bytes[name_start]) {
            index += 1;
            continue;
        }

        let mut name_end = name_start + 1;
        while name_end < range_end && is_ident_continue(bytes[name_end]) {
            name_end += 1;
        }

        let mut paren = name_end;
        while paren < range_end && bytes[paren].is_ascii_whitespace() {
            paren += 1;
        }
        if bytes.get(paren) == Some(&b'(')
            && let Some(name) = source.get(name_start..name_end)
        {
            state.push_call_ref(name.to_string(), name_start, name_end);
        }
        index = name_end;
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
    let end = body_start(node).unwrap_or_else(|| node.end_byte());
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
        Some(parent) => format!("{parent}::{name}"),
        None => name.to_string(),
    }
}

fn body_start(node: Node) -> Option<usize> {
    if let Some(body) = node.child_by_field_name("body") {
        return Some(body.start_byte());
    }

    let mut cursor = node.walk();
    node.named_children(&mut cursor)
        .find(|child| {
            matches!(
                child.kind(),
                "declaration_list" | "enum_member_declaration_list"
            )
        })
        .map(|child| child.start_byte())
}

fn variable_names(node: Node, source: &str) -> Vec<String> {
    let mut names = Vec::new();
    let mut cursor = node.walk();

    for child in node.named_children(&mut cursor) {
        if child.kind() != "variable_declaration" {
            continue;
        }

        let mut declaration_cursor = child.walk();
        for declarator in child.named_children(&mut declaration_cursor) {
            if declarator.kind() != "variable_declarator" {
                continue;
            }
            if let Some(name) = get_name(declarator, source) {
                names.push(name);
            }
        }
    }

    names
}

fn parse_type_list(list: &str) -> Vec<String> {
    split_outside_groups(list, ',')
        .into_iter()
        .filter_map(normalize_type_name)
        .collect()
}

fn normalize_type_name(raw: &str) -> Option<String> {
    let raw = raw.trim();
    if raw.is_empty() {
        return None;
    }

    let mut normalized = String::new();
    let mut angle_depth = 0usize;
    for ch in raw.chars() {
        match ch {
            '<' => angle_depth += 1,
            '>' => angle_depth = angle_depth.saturating_sub(1),
            '(' | '[' | '?' if angle_depth == 0 => break,
            ch if angle_depth == 0 && ch.is_whitespace() => break,
            ch if angle_depth == 0 => normalized.push(ch),
            _ => {}
        }
    }

    let normalized = normalized
        .trim()
        .trim_end_matches('{')
        .trim_end_matches(';')
        .trim();
    (!normalized.is_empty()).then(|| normalized.to_string())
}

fn split_outside_groups(text: &str, delimiter: char) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut start = 0usize;
    let mut angle_depth = 0usize;
    let mut paren_depth = 0usize;
    let mut bracket_depth = 0usize;

    for (index, ch) in text.char_indices() {
        match ch {
            '<' => angle_depth += 1,
            '>' => angle_depth = angle_depth.saturating_sub(1),
            '(' => paren_depth += 1,
            ')' => paren_depth = paren_depth.saturating_sub(1),
            '[' => bracket_depth += 1,
            ']' => bracket_depth = bracket_depth.saturating_sub(1),
            ch if ch == delimiter && angle_depth == 0 && paren_depth == 0 && bracket_depth == 0 => {
                parts.push(&text[start..index]);
                start = index + ch.len_utf8();
            }
            _ => {}
        }
    }
    parts.push(&text[start..]);
    parts
}

fn find_char_outside_groups(text: &str, needle: char) -> Option<usize> {
    let mut angle_depth = 0usize;
    let mut paren_depth = 0usize;
    let mut bracket_depth = 0usize;

    for (index, ch) in text.char_indices() {
        match ch {
            '<' => angle_depth += 1,
            '>' => angle_depth = angle_depth.saturating_sub(1),
            '(' => paren_depth += 1,
            ')' => paren_depth = paren_depth.saturating_sub(1),
            '[' => bracket_depth += 1,
            ']' => bracket_depth = bracket_depth.saturating_sub(1),
            ch if ch == needle && angle_depth == 0 && paren_depth == 0 && bracket_depth == 0 => {
                return Some(index);
            }
            _ => {}
        }
    }
    None
}

fn find_keyword_outside_groups(text: &str, keyword: &str) -> Option<usize> {
    let bytes = text.as_bytes();
    let keyword_bytes = keyword.as_bytes();
    let mut angle_depth = 0usize;
    let mut paren_depth = 0usize;
    let mut bracket_depth = 0usize;
    let mut index = 0usize;

    while index < bytes.len() {
        match bytes[index] {
            b'<' => angle_depth += 1,
            b'>' => angle_depth = angle_depth.saturating_sub(1),
            b'(' => paren_depth += 1,
            b')' => paren_depth = paren_depth.saturating_sub(1),
            b'[' => bracket_depth += 1,
            b']' => bracket_depth = bracket_depth.saturating_sub(1),
            _ => {}
        }

        if angle_depth == 0
            && paren_depth == 0
            && bracket_depth == 0
            && bytes[index..].starts_with(keyword_bytes)
            && is_keyword_boundary(bytes, index, keyword.len())
        {
            return Some(index);
        }

        index += 1;
    }
    None
}

fn is_keyword_boundary(bytes: &[u8], start: usize, len: usize) -> bool {
    let before = start
        .checked_sub(1)
        .and_then(|index| bytes.get(index))
        .is_none_or(|byte| !is_ident_continue(*byte));
    let after = bytes
        .get(start + len)
        .is_none_or(|byte| !is_ident_continue(*byte));
    before && after
}

fn import_symbol(target: &str) -> Option<String> {
    target
        .rsplit('.')
        .next()
        .filter(|symbol| !symbol.is_empty() && *symbol != "*")
        .map(ToOwned::to_owned)
}

fn is_ident_start(byte: u8) -> bool {
    byte == b'_' || byte.is_ascii_alphabetic()
}

fn is_ident_continue(byte: u8) -> bool {
    is_ident_start(byte) || byte.is_ascii_digit()
}
