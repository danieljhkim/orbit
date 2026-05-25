use crate::Selector;

const SELECTOR_CORPUS: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../docs/design/orbit-graph/specs/selector_corpus.txt"
));

#[test]
fn selector_audit_corpus_keeps_frozen_debug_output() {
    let actual = SELECTOR_CORPUS
        .lines()
        .filter_map(|line| {
            let selector = line.trim();
            if selector.is_empty() || selector.starts_with('#') {
                None
            } else {
                Some(selector)
            }
        })
        .map(|line| match line.parse::<Selector>() {
            Ok(selector) => format!("{selector:?}"),
            Err(error) => panic!("selector corpus entry `{line}` did not parse: {error}"),
        })
        .collect::<Vec<_>>();

    let expected = vec![
        r#"Dir { path: "src/command" }"#,
        r#"File { path: "src/lib.rs" }"#,
        r#"Symbol { path: "src/lib.rs", symbol: "hello", kind: "function" }"#,
        r#"Symbol { path: "src/lib.rs", symbol: "Greeter", kind: "trait" }"#,
        r#"Module { qualified: "orbit_core::scheduler" }"#,
        r#"Command { name: "task.update" }"#,
    ];

    assert_eq!(actual, expected);
}
