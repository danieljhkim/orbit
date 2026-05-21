#![allow(missing_docs)]

use super::super::*;

#[test]
fn extracts_nested_atx_headings_with_spans() {
    let src = "# Top\n\
               intro paragraph\n\
               ## Alpha\n\
               body a\n\
               ### Alpha Detail\n\
               deep detail\n\
               ## Beta\n\
               body b\n\
               # Other Top\n\
               last para\n";
    let out = MarkdownExtractor.extract(src);
    let kinds: Vec<&str> = out.leaves.iter().map(|l| l.kind.as_str()).collect();
    assert_eq!(kinds, vec!["section"; 5]);

    let names: Vec<&String> = out.leaves.iter().map(|l| &l.name).collect();
    assert_eq!(
        names,
        vec!["Top", "Alpha", "Alpha Detail", "Beta", "Other Top"]
    );

    let slugs: Vec<&String> = out.leaves.iter().map(|l| &l.qualified_name).collect();
    assert_eq!(
        slugs,
        vec!["top", "alpha", "alpha-detail", "beta", "other-top"]
    );

    let depths: Vec<Option<u8>> = out.leaves.iter().map(|l| l.depth).collect();
    assert_eq!(depths, vec![Some(1), Some(2), Some(3), Some(2), Some(1)]);

    // Top spans through the line before the next same-or-higher heading.
    // # Top at line 1, next same-or-higher (# Other Top) at line 9
    // → start 1, end 8.
    assert_eq!(out.leaves[0].start_line, 1);
    assert_eq!(out.leaves[0].end_line, 8);
    // ## Alpha at line 3, next same-or-higher (## Beta) at line 7 → end 6.
    assert_eq!(out.leaves[1].start_line, 3);
    assert_eq!(out.leaves[1].end_line, 6);
    // ### Alpha Detail at line 5, next same-or-higher (## Beta) at line 7 → end 6.
    assert_eq!(out.leaves[2].start_line, 5);
    assert_eq!(out.leaves[2].end_line, 6);
    // ## Beta at line 7, next same-or-higher (# Other Top) at line 9 → end 8.
    assert_eq!(out.leaves[3].start_line, 7);
    assert_eq!(out.leaves[3].end_line, 8);
    // # Other Top at line 9, no successor → end = total lines = 10.
    assert_eq!(out.leaves[4].start_line, 9);
    assert_eq!(out.leaves[4].end_line, 10);
}

#[test]
fn duplicate_slugs_disambiguate_by_line() {
    let src = "# Intro\nx\n# Intro\ny\n";
    let out = MarkdownExtractor.extract(src);
    assert_eq!(out.leaves.len(), 2);
    assert_eq!(out.leaves[0].qualified_name, "intro");
    assert_eq!(out.leaves[1].qualified_name, "intro-3");
}

#[test]
fn ignores_headings_inside_fenced_code_blocks() {
    let src = "# Real\n\
               ```\n\
               # Fake Heading Inside Fence\n\
               ```\n\
               ## Also Real\n";
    let out = MarkdownExtractor.extract(src);
    let names: Vec<&String> = out.leaves.iter().map(|l| &l.name).collect();
    assert_eq!(names, vec!["Real", "Also Real"]);
}

#[test]
fn rejects_seven_or_more_hashes() {
    let src = "####### Too Deep\n";
    let out = MarkdownExtractor.extract(src);
    assert!(out.leaves.is_empty());
}

#[test]
fn no_headings_returns_empty() {
    let src = "just a paragraph\nno headings here\n";
    let out = MarkdownExtractor.extract(src);
    assert!(out.leaves.is_empty());
}

#[test]
fn strips_trailing_hashes_and_whitespace() {
    let src = "## Trailing Hashes ##\n";
    let out = MarkdownExtractor.extract(src);
    assert_eq!(out.leaves.len(), 1);
    assert_eq!(out.leaves[0].name, "Trailing Hashes");
}
