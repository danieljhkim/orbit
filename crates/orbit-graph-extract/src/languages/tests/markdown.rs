#![allow(missing_docs)]

use std::path::Path;

use crate::Extractor;
use crate::languages::MarkdownExtractor;

fn extract(source: &str) -> crate::ExtractedFile {
    MarkdownExtractor.extract(Path::new("README.md"), source.as_bytes())
}

#[test]
fn extracts_nested_headings_as_symbols_and_notable_strings() {
    let source = r#"# Top

Some intro with [link text](https://example.com/page).

## Nested

```rust
fn example() { println!("hi"); }
```

### Deep

Text with another [ref][1] link.

[1]: https://other.com
"#;
    let file = extract(source);

    let kinds: Vec<&str> = file.symbols.iter().map(|s| s.kind.as_str()).collect();
    assert!(kinds.contains(&"heading"), "missing heading symbols");
    // at least top + nested + deep
    let heading_names: Vec<&str> = file.symbols.iter().map(|s| s.name.as_str()).collect();
    assert!(heading_names.iter().any(|n| n.contains("Top") || n.contains("Nested") || n.contains("Deep")));

    // byte spans valid
    assert!(file.symbols.iter().all(|s| s.span_start < s.span_end));

    // strings: links and code
    assert!(!file.strings.is_empty(), "expected notable strings from links/code fences");
    let has_link = file.strings.iter().any(|s| s.value.contains("example.com"));
    let has_code = file.strings.iter().any(|s| s.value.contains("example()") || s.value.contains("println"));
    assert!(has_link, "missing link string");
    assert!(has_code, "missing code block string");
}

#[test]
fn markdown_extractor_no_refs_relations_configs() {
    let file = extract("# H\n\ntext");
    assert!(file.refs.is_empty());
    assert!(file.relations.is_empty());
    assert!(file.configs.is_empty());
}
