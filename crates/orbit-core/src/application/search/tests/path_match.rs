use super::*;

#[test]
fn file_selector_matches_exact_path() {
    let selectors = vec!["file:src/auth/login.rs".to_string()];
    assert!(task_selectors_contain_path(&selectors, "src/auth/login.rs"));
    assert!(!task_selectors_contain_path(
        &selectors,
        "src/auth/logout.rs"
    ));
}

#[test]
fn dir_selector_matches_contained_path() {
    let selectors = vec!["dir:src/auth/".to_string()];
    assert!(task_selectors_contain_path(&selectors, "src/auth/login.rs"));
    assert!(task_selectors_contain_path(
        &selectors,
        "src/auth/handlers/post.rs"
    ));
    assert!(!task_selectors_contain_path(
        &selectors,
        "src/billing/charge.rs"
    ));
}

#[test]
fn symbol_selector_matches_file_component() {
    let selectors = vec!["symbol:src/auth/login.rs#login_handler:function".to_string()];
    assert!(task_selectors_contain_path(&selectors, "src/auth/login.rs"));
    assert!(!task_selectors_contain_path(
        &selectors,
        "src/auth/logout.rs"
    ));
}

#[test]
fn unrelated_dir_selector_does_not_match() {
    let selectors = vec!["dir:crates/orbit-search/".to_string()];
    assert!(!task_selectors_contain_path(
        &selectors,
        "src/auth/login.rs"
    ));
}

#[test]
fn parent_dir_query_matches_descendant_selectors() {
    let selectors = vec![
        "file:src/auth/login.rs".to_string(),
        "dir:src/auth/handlers/".to_string(),
        "symbol:src/auth/logout.rs#logout:function".to_string(),
    ];
    for selector in &selectors {
        assert!(
            task_selectors_contain_path(std::slice::from_ref(selector), "src/auth/"),
            "selector {selector} should match parent dir query"
        );
    }
}

#[test]
fn bare_selector_treated_as_file() {
    let selectors = vec!["src/auth/login.rs".to_string()];
    assert!(task_selectors_contain_path(&selectors, "src/auth/login.rs"));
}
