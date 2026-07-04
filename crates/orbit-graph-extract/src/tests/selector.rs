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

/// Guards the ORB-10011 re-export: every grammar variant stays reachable and
/// round-trips (parse -> display -> parse) through this crate's surface.
#[test]
fn reexported_selector_surface_roundtrips_every_variant() {
    let variants = [
        "dir:src/command",
        "file:src/lib.rs",
        "symbol:src/lib.rs#hello:function",
        "module:orbit_core::scheduler",
        "command:task.update",
    ];
    for raw in variants {
        let parsed: Selector = raw.parse().unwrap_or_else(|error| {
            panic!("selector `{raw}` did not parse through the re-export: {error}")
        });
        assert_eq!(parsed.to_string(), raw);
        let reparsed: Selector = parsed.to_string().parse().unwrap_or_else(|error| {
            panic!("selector `{raw}` did not re-parse from its display form: {error}")
        });
        assert_eq!(reparsed, parsed);
    }
}
