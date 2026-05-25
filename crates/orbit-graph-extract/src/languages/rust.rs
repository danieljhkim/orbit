//! Rust tree-sitter extraction.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use tree_sitter::{Node, Parser};

use super::common::{dedup_imports, dedup_refs, dedup_relations, dedup_symbols, normalize_path};
use crate::{ExtractedFile, Extractor, RawCommand, RawImport, RawRef, RawRelation, RawSymbol};

/// Extracts Rust source files into raw graph rows.
pub struct RustExtractor;

impl Extractor for RustExtractor {
    fn lang(&self) -> &'static str {
        "rust"
    }

    fn supports(&self, path: &Path) -> bool {
        path.extension().and_then(|ext| ext.to_str()) == Some("rs")
    }

    fn extract(&self, path: &Path, bytes: &[u8]) -> ExtractedFile {
        let Ok(source) = std::str::from_utf8(bytes) else {
            return ExtractedFile::default();
        };

        let mut parser = Parser::new();
        if parser
            .set_language(&tree_sitter_rust::LANGUAGE.into())
            .is_err()
        {
            return ExtractedFile::default();
        }

        let Some(tree) = parser.parse(source, None) else {
            return ExtractedFile::default();
        };

        let mut state = ExtractionState::new(path);
        let module = ModuleScope::root();
        let root = tree.root_node();
        extract_items(root, source, &module, None, None, &mut state);
        extract_commands(root, source, &mut state);
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
        if target_name.is_empty() || is_ignored_type_name(&target_name) {
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

    fn push_command(&mut self, name: String, node: Node, handler_symbol: Option<String>) {
        if name.is_empty() {
            return;
        }

        self.commands.push(RawCommand {
            name,
            file_path: self.file_path.clone(),
            span_start: node.start_byte(),
            handler_symbol,
        });
    }
}

#[derive(Debug, Clone, Default)]
struct ModuleScope {
    segments: Vec<String>,
}

impl ModuleScope {
    fn root() -> Self {
        Self::default()
    }

    fn child(&self, name: &str) -> Self {
        let mut segments = self.segments.clone();
        segments.push(name.to_string());
        Self { segments }
    }

    fn qualify(&self, name: &str) -> String {
        if self.segments.is_empty() || is_already_qualified(name) {
            name.to_string()
        } else {
            format!("{}::{name}", self.segments.join("::"))
        }
    }
}

fn extract_items(
    node: Node,
    source: &str,
    module: &ModuleScope,
    parent_symbol: Option<&str>,
    method_parent: Option<&str>,
    state: &mut ExtractionState,
) {
    let mut cursor = node.walk();
    let mut pending_attrs = Vec::new();

    for child in node.named_children(&mut cursor) {
        match child.kind() {
            "attribute_item" | "inner_attribute_item" => {
                pending_attrs.push(node_text(child, source));
            }
            "function_item" => {
                extract_function(
                    child,
                    source,
                    module,
                    parent_symbol,
                    method_parent,
                    &pending_attrs,
                    state,
                );
                pending_attrs.clear();
            }
            "function_signature_item" => {
                extract_function_signature(
                    child,
                    source,
                    module,
                    parent_symbol,
                    method_parent,
                    state,
                );
                pending_attrs.clear();
            }
            "struct_item" => {
                extract_named_item(child, source, module, parent_symbol, "struct", state);
                collect_declaration_refs(child, source, module, state);
                pending_attrs.clear();
            }
            "enum_item" => {
                extract_named_item(child, source, module, parent_symbol, "enum", state);
                collect_declaration_refs(child, source, module, state);
                pending_attrs.clear();
            }
            "trait_item" => {
                extract_trait(child, source, module, parent_symbol, state);
                pending_attrs.clear();
            }
            "impl_item" => {
                extract_impl(child, source, module, state);
                pending_attrs.clear();
            }
            "mod_item" => {
                extract_mod(child, source, module, parent_symbol, state);
                pending_attrs.clear();
            }
            "type_item" => {
                extract_named_item(child, source, module, parent_symbol, "type_alias", state);
                collect_declaration_refs(child, source, module, state);
                pending_attrs.clear();
            }
            "const_item" | "static_item" => {
                extract_named_item(child, source, module, parent_symbol, "const", state);
                collect_declaration_refs(child, source, module, state);
                pending_attrs.clear();
            }
            "use_declaration" => {
                extract_use(child, source, state);
                pending_attrs.clear();
            }
            _ => {
                collect_expression_refs(child, source, module, state);
                pending_attrs.clear();
            }
        }
    }
}

fn extract_function(
    node: Node,
    source: &str,
    module: &ModuleScope,
    parent_symbol: Option<&str>,
    method_parent: Option<&str>,
    attrs: &[String],
    state: &mut ExtractionState,
) {
    let Some(name) = get_name(node, source) else {
        return;
    };

    let kind = if has_test_attr(attrs) {
        "test"
    } else if method_parent.is_some() {
        "method"
    } else {
        "function"
    };
    let qualified = qualify_member_or_module(module, method_parent, &name);
    state.push_symbol(
        node,
        source,
        name,
        qualified,
        kind,
        method_parent.or(parent_symbol).map(ToOwned::to_owned),
    );
    collect_signature_refs(node, source, module, state);
    if let Some(body) = node.child_by_field_name("body") {
        collect_expression_refs(body, source, module, state);
    }
}

fn extract_function_signature(
    node: Node,
    source: &str,
    module: &ModuleScope,
    parent_symbol: Option<&str>,
    method_parent: Option<&str>,
    state: &mut ExtractionState,
) {
    let Some(name) = get_name(node, source) else {
        return;
    };
    let qualified = qualify_member_or_module(module, method_parent, &name);
    state.push_symbol(
        node,
        source,
        name,
        qualified,
        "method",
        method_parent.or(parent_symbol).map(ToOwned::to_owned),
    );
    collect_signature_refs(node, source, module, state);
}

fn extract_named_item(
    node: Node,
    source: &str,
    module: &ModuleScope,
    parent_symbol: Option<&str>,
    kind: &'static str,
    state: &mut ExtractionState,
) {
    let Some(name) = get_name(node, source) else {
        return;
    };
    let qualified = module.qualify(&name);
    state.push_symbol(
        node,
        source,
        name,
        qualified,
        kind,
        parent_symbol.map(ToOwned::to_owned),
    );
}

fn extract_trait(
    node: Node,
    source: &str,
    module: &ModuleScope,
    parent_symbol: Option<&str>,
    state: &mut ExtractionState,
) {
    let Some(name) = get_name(node, source) else {
        return;
    };
    let qualified = module.qualify(&name);
    state.push_symbol(
        node,
        source,
        name,
        qualified.clone(),
        "trait",
        parent_symbol.map(ToOwned::to_owned),
    );
    collect_trait_bound_fields(node, source, module, state);
    collect_signature_refs(node, source, module, state);
    if let Some(body) = node.child_by_field_name("body") {
        extract_items(
            body,
            source,
            module,
            Some(&qualified),
            Some(&qualified),
            state,
        );
    }
}

fn extract_impl(node: Node, source: &str, module: &ModuleScope, state: &mut ExtractionState) {
    let Some(type_node) = node.child_by_field_name("type") else {
        return;
    };
    let Some(type_name) = type_qualified_name(type_node, source, module) else {
        return;
    };
    let trait_name = node
        .child_by_field_name("trait")
        .and_then(|trait_node| type_qualified_name(trait_node, source, module));

    let impl_qualified = match trait_name.as_deref() {
        Some(trait_name) => format!("<{type_name} as {trait_name}>"),
        None => format!("<{type_name}>"),
    };
    state.push_symbol(
        node,
        source,
        type_name.clone(),
        impl_qualified.clone(),
        "impl",
        None,
    );

    if let Some(trait_name) = trait_name {
        state.relations.push(RawRelation {
            from_qualified: type_name,
            to_qualified: trait_name,
            kind: "impl".to_string(),
            def_file: state.file_path.clone(),
            def_span_start: node.start_byte(),
            def_span_end: node.end_byte(),
            confidence: "exact".to_string(),
        });
    }

    collect_trait_bound_fields(node, source, module, state);
    collect_type_refs(type_node, source, module, "type", state);
    if let Some(trait_node) = node.child_by_field_name("trait") {
        collect_type_refs(trait_node, source, module, "type", state);
    }
    if let Some(body) = node.child_by_field_name("body") {
        extract_items(
            body,
            source,
            module,
            Some(&impl_qualified),
            Some(&impl_qualified),
            state,
        );
    }
}

fn extract_mod(
    node: Node,
    source: &str,
    module: &ModuleScope,
    parent_symbol: Option<&str>,
    state: &mut ExtractionState,
) {
    let Some(name) = get_name(node, source) else {
        return;
    };
    let qualified = module.qualify(&name);
    state.push_symbol(
        node,
        source,
        name.clone(),
        qualified.clone(),
        "module",
        parent_symbol.map(ToOwned::to_owned),
    );
    if let Some(body) = node.child_by_field_name("body") {
        let child_module = module.child(&name);
        extract_items(body, source, &child_module, Some(&qualified), None, state);
    }
}

fn collect_signature_refs(
    node: Node,
    source: &str,
    module: &ModuleScope,
    state: &mut ExtractionState,
) {
    if let Some(params) = node.child_by_field_name("parameters") {
        collect_type_refs(params, source, module, "type", state);
    }
    if let Some(return_type) = node.child_by_field_name("return_type") {
        collect_type_refs(return_type, source, module, "type", state);
    }
    if let Some(type_parameters) = node.child_by_field_name("type_parameters") {
        collect_trait_bound_fields(type_parameters, source, module, state);
    }
    collect_trait_bound_fields(node, source, module, state);
}

fn collect_declaration_refs(
    node: Node,
    source: &str,
    module: &ModuleScope,
    state: &mut ExtractionState,
) {
    if let Some(type_parameters) = node.child_by_field_name("type_parameters") {
        collect_trait_bound_fields(type_parameters, source, module, state);
    }
    if let Some(type_node) = node.child_by_field_name("type") {
        collect_type_refs(type_node, source, module, "type", state);
    }
    if let Some(body) = node.child_by_field_name("body") {
        collect_type_refs(body, source, module, "type", state);
    }
    if let Some(value) = node.child_by_field_name("value") {
        collect_expression_refs(value, source, module, state);
    }

    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        if child.kind() == "where_clause" {
            collect_trait_bound_fields(child, source, module, state);
        }
    }
}

fn collect_trait_bound_fields(
    node: Node,
    source: &str,
    module: &ModuleScope,
    state: &mut ExtractionState,
) {
    if node.kind() == "trait_bounds" {
        collect_trait_bound_refs(node, source, module, state);
    }

    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        if child.kind() == "trait_bounds" {
            collect_trait_bound_refs(child, source, module, state);
        } else {
            collect_trait_bound_fields(child, source, module, state);
        }
    }
}

fn collect_trait_bound_refs(
    node: Node,
    source: &str,
    module: &ModuleScope,
    state: &mut ExtractionState,
) {
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        match child.kind() {
            "lifetime" => {}
            "removed_trait_bound" | "higher_ranked_trait_bound" => {
                collect_trait_bound_refs(child, source, module, state);
            }
            _ if is_type_reference_node(child) => {
                state.push_ref(
                    child,
                    source,
                    type_qualified_name(child, source, module),
                    "trait_bound",
                    confidence_for_type(child, source),
                );
                collect_type_refs(child, source, module, "type", state);
            }
            _ => collect_trait_bound_refs(child, source, module, state),
        }
    }
}

fn collect_type_refs(
    node: Node,
    source: &str,
    module: &ModuleScope,
    kind: &'static str,
    state: &mut ExtractionState,
) {
    match node.kind() {
        "primitive_type" | "lifetime" | "identifier" | "field_identifier" | "self" | "crate"
        | "super" => return,
        "type_identifier" | "scoped_type_identifier" => {
            state.push_ref(
                node,
                source,
                type_qualified_name(node, source, module),
                kind,
                confidence_for_type(node, source),
            );
            return;
        }
        "generic_type" | "generic_type_with_turbofish" => {
            if let Some(type_node) = node.child_by_field_name("type") {
                collect_type_refs(type_node, source, module, kind, state);
            }
            if let Some(arguments) = node.child_by_field_name("type_arguments") {
                collect_type_refs(arguments, source, module, kind, state);
            }
            return;
        }
        "trait_bounds" => {
            collect_trait_bound_refs(node, source, module, state);
            return;
        }
        _ => {}
    }

    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        collect_type_refs(child, source, module, kind, state);
    }
}

fn collect_expression_refs(
    node: Node,
    source: &str,
    module: &ModuleScope,
    state: &mut ExtractionState,
) {
    if node.kind() == "call_expression" {
        collect_call_ref(node, source, module, state);
        if let Some(arguments) = node.child_by_field_name("arguments") {
            collect_expression_refs(arguments, source, module, state);
        }
        return;
    }
    if is_type_reference_node(node) {
        collect_type_refs(node, source, module, "type", state);
        return;
    }

    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        match child.kind() {
            "function_item"
            | "function_signature_item"
            | "struct_item"
            | "enum_item"
            | "trait_item"
            | "impl_item"
            | "mod_item"
            | "type_item"
            | "const_item"
            | "static_item"
            | "use_declaration" => {}
            _ => collect_expression_refs(child, source, module, state),
        }
    }
}

fn collect_call_ref(node: Node, source: &str, module: &ModuleScope, state: &mut ExtractionState) {
    let Some(function) = node.child_by_field_name("function") else {
        return;
    };

    match function.kind() {
        "identifier" => state.push_ref(
            function,
            source,
            Some(module.qualify(&node_text(function, source))),
            "call",
            "fuzzy_name",
        ),
        "scoped_identifier" => state.push_ref(
            function,
            source,
            Some(normalize_qualified_name(&node_text(function, source))),
            "call",
            "import_resolved",
        ),
        "field_expression" => {
            if let Some(field) = function.child_by_field_name("field") {
                state.push_ref(field, source, None, "call", "fuzzy_name");
            }
        }
        "generic_function" | "generic_type_with_turbofish" => {
            if let Some(type_node) = function.child_by_field_name("function") {
                state.push_ref(
                    type_node,
                    source,
                    type_qualified_name(type_node, source, module),
                    "call",
                    confidence_for_type(type_node, source),
                );
            } else {
                collect_expression_refs(function, source, module, state);
            }
        }
        _ => collect_expression_refs(function, source, module, state),
    }
}

fn extract_use(node: Node, source: &str, state: &mut ExtractionState) {
    let Some(argument) = node.child_by_field_name("argument") else {
        return;
    };
    collect_use(argument, source, &[], state);
}

fn collect_use(node: Node, source: &str, prefix: &[String], state: &mut ExtractionState) {
    match node.kind() {
        "scoped_use_list" => {
            let mut next_prefix = prefix.to_vec();
            if let Some(path) = node.child_by_field_name("path") {
                next_prefix.extend(path_segments(path, source));
            }
            if let Some(list) = node.child_by_field_name("list") {
                collect_use(list, source, &next_prefix, state);
            }
        }
        "use_list" => {
            let mut cursor = node.walk();
            for child in node.named_children(&mut cursor) {
                collect_use(child, source, prefix, state);
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
            push_use_rows(
                alias,
                source,
                &source_segments,
                Some(node_text(alias, source)),
                state,
            );
        }
        "use_wildcard" => {
            let mut source_segments = prefix.to_vec();
            let mut cursor = node.walk();
            for child in node.named_children(&mut cursor) {
                source_segments.extend(path_segments(child, source));
            }
            if let Some(target_path) = join_segments(&source_segments) {
                state.imports.push(RawImport {
                    from_file: state.file_path.clone(),
                    target_path,
                    target_symbol: None,
                });
                state.push_ref(node, source, None, "use", "import_resolved");
            }
        }
        "identifier" | "crate" | "self" | "super" | "scoped_identifier" => {
            let mut source_segments = prefix.to_vec();
            source_segments.extend(path_segments(node, source));
            push_use_rows(node, source, &source_segments, None, state);
        }
        _ => {
            let mut cursor = node.walk();
            for child in node.named_children(&mut cursor) {
                collect_use(child, source, prefix, state);
            }
        }
    }
}

fn push_use_rows(
    span_node: Node,
    source: &str,
    source_segments: &[String],
    alias: Option<String>,
    state: &mut ExtractionState,
) {
    let Some(source_path) = join_segments(source_segments) else {
        return;
    };
    let imported_name = alias.unwrap_or_else(|| import_name(source_segments));
    let target_path = import_target_path(source_segments);

    state.imports.push(RawImport {
        from_file: state.file_path.clone(),
        target_path,
        target_symbol: Some(imported_name),
    });
    state.push_ref(
        span_node,
        source,
        Some(source_path),
        "use",
        "import_resolved",
    );
}

fn extract_commands(root: Node, source: &str, state: &mut ExtractionState) {
    let mut extraction = CommandExtraction::default();
    collect_command_items(root, source, &ModuleScope::root(), &mut extraction);

    for enum_index in 0..extraction.enums.len() {
        let prefix = command_prefix_for_enum(&extraction, enum_index, &state.file_path);
        let mut visited = BTreeSet::new();
        emit_commands_for_enum(
            root,
            source,
            &extraction,
            enum_index,
            &prefix,
            state,
            &mut visited,
        );
    }
}

#[derive(Default)]
struct CommandExtraction<'tree> {
    enums: Vec<CommandEnum<'tree>>,
    enum_prefixes: BTreeMap<String, Vec<String>>,
}

struct CommandEnum<'tree> {
    name: String,
    qualified: String,
    module: ModuleScope,
    variants: Vec<CommandVariant<'tree>>,
}

struct CommandVariant<'tree> {
    name: String,
    command_name: String,
    node: Node<'tree>,
    payload_type: Option<String>,
}

fn collect_command_items<'tree>(
    node: Node<'tree>,
    source: &str,
    module: &ModuleScope,
    extraction: &mut CommandExtraction<'tree>,
) {
    let mut cursor = node.walk();
    let mut pending_attrs = Vec::new();

    for child in node.named_children(&mut cursor) {
        match child.kind() {
            "attribute_item" | "inner_attribute_item" => {
                pending_attrs.push(node_text(child, source));
            }
            "struct_item" => {
                register_subcommand_field_prefixes(
                    child,
                    source,
                    module,
                    &pending_attrs,
                    extraction,
                );
                pending_attrs.clear();
            }
            "enum_item" => {
                if has_clap_command_derive(&pending_attrs)
                    && let Some(command_enum) =
                        command_enum_from_node(child, source, module, &pending_attrs)
                {
                    extraction.enums.push(command_enum);
                }
                pending_attrs.clear();
            }
            "mod_item" => {
                if let Some(name) = get_name(child, source)
                    && let Some(body) = child.child_by_field_name("body")
                {
                    let child_module = module.child(&name);
                    collect_command_items(body, source, &child_module, extraction);
                }
                pending_attrs.clear();
            }
            _ => {
                pending_attrs.clear();
            }
        }
    }
}

fn register_subcommand_field_prefixes(
    node: Node,
    source: &str,
    module: &ModuleScope,
    attrs: &[String],
    extraction: &mut CommandExtraction<'_>,
) {
    let Some(struct_name) = get_name(node, source) else {
        return;
    };
    let prefix = struct_command_prefix(&struct_name, attrs);
    for field_type in subcommand_field_types(node, source) {
        let qualified = module.qualify(&field_type);
        extraction
            .enum_prefixes
            .entry(field_type.clone())
            .or_insert_with(|| prefix.clone());
        extraction
            .enum_prefixes
            .entry(qualified)
            .or_insert_with(|| prefix.clone());
    }
}

fn subcommand_field_types(node: Node, source: &str) -> Vec<String> {
    let mut field_types = Vec::new();
    collect_subcommand_field_types(node, source, &mut field_types);
    field_types.sort();
    field_types.dedup();
    field_types
}

fn collect_subcommand_field_types(node: Node, source: &str, field_types: &mut Vec<String>) {
    if node.kind() == "field_declaration" && node_text(node, source).contains("subcommand") {
        if let Some(type_node) = node.child_by_field_name("type")
            && let Some(field_type) = command_payload_type(type_node, source)
        {
            field_types.push(field_type);
        }
        return;
    }

    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        collect_subcommand_field_types(child, source, field_types);
    }
}

fn command_enum_from_node<'tree>(
    node: Node<'tree>,
    source: &str,
    module: &ModuleScope,
    _attrs: &[String],
) -> Option<CommandEnum<'tree>> {
    let name = get_name(node, source)?;
    let qualified = module.qualify(&name);
    let variants = command_variants(node, source);
    Some(CommandEnum {
        name,
        qualified,
        module: module.clone(),
        variants,
    })
}

fn command_variants<'tree>(node: Node<'tree>, source: &str) -> Vec<CommandVariant<'tree>> {
    let mut variants = Vec::new();
    let Some(body) = node.child_by_field_name("body") else {
        return variants;
    };

    let mut cursor = body.walk();
    let mut pending_attrs = Vec::new();
    for child in body.named_children(&mut cursor) {
        match child.kind() {
            "attribute_item" | "inner_attribute_item" => {
                pending_attrs.push(node_text(child, source));
            }
            "enum_variant" => {
                if let Some(name) = get_name(child, source) {
                    let mut attrs = pending_attrs.clone();
                    attrs.extend(attributes_inside(child, source));
                    let command_name =
                        command_attr_name(&attrs).unwrap_or_else(|| kebab_case(&name));
                    variants.push(CommandVariant {
                        name,
                        command_name,
                        node: child,
                        payload_type: enum_variant_payload_type(child, source),
                    });
                }
                pending_attrs.clear();
            }
            _ => {
                pending_attrs.clear();
            }
        }
    }

    variants
}

fn attributes_inside(node: Node, source: &str) -> Vec<String> {
    let mut attrs = Vec::new();
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        if matches!(child.kind(), "attribute_item" | "inner_attribute_item") {
            attrs.push(node_text(child, source));
        }
    }
    attrs
}

fn command_prefix_for_enum(
    extraction: &CommandExtraction<'_>,
    enum_index: usize,
    file_path: &str,
) -> Vec<String> {
    let command_enum = &extraction.enums[enum_index];
    extraction
        .enum_prefixes
        .get(&command_enum.qualified)
        .or_else(|| extraction.enum_prefixes.get(&command_enum.name))
        .cloned()
        .unwrap_or_else(|| fallback_enum_prefix(&command_enum.name, file_path))
}

fn emit_commands_for_enum(
    root: Node,
    source: &str,
    extraction: &CommandExtraction<'_>,
    enum_index: usize,
    prefix: &[String],
    state: &mut ExtractionState,
    visited: &mut BTreeSet<String>,
) {
    let command_enum = &extraction.enums[enum_index];
    let visit_key = format!("{}:{}", command_enum.qualified, prefix.join(" "));
    if !visited.insert(visit_key) {
        return;
    }

    for variant in &command_enum.variants {
        let mut path = prefix.to_vec();
        path.push(variant.command_name.clone());
        let command_name = path.join(" ");
        let handler_symbol = handler_for_variant(root, source, command_enum, variant, state);
        state.push_command(command_name, variant.node, handler_symbol);

        if let Some(payload_type) = variant.payload_type.as_deref()
            && let Some(nested_index) =
                find_command_enum(extraction, payload_type, &command_enum.module)
        {
            emit_commands_for_enum(
                root,
                source,
                extraction,
                nested_index,
                &path,
                state,
                visited,
            );
        }
    }
}

fn find_command_enum(
    extraction: &CommandExtraction<'_>,
    type_name: &str,
    module: &ModuleScope,
) -> Option<usize> {
    let qualified = module.qualify(type_name);
    let matches = extraction
        .enums
        .iter()
        .enumerate()
        .filter(|(_, command_enum)| {
            command_enum.qualified == qualified || command_enum.name == type_name
        })
        .map(|(index, _)| index)
        .collect::<Vec<_>>();
    if matches.len() == 1 {
        matches.first().copied()
    } else {
        None
    }
}

fn handler_for_variant(
    root: Node,
    source: &str,
    command_enum: &CommandEnum<'_>,
    variant: &CommandVariant<'_>,
    state: &ExtractionState,
) -> Option<String> {
    let mut handlers = BTreeSet::new();
    collect_variant_handlers(root, source, command_enum, variant, state, &mut handlers);
    if handlers.len() == 1 {
        handlers.first().cloned()
    } else {
        None
    }
}

fn collect_variant_handlers(
    node: Node,
    source: &str,
    command_enum: &CommandEnum<'_>,
    variant: &CommandVariant<'_>,
    state: &ExtractionState,
    handlers: &mut BTreeSet<String>,
) {
    if node.kind() == "match_arm" {
        if let Some(handler) = handler_from_match_arm(node, source, command_enum, variant, state) {
            handlers.insert(handler);
        }
        return;
    }

    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        collect_variant_handlers(child, source, command_enum, variant, state, handlers);
    }
}

fn handler_from_match_arm(
    node: Node,
    source: &str,
    command_enum: &CommandEnum<'_>,
    variant: &CommandVariant<'_>,
    state: &ExtractionState,
) -> Option<String> {
    let text = node_text(node, source);
    let (pattern, _) = text.split_once("=>")?;
    if !pattern_mentions_variant(pattern, &command_enum.name, &variant.name) {
        return None;
    }

    let mut handlers = BTreeSet::new();
    collect_call_handlers(
        node,
        source,
        &command_enum.module,
        variant,
        state,
        &mut handlers,
    );
    if handlers.len() == 1 {
        handlers.first().cloned()
    } else {
        None
    }
}

fn collect_call_handlers(
    node: Node,
    source: &str,
    module: &ModuleScope,
    variant: &CommandVariant<'_>,
    state: &ExtractionState,
    handlers: &mut BTreeSet<String>,
) {
    if node.kind() == "call_expression" {
        if let Some(handler) = call_handler(node, source, module, variant, state) {
            handlers.insert(handler);
        }
    }

    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        collect_call_handlers(child, source, module, variant, state, handlers);
    }
}

fn call_handler(
    node: Node,
    source: &str,
    module: &ModuleScope,
    variant: &CommandVariant<'_>,
    state: &ExtractionState,
) -> Option<String> {
    let function = node.child_by_field_name("function")?;
    match function.kind() {
        "identifier" => Some(module.qualify(&node_text(function, source))),
        "scoped_identifier" => Some(normalize_qualified_name(&node_text(function, source))),
        "field_expression" => {
            let method = function.child_by_field_name("field")?;
            let method_name = node_text(method, source);
            variant.payload_type.as_deref().and_then(|payload_type| {
                handler_for_payload_method(payload_type, &method_name, state)
            })
        }
        "generic_function" | "generic_type_with_turbofish" => function
            .child_by_field_name("function")
            .and_then(|function| call_function_handler(function, source, module)),
        _ => None,
    }
}

fn call_function_handler(function: Node, source: &str, module: &ModuleScope) -> Option<String> {
    match function.kind() {
        "identifier" => Some(module.qualify(&node_text(function, source))),
        "scoped_identifier" => Some(normalize_qualified_name(&node_text(function, source))),
        _ => None,
    }
}

fn handler_for_payload_method(
    payload_type: &str,
    method_name: &str,
    state: &ExtractionState,
) -> Option<String> {
    let candidates = state
        .symbols
        .iter()
        .filter(|symbol| symbol.kind == "method" && symbol.name == method_name)
        .filter(|symbol| {
            symbol.qualified == format!("<{payload_type}>::{method_name}")
                || (symbol.qualified.ends_with(&format!(">::{method_name}"))
                    && symbol
                        .qualified
                        .trim_start_matches('<')
                        .starts_with(payload_type))
                || (symbol.qualified.ends_with(&format!(">::{method_name}"))
                    && symbol.qualified.contains(&format!("::{payload_type} as ")))
        })
        .map(|symbol| symbol.qualified.clone())
        .collect::<BTreeSet<_>>();
    if candidates.len() == 1 {
        candidates.first().cloned()
    } else if method_name == "execute" {
        Some(format!("<{payload_type} as Execute>::execute"))
    } else if method_name == "run" {
        Some(format!("<{payload_type}>::run"))
    } else {
        None
    }
}

fn pattern_mentions_variant(pattern: &str, enum_name: &str, variant_name: &str) -> bool {
    let compact: String = pattern.chars().filter(|ch| !ch.is_whitespace()).collect();
    compact.contains(&format!("::{variant_name}("))
        || compact.contains(&format!("::{variant_name}{{"))
        || compact.ends_with(&format!("::{variant_name}"))
        || compact.contains(&format!("{enum_name}::{variant_name}"))
        || compact.starts_with(&format!("{variant_name}("))
        || compact.starts_with(&format!("{variant_name}{{"))
        || compact == variant_name
}

fn has_clap_command_derive(attrs: &[String]) -> bool {
    attrs
        .iter()
        .any(|attr| attr_has_derive(attr, "Subcommand") || attr_has_derive(attr, "Parser"))
}

fn attr_has_derive(attr: &str, target: &str) -> bool {
    let compact: String = attr.chars().filter(|ch| !ch.is_whitespace()).collect();
    let Some(start) = compact.find("derive(") else {
        return false;
    };
    let body = compact.get(start + "derive(".len()..).unwrap_or_default();
    let end = body.find(')').unwrap_or(body.len());
    body.get(..end)
        .unwrap_or_default()
        .split(',')
        .any(|token| token.rsplit("::").next() == Some(target))
}

fn command_attr_name(attrs: &[String]) -> Option<String> {
    attrs
        .iter()
        .filter(|attr| attr.contains("command"))
        .find_map(|attr| string_keyword_value(attr, "name"))
}

fn string_keyword_value(text: &str, key: &str) -> Option<String> {
    let key_index = text.find(key)?;
    let after_key = text.get(key_index + key.len()..)?;
    let equals_index = after_key.find('=')?;
    let after_equals = after_key.get(equals_index + 1..)?.trim_start();
    let quote = after_equals.chars().next()?;
    if !matches!(quote, '"' | '\'') {
        return None;
    }
    let rest = after_equals.get(quote.len_utf8()..)?;
    let end = rest.find(quote)?;
    Some(rest.get(..end)?.to_string())
}

fn struct_command_prefix(struct_name: &str, attrs: &[String]) -> Vec<String> {
    if attrs.iter().any(|attr| attr_has_derive(attr, "Parser"))
        && let Some(command_name) = command_attr_name(attrs)
    {
        if command_name == "orbit" {
            return Vec::new();
        }
        return vec![command_name];
    }

    type_name_prefix(struct_name)
}

fn fallback_enum_prefix(enum_name: &str, file_path: &str) -> Vec<String> {
    let prefix = type_name_prefix(enum_name);
    if !prefix.is_empty() || enum_name != "Command" {
        return prefix;
    }
    crate_name_from_path(file_path).into_iter().collect()
}

fn type_name_prefix(name: &str) -> Vec<String> {
    let stem = name
        .strip_suffix("Subcommands")
        .or_else(|| name.strip_suffix("Subcommand"))
        .or_else(|| name.strip_suffix("Commands"))
        .or_else(|| name.strip_suffix("Command"))
        .or_else(|| name.strip_suffix("Args"))
        .or_else(|| name.strip_suffix("Cli"))
        .unwrap_or(name);
    if stem.is_empty() || stem == name && matches!(name, "Command" | "Commands" | "Cli") {
        return Vec::new();
    }
    kebab_case(stem)
        .split('-')
        .filter(|segment| !segment.is_empty())
        .map(ToOwned::to_owned)
        .collect()
}

fn crate_name_from_path(path: &str) -> Option<String> {
    let mut parts = path.split('/');
    while let Some(part) = parts.next() {
        if part == "crates" {
            return parts.next().map(ToOwned::to_owned);
        }
    }
    None
}

fn enum_variant_payload_type(node: Node, source: &str) -> Option<String> {
    node.child_by_field_name("body")
        .and_then(|body| command_payload_type(body, source))
        .or_else(|| {
            let name_span = node
                .child_by_field_name("name")
                .map(|name| (name.start_byte(), name.end_byte()));
            let mut types = Vec::new();
            collect_command_type_names(node, source, name_span, &mut types);
            choose_payload_type(types)
        })
}

fn command_payload_type(node: Node, source: &str) -> Option<String> {
    let mut types = Vec::new();
    collect_command_type_names(node, source, None, &mut types);
    choose_payload_type(types)
}

fn collect_command_type_names(
    node: Node,
    source: &str,
    excluded_span: Option<(usize, usize)>,
    types: &mut Vec<String>,
) {
    if excluded_span.is_some_and(|span| span == (node.start_byte(), node.end_byte())) {
        return;
    }

    if matches!(
        node.kind(),
        "type_identifier" | "scoped_type_identifier" | "identifier" | "scoped_identifier"
    ) {
        let text = normalize_qualified_name(&node_text(node, source));
        if let Some(name) = text
            .rsplit("::")
            .next()
            .filter(|name| !name.is_empty())
            .map(ToOwned::to_owned)
        {
            types.push(name);
        }
        return;
    }

    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        collect_command_type_names(child, source, excluded_span, types);
    }
}

fn choose_payload_type(types: Vec<String>) -> Option<String> {
    types
        .into_iter()
        .filter(|name| !matches!(name.as_str(), "Box" | "Option" | "Vec"))
        .next_back()
}

fn kebab_case(name: &str) -> String {
    let mut out = String::new();
    let chars = name.chars().collect::<Vec<_>>();
    for (index, ch) in chars.iter().enumerate() {
        if ch.is_ascii_uppercase() {
            let prev = index.checked_sub(1).and_then(|prev| chars.get(prev));
            let next = chars.get(index + 1);
            if index > 0
                && (prev.is_some_and(|prev| prev.is_ascii_lowercase() || prev.is_ascii_digit())
                    || next.is_some_and(|next| next.is_ascii_lowercase()))
            {
                out.push('-');
            }
            out.push(ch.to_ascii_lowercase());
        } else if *ch == '_' {
            out.push('-');
        } else {
            out.push(*ch);
        }
    }
    out
}

fn get_name(node: Node, source: &str) -> Option<String> {
    node.child_by_field_name("name")
        .map(|name| node_text(name, source))
        .filter(|name| !name.is_empty())
}

fn node_text(node: Node, source: &str) -> String {
    node.utf8_text(source.as_bytes()).unwrap_or("").to_string()
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

fn has_test_attr(attrs: &[String]) -> bool {
    attrs.iter().any(|attr| {
        let compact: String = attr.chars().filter(|ch| !ch.is_whitespace()).collect();
        compact == "#[test]" || compact.ends_with("::test]") || compact.contains("(test")
    })
}

fn qualify_member_or_module(
    module: &ModuleScope,
    parent_symbol: Option<&str>,
    name: &str,
) -> String {
    match parent_symbol {
        Some(parent) => format!("{parent}::{name}"),
        None => module.qualify(name),
    }
}

fn type_qualified_name(node: Node, source: &str, module: &ModuleScope) -> Option<String> {
    let text = type_reference_text(node, source)?;
    if text.is_empty() || is_ignored_type_name(&text) {
        None
    } else if is_already_qualified(&text) {
        Some(normalize_qualified_name(&text))
    } else {
        Some(module.qualify(&text))
    }
}

fn type_reference_text(node: Node, source: &str) -> Option<String> {
    match node.kind() {
        "type_identifier" | "identifier" | "scoped_type_identifier" | "scoped_identifier" => {
            Some(normalize_qualified_name(&node_text(node, source)))
        }
        "generic_type" | "generic_type_with_turbofish" => node
            .child_by_field_name("type")
            .and_then(|type_node| type_reference_text(type_node, source)),
        "reference_type" | "pointer_type" | "array_type" | "tuple_type" | "unit_type" => {
            first_type_reference_child(node, source)
        }
        _ => {
            if is_type_reference_node(node) {
                first_type_reference_child(node, source)
            } else {
                None
            }
        }
    }
}

fn first_type_reference_child(node: Node, source: &str) -> Option<String> {
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        if let Some(text) = type_reference_text(child, source) {
            return Some(text);
        }
    }
    None
}

fn target_name(node: Node, source: &str) -> Option<String> {
    let text = match node.kind() {
        "generic_type" | "generic_type_with_turbofish" => node
            .child_by_field_name("type")
            .map(|type_node| node_text(type_node, source))?,
        _ => node_text(node, source),
    };
    let normalized = normalize_qualified_name(&text);
    normalized
        .rsplit("::")
        .next()
        .map(str::to_string)
        .filter(|name| !name.is_empty() && name != "*")
}

fn normalize_qualified_name(name: &str) -> String {
    name.split("::")
        .map(str::trim)
        .filter(|segment| !segment.is_empty())
        .collect::<Vec<_>>()
        .join("::")
}

fn is_already_qualified(name: &str) -> bool {
    name.contains("::") || name.starts_with('<')
}

fn is_ignored_type_name(name: &str) -> bool {
    matches!(
        name,
        "Self"
            | "self"
            | "str"
            | "bool"
            | "char"
            | "usize"
            | "isize"
            | "u8"
            | "u16"
            | "u32"
            | "u64"
            | "u128"
            | "i8"
            | "i16"
            | "i32"
            | "i64"
            | "i128"
            | "f32"
            | "f64"
            | "()"
    )
}

fn is_type_reference_node(node: Node) -> bool {
    matches!(
        node.kind(),
        "type_identifier"
            | "scoped_type_identifier"
            | "generic_type"
            | "generic_type_with_turbofish"
            | "reference_type"
            | "pointer_type"
            | "array_type"
            | "tuple_type"
            | "unit_type"
    )
}

fn confidence_for_type(node: Node, source: &str) -> &'static str {
    let text = node_text(node, source);
    if text.contains("::") {
        "import_resolved"
    } else {
        "fuzzy_name"
    }
}

fn path_segments(node: Node, source: &str) -> Vec<String> {
    normalize_qualified_name(&node_text(node, source))
        .split("::")
        .filter(|segment| !segment.is_empty())
        .map(ToOwned::to_owned)
        .collect()
}

fn join_segments(segments: &[String]) -> Option<String> {
    if segments.is_empty() {
        None
    } else {
        Some(segments.join("::"))
    }
}

fn import_name(segments: &[String]) -> String {
    match segments.last().map(String::as_str) {
        Some("self") if segments.len() > 1 => segments[segments.len() - 2].clone(),
        Some(name) => name.to_string(),
        None => String::new(),
    }
}

fn import_target_path(segments: &[String]) -> String {
    if segments.len() <= 1 {
        return segments.first().cloned().unwrap_or_default();
    }
    segments[..segments.len() - 1].join("::")
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
