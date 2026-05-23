//! Frontmatter parsing tests (strict + tolerant) migrated from the original
//! monolithic docs.rs test block for ORB-00250.

use std::path::Path;

use super::super::frontmatter::parse_doc_tolerant;
use super::super::types::DocType;
use super::*;

#[test]
fn strict_frontmatter_accepts_locked_schema() {
    let parsed = parse_frontmatter(
        "---\ntype: design\nsummary: Hook rewrite design\ntags: [hook, audit]\nrelated_artifacts: [ORB-00160, ADR-0168, L-0003, F2026-05-001]\n---\n# Body\n",
    )
    .expect("valid frontmatter");
    assert_eq!(parsed.doc_type, DocType::Design);
    assert_eq!(parsed.summary, "Hook rewrite design");
    assert_eq!(parsed.tags, vec!["hook", "audit"]);
    assert_eq!(parsed.related_artifacts.len(), 4);
}

#[test]
fn strict_frontmatter_rejects_missing_required_fields() {
    let missing_type =
        parse_frontmatter("---\nsummary: A doc\n---\nbody\n").expect_err("missing type");
    assert!(
        missing_type
            .to_string()
            .contains("missing required field `type`")
    );
    let missing_summary =
        parse_frontmatter("---\ntype: design\n---\nbody\n").expect_err("missing summary");
    assert!(
        missing_summary
            .to_string()
            .contains("missing required field `summary`")
    );
}

#[test]
fn strict_frontmatter_rejects_unknown_artifact_prefix() {
    let error = parse_frontmatter(
        "---\ntype: design\nsummary: A doc\nrelated_artifacts: [XYZ-1]\n---\nbody\n",
    )
    .expect_err("unknown artifact prefix");
    assert!(error.to_string().contains("unknown related_artifacts"));
}

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
fn malformed_yaml_errors_in_strict_and_falls_back_in_tolerant() {
    let raw = "---\ntype: [\nsummary: bad\n---\n# Fallback\n";
    assert!(parse_frontmatter(raw).is_err());
    let parsed = parse_doc_tolerant(Path::new("docs/context/bad.md"), Path::new("bad.md"), raw);
    assert_eq!(parsed.frontmatter.doc_type, DocType::Context);
}
