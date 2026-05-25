//! Rust clap command-surface extraction.

use std::collections::{BTreeMap, BTreeSet};

use tree_sitter::Node;

use super::{ExtractionState, ModuleScope, get_name, node_text, normalize_qualified_name};

pub(super) fn extract_commands(root: Node, source: &str, state: &mut ExtractionState) {
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
    if node.kind() == "call_expression"
        && let Some(handler) = call_handler(node, source, module, variant, state)
    {
        handlers.insert(handler);
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
        .rfind(|name| !matches!(name.as_str(), "Box" | "Option" | "Vec"))
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
