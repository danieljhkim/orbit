#![allow(missing_docs)]

use super::*;

#[test]
fn simple_selector_symbol_name_handles_qualified_leaf_ids() {
    let cases = [
        ("User.save", "save"),
        ("Outer.Inner.run#2", "run"),
        ("Client::connect#1#2", "connect"),
        ("<Foo as Runnable>::run", "run"),
        ("load#3", "load"),
        ("plain_function", "plain_function"),
    ];

    for (symbol, expected) in cases {
        assert_eq!(simple_selector_symbol_name(symbol), expected);
    }
}

#[test]
fn ref_kind_classification_matches_snapshot() {
    let snapshot = [
        ("src/main.rs", RefKind::Code),
        ("docs/guide.md", RefKind::Doc),
        ("orbit.toml", RefKind::Config),
        ("config/settings.jsonc", RefKind::Config),
    ];

    for (path, expected) in snapshot {
        assert_eq!(classify_ref_kind(path), expected);
    }
}

#[test]
fn include_all_expands_every_kind() {
    let include = RefInclude::from_names(vec!["all".to_string()]).expect("include");
    assert!(include.includes(RefKind::Code));
    assert!(include.includes(RefKind::Doc));
    assert!(include.includes(RefKind::Config));
}
