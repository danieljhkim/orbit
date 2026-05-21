//! Tests for context_files exposure in task.update.

use super::super::*;

#[test]
fn schema_exposes_context_files() {
    let schema = OrbitTaskUpdateTool.schema();

    let param = schema
        .parameters
        .iter()
        .find(|param| param.name == "context_files")
        .expect("context_files param");

    assert_eq!(param.param_type, "string_list");
    assert!(!param.required);
    assert!(
        param
            .description
            .contains("comma-separated string or array")
    );
    assert!(param.description.contains("file:path"));
}
