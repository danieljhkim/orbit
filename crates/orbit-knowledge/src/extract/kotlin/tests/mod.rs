#![allow(missing_docs)]

use super::super::*;

fn fixture() -> &'static str {
    r#"package com.example.graph

typealias UserId = String

data class User(val id: UserId) {
    val displayName: String = id

    fun greet(): String = "hello $displayName"

    companion object {
        fun from(id: UserId) = User(id)
    }
}

sealed class ResultState

enum class Mode {
    Fast,
    Slow
}

object Registry {
    var current: User? = null
}

interface Greeter {
    fun greet(): String
}

val topLevelName = "orbit"
var topLevelCount = 1

fun topLevelFun(user: User): String = user.greet()

fun String.asUserId(): UserId = this
"#
}

fn leaf<'a>(leaves: &'a [ExtractedLeaf], name: &str, kind: &str) -> &'a ExtractedLeaf {
    leaves
        .iter()
        .find(|leaf| leaf.name == name && leaf.kind == kind)
        .unwrap_or_else(|| panic!("missing {kind} leaf {name} in {leaves:#?}"))
}

#[test]
fn file_kind_is_kotlin() {
    assert_eq!(
        KotlinExtractor.file_kind(),
        FileKind::Code(Language::Kotlin)
    );
}

#[test]
fn extracts_required_kotlin_symbols() {
    let result = KotlinExtractor.extract(fixture());
    let leaves = result.leaves;

    let package = leaf(&leaves, "com.example.graph", "package");
    assert_eq!(package.start_line, 1);
    assert_eq!(package.end_line, 1);

    let alias = leaf(&leaves, "UserId", "type_alias");
    assert_eq!(alias.start_line, 3);
    assert_eq!(alias.end_line, 3);

    let user = leaf(&leaves, "User", "class");
    assert_eq!(user.start_line, 5);
    assert_eq!(user.end_line, 13);
    assert!(
        user.children_qualified_names
            .contains(&"User::greet".to_string())
    );

    let display_name = leaf(&leaves, "displayName", "field");
    assert_eq!(display_name.start_line, 6);
    assert_eq!(display_name.parent_qualified_name.as_deref(), Some("User"));

    let method = leaf(&leaves, "greet", "method");
    assert_eq!(method.qualified_name, "User::greet");
    assert_eq!(method.start_line, 8);
    assert_eq!(method.parent_qualified_name.as_deref(), Some("User"));

    let companion = leaf(&leaves, "Companion", "companion_object");
    assert_eq!(companion.qualified_name, "User::Companion");
    assert_eq!(companion.start_line, 10);
    assert_eq!(companion.end_line, 12);

    let companion_method = leaves
        .iter()
        .find(|leaf| leaf.qualified_name == "User::Companion::from")
        .expect("missing companion method");
    assert_eq!(companion_method.name, "from");
    assert_eq!(companion_method.kind, "method");

    let sealed = leaf(&leaves, "ResultState", "class");
    assert_eq!(sealed.start_line, 15);
    assert!(sealed.source.starts_with("sealed class ResultState"));

    let mode = leaf(&leaves, "Mode", "class");
    assert_eq!(mode.start_line, 17);
    assert_eq!(mode.end_line, 20);
    assert!(mode.source.starts_with("enum class Mode"));

    let registry = leaf(&leaves, "Registry", "object");
    assert_eq!(registry.start_line, 22);
    assert_eq!(registry.end_line, 24);

    let interface = leaf(&leaves, "Greeter", "interface");
    assert_eq!(interface.start_line, 26);
    assert_eq!(interface.end_line, 28);

    let top_val = leaf(&leaves, "topLevelName", "field");
    assert_eq!(top_val.start_line, 30);
    assert_eq!(top_val.parent_qualified_name, None);

    let top_var = leaf(&leaves, "topLevelCount", "field");
    assert_eq!(top_var.start_line, 31);
    assert_eq!(top_var.parent_qualified_name, None);

    let function = leaf(&leaves, "topLevelFun", "function");
    assert_eq!(function.start_line, 33);
    assert_eq!(function.end_line, 33);
    assert!(function.source.starts_with("fun topLevelFun"));

    let extension = leaf(&leaves, "String.asUserId", "function");
    assert_eq!(extension.qualified_name, "String.asUserId");
    assert_eq!(extension.start_line, 35);
    assert!(extension.source.starts_with("fun String.asUserId"));
}
