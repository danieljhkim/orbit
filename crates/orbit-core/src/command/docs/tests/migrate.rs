//! Migration (frontmatter upgrade + diff/patch) tests migrated for ORB-00250.

use std::fs;
use std::path::Path;

use tempfile::tempdir;

use super::super::frontmatter::split_frontmatter;
use super::super::migrate::{migrate_doc_content, migration_diff};
use super::super::types::DocType;
use super::*;

#[test]
fn migrate_adds_locked_fields_to_legacy_frontmatter() {
    let raw = "---\ntitle: Example\nowner: codex\n---\n\n# Example\n";
    let updated = migrate_doc_content(
        Path::new("docs/design/sample/1_overview.md"),
        Path::new("docs/design/sample/1_overview.md"),
        raw,
    )
    .expect("migrate")
    .expect("changed");
    let parsed =
        parse_doc_frontmatter_strict(Path::new("doc.md"), &updated).expect("valid locked schema");
    assert_eq!(parsed.doc_type, DocType::Design);
    assert_eq!(parsed.tags, vec!["sample"]);
    assert!(
        migrate_doc_content(
            Path::new("docs/design/sample/1_overview.md"),
            Path::new("doc.md"),
            &updated
        )
        .unwrap()
        .is_none()
    );
}

#[test]
fn migration_diff_applies_to_original_content() {
    let dir = tempdir().expect("tempdir");
    let root = dir.path();
    let relative = Path::new("docs/design/sample/1_overview.md");
    let path = root.join(relative);
    fs::create_dir_all(path.parent().expect("parent")).expect("create docs");
    let raw = "---\ntitle: Example\nowner: codex\n---\n\n# Example\n";
    fs::write(&path, raw).expect("write original");

    let updated = migrate_doc_content(relative, &path, raw)
        .expect("migrate")
        .expect("changed");
    let diff = migration_diff("docs/design/sample/1_overview.md", raw, &updated);

    apply_patch(root, &diff, true);
    apply_patch(root, &diff, false);
    assert_eq!(fs::read_to_string(&path).expect("read patched"), updated);
}

#[test]
fn migrate_preserves_multiline_frontmatter_values() {
    let raw = "---\ntitle: Example\ndescription: |\n  First line\n  Second: line\nowner: codex\n---\n\n# Example\n";
    let updated = migrate_doc_content(
        Path::new("docs/design/sample/1_overview.md"),
        Path::new("docs/design/sample/1_overview.md"),
        raw,
    )
    .expect("migrate")
    .expect("changed");
    let block = split_frontmatter(&updated)
        .expect("split")
        .expect("frontmatter");
    let yaml = serde_yaml::from_str::<serde_yaml::Value>(block.raw).expect("yaml");
    let mapping = yaml.as_mapping().expect("mapping");

    assert_eq!(
        yaml_string(mapping, "description"),
        Some("First line\nSecond: line\n")
    );
    assert_eq!(yaml_string(mapping, "type"), Some("design"));
    assert_eq!(yaml_string(mapping, "summary"), Some("Example"));
    parse_doc_frontmatter_strict(Path::new("doc.md"), &updated).expect("valid locked schema");
}

#[test]
fn migrate_preserves_quoted_colon_value() {
    let raw = "---\ntitle: \"Foo: bar\"\nowner: codex\n---\n\n# Example\n";
    let updated = migrate_doc_content(
        Path::new("docs/design/sample/1_overview.md"),
        Path::new("docs/design/sample/1_overview.md"),
        raw,
    )
    .expect("migrate")
    .expect("changed");
    let block = split_frontmatter(&updated)
        .expect("split")
        .expect("frontmatter");
    let yaml = serde_yaml::from_str::<serde_yaml::Value>(block.raw).expect("yaml");
    let mapping = yaml.as_mapping().expect("mapping");

    assert_eq!(yaml_string(mapping, "title"), Some("Foo: bar"));
    assert_eq!(yaml_string(mapping, "type"), Some("design"));
    assert_eq!(yaml_string(mapping, "summary"), Some("Example"));
}
