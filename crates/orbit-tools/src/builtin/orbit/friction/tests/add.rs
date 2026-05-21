//! Tags parameter description tests for friction.add.
//
// Migrated from nested `friction/add/tests/` (anti-pattern child of source)
// to sibling layout under `friction/tests/` per ORB-00243 and
// docs/design-patterns/test_layout.md.

use orbit_common::friction::{DEFAULT_FRICTION_TAGS, friction_tags_literal};

use super::super::add::*;
use crate::Tool;

#[test]
fn tags_parameter_description_lists_default_taxonomy() {
    let schema = OrbitFrictionAddTool.schema();
    let tags_param = schema
        .parameters
        .iter()
        .find(|param| param.name == "tags")
        .expect("tags parameter");

    assert!(
        tags_param.description.contains(&friction_tags_literal()),
        "{}",
        tags_param.description
    );
    for (tag, _description) in DEFAULT_FRICTION_TAGS {
        assert!(
            tags_param.description.contains(tag),
            "tags description should include {tag}: {}",
            tags_param.description
        );
    }
}
