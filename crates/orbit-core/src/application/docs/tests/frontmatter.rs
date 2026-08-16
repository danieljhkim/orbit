//! Frontmatter parsing tests (strict + tolerant) migrated from the original
//! monolithic docs.rs test block for ORB-00250.

use std::path::Path;

use super::super::frontmatter::parse_doc_tolerant;
use super::super::types::DocType;

#[test]
fn tolerant_frontmatter_infers_legacy_design_doc() {
    let parsed = parse_doc_tolerant(
        Path::new("docs/design/hook-rewrite/4_decisions.md"),
        Path::new("docs/design/hook-rewrite/4_decisions.md"),
        "# Decisions\n\nBody\n",
    );
    assert_eq!(parsed.frontmatter.doc_type, DocType::Design);
    assert_eq!(parsed.frontmatter.tags, vec!["hook-rewrite"]);
    assert_eq!(parsed.frontmatter.summary, "Decisions");
}

#[test]
fn tolerant_frontmatter_infers_design_pattern_doc() {
    let parsed = parse_doc_tolerant(
        Path::new("docs/design-patterns/error_translation.md"),
        Path::new("docs/design-patterns/error_translation.md"),
        "# Crate-Boundary Error Translation\n",
    );
    assert_eq!(parsed.frontmatter.doc_type, DocType::Pattern);
    assert_eq!(
        parsed.frontmatter.summary,
        "Crate-Boundary Error Translation"
    );
}

#[test]
fn malformed_yaml_falls_back_in_tolerant() {
    let raw = "---\ntype: [\nsummary: bad\n---\n# Fallback\n";
    let parsed = parse_doc_tolerant(Path::new("docs/context/bad.md"), Path::new("bad.md"), raw);
    assert_eq!(parsed.frontmatter.doc_type, DocType::Context);
}
