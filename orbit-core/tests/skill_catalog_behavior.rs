use std::fs;

use orbit_core::skill_catalog::{SkillCatalog, SkillCatalogDoctorStatus};
use tempfile::tempdir;

fn write_skill(root: &std::path::Path, id: &str, skill_md: &str) {
    let dir = root.join(id);
    fs::create_dir_all(&dir).expect("create skill dir");
    fs::write(dir.join("SKILL.md"), skill_md).expect("write skill md");
}

#[test]
fn list_returns_only_valid_skills() {
    let dir = tempdir().expect("tempdir");
    let root = dir.path().join("skills");
    let catalog = SkillCatalog::new(root.clone());
    catalog.ensure_layout().expect("layout");

    write_skill(
        &root,
        "valid",
        r#"# valid

## Purpose
Analyze architecture.

## Behavioral Constraints
- Must be deterministic.

## Output Requirements
- JSON output.
"#,
    );
    write_skill(
        &root,
        "invalid",
        r#"# invalid

## Purpose
Oops.
"#,
    );

    let skills = catalog.list().expect("list");
    assert_eq!(skills.len(), 1);
    assert_eq!(skills[0].id, "valid");
}

#[test]
fn load_rejects_unknown_section() {
    let dir = tempdir().expect("tempdir");
    let root = dir.path().join("skills");
    let catalog = SkillCatalog::new(root.clone());
    catalog.ensure_layout().expect("layout");

    write_skill(
        &root,
        "bad",
        r#"# bad

## Purpose
Analyze architecture.

## Behavioral Constraints
- Must be deterministic.

## Output Requirements
- JSON output.

## Unknown
nope
"#,
    );

    let err = catalog.load("bad").expect_err("must fail");
    assert!(err.to_string().contains("unknown section header"));
}

#[test]
fn load_rejects_mismatched_heading_and_directory() {
    let dir = tempdir().expect("tempdir");
    let root = dir.path().join("skills");
    let catalog = SkillCatalog::new(root.clone());
    catalog.ensure_layout().expect("layout");

    write_skill(
        &root,
        "dir-id",
        r#"# heading-id

## Purpose
Analyze architecture.

## Behavioral Constraints
- Must be deterministic.

## Output Requirements
- JSON output.
"#,
    );

    let err = catalog.load("dir-id").expect_err("must fail");
    assert!(err.to_string().contains("must match directory"));
}

#[test]
fn load_parses_meta_and_output_schema() {
    let dir = tempdir().expect("tempdir");
    let root = dir.path().join("skills");
    let catalog = SkillCatalog::new(root.clone());
    catalog.ensure_layout().expect("layout");

    write_skill(
        &root,
        "with-meta",
        r#"# with-meta

## Purpose
Analyze architecture.

## Behavioral Constraints
- Must be deterministic.

## Output Requirements
- JSON output.
"#,
    );
    fs::write(
        root.join("with-meta").join("meta.json"),
        r#"{
  "name": "Assess Codebase",
  "summary": "Deep architecture review",
  "tags": ["architecture", "audit"],
  "version": "1.2.3",
  "type": "object",
  "required": ["summary"],
  "properties": {
    "summary": { "type": "string" }
  }
}"#,
    )
    .expect("meta");

    let loaded = catalog.load("with-meta").expect("load");
    let meta = loaded.meta.expect("meta");
    assert_eq!(meta.name.as_deref(), Some("Assess Codebase"));
    assert_eq!(
        meta.tags,
        vec!["architecture".to_string(), "audit".to_string()]
    );
    assert!(loaded.output_schema.is_some());
    assert!(!loaded.content_hash.is_empty());
}

#[test]
fn doctor_reports_invalid_skill() {
    let dir = tempdir().expect("tempdir");
    let root = dir.path().join("skills");
    let catalog = SkillCatalog::new(root.clone());
    catalog.ensure_layout().expect("layout");

    write_skill(
        &root,
        "broken",
        r#"# broken

## Purpose
Missing required sections.
"#,
    );

    let report = catalog.doctor().expect("doctor");
    assert_eq!(report.len(), 1);
    assert_eq!(report[0].status, SkillCatalogDoctorStatus::Error);
    assert!(report[0].message.contains("missing required section"));
}

#[test]
fn load_rejects_invalid_meta_schema_document() {
    let dir = tempdir().expect("tempdir");
    let root = dir.path().join("skills");
    let catalog = SkillCatalog::new(root.clone());
    catalog.ensure_layout().expect("layout");

    write_skill(
        &root,
        "bad-meta",
        r#"# bad-meta

## Purpose
Analyze architecture.

## Behavioral Constraints
- Must be deterministic.

## Output Requirements
- JSON output.
"#,
    );
    fs::write(
        root.join("bad-meta").join("meta.json"),
        r#"{
  "name": "Bad Meta",
  "type": 7
}"#,
    )
    .expect("meta");

    let err = catalog.load("bad-meta").expect_err("must fail");
    assert!(err.to_string().contains("valid JSON Schema document"));
}
