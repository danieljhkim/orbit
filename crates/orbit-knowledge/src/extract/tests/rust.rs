#![allow(missing_docs)]

// Tests for extract/rust.rs live here as sibling under extract/tests/ per
// docs/design-patterns/test_layout.md. Explicit named imports, no blanket.

use super::super::rust::RustExtractor;
use super::super::{ExtractedExport, FileExtractor};

fn extract_exports(source: &str) -> Vec<ExtractedExport> {
    RustExtractor.extract(source).exports
}

fn export_names(source: &str) -> Vec<String> {
    extract_exports(source)
        .into_iter()
        .map(|export| export.name)
        .collect()
}

#[test]
fn extracts_pub_use_simple_alias_and_grouped_exports() {
    let exports = extract_exports(
        r#"
pub use foo::Bar;
pub use foo::Bar as Baz;
pub use foo::{Qux, Quux};
use private::Hidden;
pub(crate) use crate_only::CrateOnly;
"#,
    );
    let names: Vec<&str> = exports.iter().map(|export| export.name.as_str()).collect();

    assert!(names.contains(&"Bar"));
    assert!(names.contains(&"Baz"));
    assert!(names.contains(&"Qux"));
    assert!(names.contains(&"Quux"));
    assert!(!names.contains(&"Hidden"));
    assert!(!names.contains(&"CrateOnly"));
    assert_eq!(
        exports
            .iter()
            .find(|export| export.name == "Baz")
            .and_then(|export| export.source_path.as_deref()),
        Some("foo::Bar")
    );
}

#[test]
fn extracts_nested_grouped_and_glob_reexports() {
    let names = export_names(
        r#"
pub use foo::{bar::Baz, qux::{A, B}, nested::*};
"#,
    );

    assert!(names.contains(&"Baz".to_string()));
    assert!(names.contains(&"A".to_string()));
    assert!(names.contains(&"B".to_string()));
    assert!(names.contains(&"foo::nested::*".to_string()));
}

#[test]
fn combines_defined_public_items_and_reexports() {
    let names = export_names(
        r#"
pub fn defined_here() {}
fn private_helper() {}
pub use foo::Imported;
"#,
    );

    assert!(names.contains(&"defined_here".to_string()));
    assert!(names.contains(&"Imported".to_string()));
    assert!(!names.contains(&"private_helper".to_string()));
}
