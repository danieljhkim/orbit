//! Config parsing (roots + search weights) tests migrated for ORB-00250.

use super::super::config::parse_task_context_docs_roots_from_config_toml;
use super::super::{
    parse_adr_search_config_from_config_toml, parse_docs_roots_from_config_toml,
    parse_docs_search_config_from_config_toml,
};

#[test]
fn config_roots_default_and_parse_explicit_values() {
    assert_eq!(
        parse_docs_roots_from_config_toml("").unwrap(),
        vec!["docs/"]
    );
    assert_eq!(
        parse_docs_roots_from_config_toml("[docs]\nroots = [\"docs/\", \"apps/*/docs/\"]\n")
            .unwrap(),
        vec!["docs/", "apps/*/docs/"]
    );
}

#[test]
fn docs_search_config_defaults_and_clamps_semantic_weight() {
    assert_eq!(
        parse_docs_search_config_from_config_toml("")
            .unwrap()
            .semantic_weight,
        0.5
    );
    assert_eq!(
        parse_docs_search_config_from_config_toml("[docs.search]\nsemantic_weight = 0.7\n")
            .unwrap()
            .semantic_weight,
        0.7
    );
    assert_eq!(
        parse_docs_search_config_from_config_toml("[docs.search]\nsemantic_weight = -1.0\n")
            .unwrap()
            .semantic_weight,
        0.0
    );
    assert_eq!(
        parse_docs_search_config_from_config_toml("[docs.search]\nsemantic_weight = 2.0\n")
            .unwrap()
            .semantic_weight,
        1.0
    );
}

#[test]
fn adr_search_config_defaults_and_clamps_semantic_weight() {
    assert_eq!(
        parse_adr_search_config_from_config_toml("")
            .unwrap()
            .semantic_weight,
        0.5
    );
    assert_eq!(
        parse_adr_search_config_from_config_toml("[adr.search]\nsemantic_weight = 0.7\n")
            .unwrap()
            .semantic_weight,
        0.7
    );
    assert_eq!(
        parse_adr_search_config_from_config_toml("[adr.search]\nsemantic_weight = -1.0\n")
            .unwrap()
            .semantic_weight,
        0.0
    );
    assert_eq!(
        parse_adr_search_config_from_config_toml("[adr.search]\nsemantic_weight = 2.0\n")
            .unwrap()
            .semantic_weight,
        1.0
    );
}

#[test]
fn task_context_docs_roots_skip_explicit_empty_or_unset_roots() {
    assert_eq!(
        parse_task_context_docs_roots_from_config_toml("[docs]\n").unwrap(),
        Vec::<String>::new()
    );
    assert_eq!(
        parse_task_context_docs_roots_from_config_toml("[docs]\nroots = []\n").unwrap(),
        Vec::<String>::new()
    );
    assert_eq!(
        parse_task_context_docs_roots_from_config_toml("").unwrap(),
        vec!["docs/"]
    );
}
